#!/usr/bin/env bash
# A/B update end to end with real RAUC on loop devices and an emulated
# Raspberry Pi tryboot firmware (crate::bootctl, docs/UPDATE.md).
# Usage (root): ci/update-e2e.sh <t3box-bootctl>
set -euo pipefail
bootctl=$(readlink -f "$1")
# CI uses verity bundles (needs device-mapper). UPDATE_E2E_FORMAT=plain is
# only for local runs in containers without /dev/mapper.
format=${UPDATE_E2E_FORMAT:-verity}
root=$(cd "$(dirname "$0")/.." && pwd)
w=$(mktemp -d)
pids=(); loops=()
svc=""
cleanup() {
  [ -n "$svc" ] && kill "$svc" 2>/dev/null || true
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  awk -v m="$w/mnt" 'index($2, m) == 1 {print $2}' /proc/mounts | sort -r | xargs -r umount -l 2>/dev/null || true
  for l in "${loops[@]}"; do losetup -d "$l" 2>/dev/null || true; done
}
trap cleanup EXIT
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; for f in "$w"/*.log; do echo "--- $f"; tail -40 "$f"; done; exit 1; }

mkdir -p "$w/dt" "$w/data" "$w/selector" "$w/mnt" "$w/rauc-data"
for s in boot-a boot-b rootfs-a rootfs-b; do
  truncate -s 64M "$w/$s.img"
  loops+=("$(losetup --find --show "$w/$s.img")")
done
declare -A dev=([boot-a]=${loops[0]} [boot-b]=${loops[1]} [rootfs-a]=${loops[2]} [rootfs-b]=${loops[3]})

# Emulated firmware: picks [all] or (once) [tryboot] from autoboot.txt and
# exposes the result like /proc/device-tree/chosen/bootloader.
printf '[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=3\n' >"$w/selector/autoboot.txt"
firmware_boot() { # $1 = normal|tryboot
  local sect=all; [ "$1" = tryboot ] && sect=tryboot
  local p; p=$(awk -v s="[$sect]" '/^\[/{cur=$0} cur==s && /^boot_partition=/{split($0,a,"=");print a[2]}' "$w/selector/autoboot.txt")
  python3 -c 'import struct,sys;open(sys.argv[1],"wb").write(struct.pack(">I",int(sys.argv[2])));open(sys.argv[3],"wb").write(struct.pack(">I",int(sys.argv[4])))' \
    "$w/dt/partition" "$p" "$w/dt/tryboot" "$([ "$1" = tryboot ] && echo 1 || echo 0)"
  booted=$([ "$p" = 2 ] && echo A || echo B)
  start_rauc "$booted"
}
export T3BOX_AUTOBOOT=$w/selector/autoboot.txt T3BOX_BOOT_STATE=$w/data/bootctl.json T3BOX_BOOT_SLOTS=A=2,B=3 \
  T3BOX_DEVICE_TREE=$w/dt T3BOX_ENGINE_SOCKET=$w/engine.sock T3BOX_DATA_DIR=$w/data T3BOX_HEALTH_ATTEMPTS=2 T3BOX_REBOOT=true

cat >"$w/pre-install" <<SH
#!/bin/sh
exec "$bootctl" guard
SH
chmod +x "$w/pre-install"
cat >"$w/system.conf" <<CONF
[system]
compatible=t3box-pi5
bootloader=custom
bundle-formats=$format
data-directory=$w/rauc-data
mountprefix=$w/mnt
[keyring]
path=$w/ca.pem
check-purpose=codesign
[handlers]
bootloader-custom-backend=$bootctl
pre-install=$w/pre-install
[slot.rootfs.0]
device=${dev[rootfs-a]}
type=raw
bootname=A
[slot.rootfs.1]
device=${dev[rootfs-b]}
type=raw
bootname=B
[slot.boot.0]
device=${dev[boot-a]}
type=raw
parent=rootfs.0
[slot.boot.1]
device=${dev[boot-b]}
type=raw
parent=rootfs.1
CONF

start_rauc() { # one RAUC service per emulated boot
  [ -n "$svc" ] && { kill "$svc" 2>/dev/null || true; wait "$svc" 2>/dev/null || true; }
  rauc --conf="$w/system.conf" --override-boot-slot="$1" service >>"$w/rauc.log" 2>&1 &
  svc=$!
  for _ in $(seq 50); do rauc status >/dev/null 2>&1 && return; sleep 0.2; done
  fail "rauc service did not start"
}
primary() { "$bootctl" get-primary; }
committed() { awk '/^\[all\]/{a=1} /^\[tryboot\]/{a=0} a && /^boot_partition=/{split($0,x,"=");print x[2]}' "$w/selector/autoboot.txt"; }

# Test-only signing PKI (production keys never exist in CI or the repo).
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj /CN=t3box-test-ca \
  -keyout "$w/ca.key" -out "$w/ca.pem" -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign >/dev/null 2>&1
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj /CN=t3box-test-release -keyout "$w/rel.key" -out "$w/rel.csr" >/dev/null 2>&1
openssl x509 -req -in "$w/rel.csr" -CA "$w/ca.pem" -CAkey "$w/ca.key" -CAcreateserial -days 2 -out "$w/rel.pem" \
  -extfile <(printf 'keyUsage=critical,digitalSignature\nextendedKeyUsage=codeSigning\n') >/dev/null 2>&1
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj /CN=attacker \
  -keyout "$w/evil.key" -out "$w/evil.pem" >/dev/null 2>&1

make_bundle() { # $1 out, $2 version, $3 compatible, $4 key, $5 cert
  local d="$w/content-$2"
  rm -rf "$d"; mkdir -p "$d/root/etc"
  echo "$2" >"$d/root/etc/t3box-version"
  head -c 40M /dev/urandom >"$d/root/payload"
  mksquashfs "$d/root" "$d/rootfs.img" -noappend -quiet >/dev/null
  truncate -s 16M "$d/boot.vfat"; mkfs.vfat -n BOOT "$d/boot.vfat" >/dev/null
  printf '[update]\ncompatible=%s\nversion=%s\n[bundle]\nformat=%s\n[image.rootfs]\nfilename=rootfs.img\n[image.boot]\nfilename=boot.vfat\n' "$3" "$2" "$format" >"$d/manifest.raucm"
  rm -f "$d/root" -r
  rauc bundle --cert="$5" --key="$4" "$d" "$1" >>"$w/bundle.log" 2>&1
  sha256sum "$d/rootfs.img" | cut -d' ' -f1 >"$1.rootfs.sha256"
  stat -c %s "$d/rootfs.img" >"$1.rootfs.size"
}
slot_sha() { head -c "$(cat "$2.rootfs.size")" "${dev[$1]}" | sha256sum | cut -d' ' -f1; }

firmware_boot normal
[ "$(primary)" = A ] || fail "initial primary"
pass "booted A (emulated firmware + RAUC service)"

make_bundle "$w/v2.raucb" 2.0.0 t3box-pi5 "$w/rel.key" "$w/rel.pem"
make_bundle "$w/evil.raucb" 6.6.6 t3box-pi5 "$w/evil.key" "$w/evil.pem"
make_bundle "$w/other.raucb" 2.0.1 other-device "$w/rel.key" "$w/rel.pem"

expect_refused() { # $1 bundle, $2 pattern expected in RAUC's reason
  rauc install "$1" >"$w/last-install.log" 2>&1 && fail "$1 was accepted"
  cat "$w/last-install.log" >>"$w/install.log"
  grep -qiE "$2" "$w/last-install.log" "$w/rauc.log" || fail "$1 refused for the wrong reason (want /$2/): $(tail -3 "$w/last-install.log")"
}
expect_refused "$w/evil.raucb" "signature verification failed"
[ "$(primary)" = A ] || fail "state changed"
pass "bundle from an untrusted signer refused"
grep -q "unsuitable certificate purpose" "$w/last-install.log" && fail "release cert purpose rejected"
expect_refused "$w/other.raucb" "Compatible mismatch"
pass "incompatible bundle refused"

python3 "$root/ci/fake_engine.py" "$w/engine.sock" streaming & pids+=($!)
sleep 0.5
expect_refused "$w/v2.raucb" "refusing to update while streaming"
[ "$(primary)" = A ] || fail "primary changed after refused install"
pass "install refused while streaming (pre-install guard)"
kill "${pids[-1]}"; wait "${pids[-1]}" 2>/dev/null || true
python3 "$root/ci/fake_engine.py" "$w/engine.sock" idle & pids+=($!)
sleep 0.5

# Interruption: kill RAUC in the middle of writing; nothing may change.
( rauc install "$w/v2.raucb" >>"$w/install.log" 2>&1 & )
started=0
for _ in $(seq 300); do
  if [ "$(head -c 4096 "${dev[rootfs-b]}" | tr -d '\0' | wc -c)" -gt 0 ] || [ "$(head -c 4096 "${dev[boot-b]}" | tr -d '\0' | wc -c)" -gt 0 ]; then started=1; break; fi
  sleep 0.02
done
kill -9 "$svc"; svc=""
[ "$started" = 1 ] || fail "install never started writing"
[ "$(committed)" = 2 ] && [ "$(primary)" = A ] || fail "interrupted install changed boot selection"
firmware_boot normal
[ "$("$bootctl" get-current)" = A ] || fail "after interruption"
pass "interrupted install leaves A primary and bootable"

rauc install "$w/v2.raucb" >>"$w/install.log" 2>&1 || fail "install v2"
[ "$(primary)" = B ] && [ "$(committed)" = 2 ] || fail "after install: primary=$(primary) committed=$(committed)"
[ "$(slot_sha rootfs-b "$w/v2.raucb")" = "$(cat "$w/v2.raucb.rootfs.sha256")" ] || fail "slot B content"
pass "v2 written to inactive B and staged as a one-time trial (A still committed)"

firmware_boot tryboot
"$bootctl" confirm >>"$w/confirm.log" 2>&1 || fail "confirm healthy trial"
[ "$(committed)" = 3 ] || fail "not committed"
firmware_boot normal
[ "$("$bootctl" get-current)" = B ] || fail "normal boot after commit"
pass "healthy trial committed; normal boots stay on B"

make_bundle "$w/v3.raucb" 3.0.0 t3box-pi5 "$w/rel.key" "$w/rel.pem"
rauc install "$w/v3.raucb" >>"$w/install.log" 2>&1 || fail "install v3"
[ "$(primary)" = A ] || fail "v3 pending on A"
kill "${pids[-1]}"; wait "${pids[-1]}" 2>/dev/null || true; rm -f "$w/engine.sock"  # engine broken in v3
firmware_boot tryboot
"$bootctl" confirm >>"$w/confirm.log" 2>&1 && fail "unhealthy trial confirmed"
firmware_boot normal
[ "$("$bootctl" get-current)" = B ] && [ "$(committed)" = 3 ] || fail "rollback after failed health"
[ "$("$bootctl" get-state A)" = bad ] || fail "A not marked bad"
pass "unhealthy trial (engine down) rolled back to B and marked bad"

python3 "$root/ci/fake_engine.py" "$w/engine.sock" idle & pids+=($!)
sleep 0.5
rauc install "$w/v3.raucb" >>"$w/install.log" 2>&1 || fail "reinstall v3"
firmware_boot tryboot
firmware_boot normal   # trial crashed / watchdog / power loss before confirmation
"$bootctl" confirm >>"$w/confirm.log" 2>&1 || fail "confirm after crashed trial"
[ "$("$bootctl" get-current)" = B ] && [ "$(primary)" = B ] && [ "$("$bootctl" get-state A)" = bad ] || fail "crashed trial"
pass "crashed trial falls back to B by firmware rule and is marked bad"
rauc status --detailed >>"$w/rauc.log" 2>&1 || true
echo "A/B update e2e: all checks passed"
