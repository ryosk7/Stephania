#!/usr/bin/env bash
# A/B update end to end with real RAUC on loop devices and an emulated
# Raspberry Pi tryboot firmware (crate::bootctl, docs/UPDATE.md).
# Usage (root): ci/update-e2e.sh <stephania-bootctl>
# Needs rauc, sqlite3, squashfs-tools, dosfstools and mtools.
set -euo pipefail
bootctl=$(readlink -f "$1")
# CI uses verity bundles (needs device-mapper; RAUC v1.15 rejects the
# dm-verity status format of kernels >= 6.19, so CI runs on a 6.8 kernel). UPDATE_E2E_FORMAT=plain is
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

echo "using $(command -v rauc): $(rauc --version)"
mkdir -p "$w/dt" "$w/data" "$w/selector" "$w/mnt" "$w/rauc-data"
for s in boot-a boot-b rootfs-a rootfs-b; do
  truncate -s 64M "$w/$s.img"
  loops+=("$(losetup --find --show "$w/$s.img")")
done
declare -A dev=([boot-a]=${loops[0]} [boot-b]=${loops[1]} [rootfs-a]=${loops[2]} [rootfs-b]=${loops[3]})

# Emulated firmware: picks [all] or (once) [tryboot] from autoboot.txt and
# exposes the result like /proc/device-tree/chosen/bootloader.
printf '[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=3\n' >"$w/selector/autoboot.txt"
# A reboot also clears what a killed RAUC left behind (bundle mounts,
# dm-verity device, bundle loop device), as on real hardware.
reboot_cleanup() {
  [ -n "$svc" ] && { kill "$svc" 2>/dev/null || true; wait "$svc" 2>/dev/null || true; svc=""; }
  awk -v m="$w/mnt" 'index($2, m) == 1 {print $2}' /proc/mounts | sort -r | xargs -r umount -l 2>/dev/null || true
  dmsetup ls 2>/dev/null | awk '$1 ~ /^rauc/ {print $1}' | xargs -r -n1 dmsetup remove --force 2>/dev/null || true
  losetup -a | awk -F: '/\.raucb/ {print $1}' | xargs -r -n1 losetup -d 2>/dev/null || true
}
# Stand-in control database: schema_info.version as Sequel keeps it.
db_version() { sqlite3 "$STEPHANIA_CONTROL_DB" 'select version from schema_info'; }
migrate_to() { sqlite3 "$STEPHANIA_CONTROL_DB" "pragma journal_mode=wal; update schema_info set version=$1; insert into scenes values('from-v$1');" >/dev/null; }
firmware_boot() { # $1 = normal|tryboot
  reboot_cleanup
  local sect=all; [ "$1" = tryboot ] && sect=tryboot
  local p; p=$(awk -v s="[$sect]" '/^\[/{cur=$0} cur==s && /^boot_partition=/{split($0,a,"=");print a[2]}' "$w/selector/autoboot.txt")
  python3 -c 'import struct,sys;open(sys.argv[1],"wb").write(struct.pack(">I",int(sys.argv[2])));open(sys.argv[3],"wb").write(struct.pack(">I",int(sys.argv[4])))' \
    "$w/dt/partition" "$p" "$w/dt/tryboot" "$([ "$1" = tryboot ] && echo 1 || echo 0)"
  booted=$([ "$p" = 2 ] && echo A || echo B)
  # The running release is the booted rootfs's /etc/stephania-version
  # (the factory slot A, still empty here, counts as 1.0.0).
  local r=rootfs-a; [ "$booted" = B ] && r=rootfs-b
  unsquashfs -cat "${dev[$r]}" etc/stephania-version >"$w/stephania-version" 2>/dev/null || echo 1.0.0 >"$w/stephania-version"
  "$bootctl" restore-data >>"$w/restore.log" 2>&1 || fail "restore-data"
  start_rauc "$booted"
}
export STEPHANIA_CONTROL_DB=$w/data/control.db STEPHANIA_SNAPSHOT_DIR=$w/data/snapshot STEPHANIA_VERSION_FILE=$w/stephania-version
export STEPHANIA_AUTOBOOT=$w/selector/autoboot.txt STEPHANIA_BOOT_STATE=$w/data/bootctl.json STEPHANIA_BOOT_SLOTS=A=2,B=3 \
  STEPHANIA_DEVICE_TREE=$w/dt STEPHANIA_ENGINE_SOCKET=$w/engine.sock STEPHANIA_DATA_DIR=$w/data STEPHANIA_HEALTH_ATTEMPTS=2 STEPHANIA_REBOOT=true

cat >"$w/pre-install" <<SH
#!/bin/sh
"$bootctl" guard || exit 1
"$bootctl" check-version || exit 1
exec "$bootctl" snapshot
SH
chmod +x "$w/pre-install"
cat >"$w/system.conf" <<CONF
[system]
compatible=stephania-pi5
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
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj /CN=stephania-test-ca \
  -keyout "$w/ca.key" -out "$w/ca.pem" -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign >/dev/null 2>&1
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj /CN=stephania-test-release -keyout "$w/rel.key" -out "$w/rel.csr" >/dev/null 2>&1
openssl x509 -req -in "$w/rel.csr" -CA "$w/ca.pem" -CAkey "$w/ca.key" -CAcreateserial -days 2 -out "$w/rel.pem" \
  -extfile <(printf 'keyUsage=critical,digitalSignature\nextendedKeyUsage=codeSigning\n') >/dev/null 2>&1
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 2 -subj /CN=attacker \
  -keyout "$w/evil.key" -out "$w/evil.pem" >/dev/null 2>&1

make_bundle() { # $1 out, $2 version, $3 compatible, $4 key, $5 cert
  local d="$w/content-$2"
  rm -rf "$d"; mkdir -p "$d/root/etc"
  echo "$2" >"$d/root/etc/stephania-version"
  head -c 40M /dev/urandom >"$d/root/payload"
  mksquashfs "$d/root" "$d/rootfs.img" -noappend -quiet >/dev/null
  # Per-slot boot images and the production bundle hook, as
  # os/release/assemble.sh builds them.
  local slot
  for slot in a b; do
    truncate -s 16M "$d/boot-$slot.vfat"; mkfs.vfat -n "BOOT-${slot^^}" "$d/boot-$slot.vfat" >/dev/null
    echo "root=PARTLABEL=rootfs-$slot ro rauc.slot=${slot^^} stephania.test=$2" >"$d/cmdline.txt"
    mcopy -i "$d/boot-$slot.vfat" "$d/cmdline.txt" ::cmdline.txt
  done
  rm -f "$d/cmdline.txt"
  install -m 0755 "$root/os/rauc/bundle-hook" "$d/hook"
  printf '[update]\ncompatible=%s\nversion=%s\n[bundle]\nformat=%s\n[hooks]\nfilename=hook\n[image.rootfs]\nfilename=rootfs.img\n[image.boot]\nfilename=boot-a.vfat\nhooks=install\n' "$3" "$2" "$format" >"$d/manifest.raucm"
  rm -f "$d/root" -r
  rauc bundle --cert="$5" --key="$4" "$d" "$1" >>"$w/bundle.log" 2>&1
  sha256sum "$d/rootfs.img" | cut -d' ' -f1 >"$1.rootfs.sha256"
  stat -c %s "$d/rootfs.img" >"$1.rootfs.size"
}
slot_sha() { head -c "$(cat "$2.rootfs.size")" "${dev[$1]}" | sha256sum | cut -d' ' -f1; }
# The boot slot's cmdline must name its own rootfs and RAUC slot, from the
# installed version ($2).
boot_cmdline_ok() { # $1 a|b, $2 version
  local cmd; cmd=$(mtype -i "${dev[boot-$1]}" ::cmdline.txt 2>/dev/null) || return 1
  local want
  for want in "root=PARTLABEL=rootfs-$1" "rauc.slot=${1^^}" "stephania.test=$2"; do
    case " $cmd " in *" $want "*) ;; *) echo "boot-$1 cmdline lacks $want: $cmd" >&2; return 1 ;; esac
  done
}

sqlite3 "$STEPHANIA_CONTROL_DB" 'create table schema_info(version integer); insert into schema_info values(2); create table scenes(id text);'
firmware_boot normal
[ "$(primary)" = A ] || fail "initial primary"
pass "booted A (emulated firmware + RAUC service)"

make_bundle "$w/v2.raucb" 2.0.0 stephania-pi5 "$w/rel.key" "$w/rel.pem"
make_bundle "$w/evil.raucb" 6.6.6 stephania-pi5 "$w/evil.key" "$w/evil.pem"
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
boot_cmdline_ok b 2.0.0 || fail "boot-b does not boot rootfs-b as slot B"
pass "v2 written to inactive B and staged as a one-time trial (A still committed)"
pass "boot-b received the B boot image (root=PARTLABEL=rootfs-b rauc.slot=B)"

[ -f "$STEPHANIA_SNAPSHOT_DIR/control.db" ] || fail "pre-install snapshot missing"
firmware_boot tryboot
migrate_to 3   # v2 migrates the schema during its trial
"$bootctl" confirm >>"$w/confirm.log" 2>&1 || fail "confirm healthy trial"
[ ! -f "$STEPHANIA_SNAPSHOT_DIR/control.db" ] || fail "snapshot kept after commit"
[ "$(committed)" = 3 ] || fail "not committed"
firmware_boot normal
[ "$("$bootctl" get-current)" = B ] || fail "normal boot after commit"
[ "$(db_version)" = 3 ] || fail "committed data lost"
pass "healthy trial committed with its migrated data; normal boots stay on B"

# Anti-rollback: B now runs 2.0.0; an older signed release is refused.
make_bundle "$w/v1.raucb" 1.5.0 stephania-pi5 "$w/rel.key" "$w/rel.pem"
expect_refused "$w/v1.raucb" "older than the installed 2.0.0"
[ "$(primary)" = B ] && [ ! -f "$STEPHANIA_SNAPSHOT_DIR/control.db" ] || fail "refused downgrade changed state"
pass "bundle older than the running release refused (anti-rollback)"

make_bundle "$w/v3.raucb" 3.0.0 stephania-pi5 "$w/rel.key" "$w/rel.pem"
rauc install "$w/v3.raucb" >>"$w/install.log" 2>&1 || fail "install v3"
[ "$(primary)" = A ] || fail "v3 pending on A"
boot_cmdline_ok a 3.0.0 || fail "boot-a does not boot rootfs-a as slot A"
[ "$(stat -c %a "$STEPHANIA_SNAPSHOT_DIR")" = 700 ] && [ "$(stat -c %a "$STEPHANIA_SNAPSHOT_DIR/control.db")" = 600 ] || fail "snapshot readable by others"
pass "boot-a received the A boot image; the data snapshot is private (0700/0600)"
kill "${pids[-1]}"; wait "${pids[-1]}" 2>/dev/null || true; rm -f "$w/engine.sock"  # engine broken in v3
firmware_boot tryboot
migrate_to 4
"$bootctl" confirm >>"$w/confirm.log" 2>&1 && fail "unhealthy trial confirmed"
firmware_boot normal
[ "$(db_version)" = 3 ] || fail "data not restored after rollback: $(db_version)"
[ "$(sqlite3 "$STEPHANIA_CONTROL_DB" "select count(*) from scenes where id='from-v4'")" = 0 ] || fail "trial writes survived"
[ "$("$bootctl" get-current)" = B ] && [ "$(committed)" = 3 ] || fail "rollback after failed health"
[ "$("$bootctl" get-state A)" = bad ] || fail "A not marked bad"
[ ! -e "$STEPHANIA_SNAPSHOT_DIR/control.db" ] || fail "snapshot kept after the restore"
pass "unhealthy trial (engine down) rolled back to B, marked bad, data restored to the pre-update snapshot"

python3 "$root/ci/fake_engine.py" "$w/engine.sock" idle & pids+=($!)
sleep 0.5
rauc install "$w/v3.raucb" >>"$w/install.log" 2>&1 || fail "reinstall v3"
firmware_boot tryboot
migrate_to 4
firmware_boot normal   # trial crashed / watchdog / power loss before confirmation
[ "$(db_version)" = 3 ] || fail "data not restored after crashed trial"
"$bootctl" confirm >>"$w/confirm.log" 2>&1 || fail "confirm after crashed trial"
sqlite3 "$STEPHANIA_CONTROL_DB" "insert into scenes values('after-rollback')"
firmware_boot normal
[ "$(sqlite3 "$STEPHANIA_CONTROL_DB" "select count(*) from scenes where id='after-rollback'")" = 1 ] || fail "second restore clobbered newer data"
[ "$("$bootctl" get-current)" = B ] && [ "$(primary)" = B ] && [ "$("$bootctl" get-state A)" = bad ] || fail "crashed trial"
pass "crashed trial falls back to B by firmware rule, marked bad, data restored exactly once"
# Deadline: a trial that never gets confirmed is rejected by the timer.
rauc install "$w/v3.raucb" >>"$w/install.log" 2>&1 || fail "install v3 (deadline)"
firmware_boot tryboot
"$bootctl" deadline >>"$w/confirm.log" 2>&1 || fail "deadline"
firmware_boot normal
[ "$("$bootctl" get-current)" = B ] && [ "$("$bootctl" get-state A)" = bad ] || fail "deadline rollback"
pass "unconfirmed trial rejected at the deadline"
rauc status --detailed >>"$w/rauc.log" 2>&1 || true
echo "A/B update e2e: all checks passed"
