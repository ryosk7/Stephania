#!/usr/bin/env bash
# RAM installer end-to-end (docs/INSTALL.md) under QEMU (amd64, KVM when
# available): the release ZIP copied onto a FAT32 "card" next to unrelated
# disks; refusal when the target is ambiguous; power cuts after every stage
# with resume or a clear recovery instruction; then the installed card boots
# and passes the image smoke test.
# Usage (root): ci/installer-e2e.sh <image-out-dir> <installer-out-dir>
set -Eeuo pipefail
img=$(cd "$1" && pwd)
inst=$(cd "$2" && pwd)
zip="$inst/zip"
work=$(mktemp -d)
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; for l in "$work"/*.log; do echo "--- $l"; tail -40 "$l"; done; exit 1; }
accel=tcg; [ -w /dev/kvm ] && accel=kvm

card() { # path size-MiB: MBR + one FAT32 partition with the ZIP contents
  rm -f "$1"; truncate -s "${2}M" "$1"
  printf 'label: dos\nstart=2048, type=c\n' | sfdisk -q "$1"
  mkfs.fat -F 32 -n STEPHANIA --offset 2048 "$1" >/dev/null
  (cd "$zip" && find . -mindepth 1 -maxdepth 1 -exec mcopy -s -i "$1@@1M" {} ::/ \;)
}
decoy() { # an unrelated USB stick: FAT32 with files, no Stephania manifest
  rm -f "$1"; truncate -s 64M "$1"
  printf 'label: dos\nstart=2048, type=c\n' | sfdisk -q "$1"
  mkfs.fat -F 32 -n PHOTOS --offset 2048 "$1" >/dev/null
  echo "holiday photos" >"$work/readme.txt"; mcopy -i "$1@@1M" "$work/readme.txt" ::/readme.txt
}
run_installer() { # log extra-cmdline disks...
  local log=$1 extra=$2; shift 2
  local drives=()
  for d in "$@"; do drives+=(-drive "file=$d,if=virtio,format=raw"); done
  timeout 900 qemu-system-x86_64 -M q35 -accel $accel -m 3072 -smp 2 -nographic -no-reboot \
    -kernel "$zip/vmlinuz" -initrd "$zip/installer.cpio.gz" "${drives[@]}" \
    -append "$(cat "$zip/cmdline.txt") console=ttyS0 stephania.install_poweroff=1 panic=-1 $extra" >"$log" 2>&1 || true
  grep -a "STEPHANIA-INSTALL" "$log" | sed 's/^/    /' || true
}
digest() { sha256sum "$1" | cut -d' ' -f1; }

# 1. Ambiguous: two cards with the same ZIP. Nothing may be written.
card "$work/a.img" 4096; card "$work/b.img" 4096
ha=$(digest "$work/a.img"); hb=$(digest "$work/b.img")
run_installer "$work/ambiguous.log" "" "$work/a.img" "$work/b.img"
grep -aq "expected exactly one Stephania install medium, found 2" "$work/ambiguous.log" || fail "ambiguous target not refused"
[ "$(digest "$work/a.img")" = "$ha" ] && [ "$(digest "$work/b.img")" = "$hb" ] || fail "a disk changed although the target was ambiguous"
pass "two candidate media: refused, both unchanged"

# 2. Power cut after verify (before any write): card unchanged, then a normal run works.
card "$work/c.img" 4096; decoy "$work/usb.img"
hc=$(digest "$work/c.img"); hu=$(digest "$work/usb.img")
run_installer "$work/verify.log" "stephania.install_fail_at=verify" "$work/usb.img" "$work/c.img"
grep -aq "TEST power cut after stage verify" "$work/verify.log" || fail "no power cut after verify"
[ "$(digest "$work/c.img")" = "$hc" ] || fail "card changed before the destructive stage"
pass "power cut after verification leaves the card untouched"

# 3. Power cut after partitioning, before the payload is staged: recovery message.
run_installer "$work/partition.log" "stephania.install_fail_at=partition" "$work/usb.img" "$work/c.img"
grep -aq "stage partition" "$work/partition.log" || fail "partition stage not reached"
run_installer "$work/partition-resume.log" "" "$work/usb.img" "$work/c.img"
grep -aq "RECOVERY: power was lost before the installation files were saved" "$work/partition-resume.log" || fail "no recovery instruction after a cut in the unsafe window"
pass "power cut in the partition window: installer boots again and asks for the ZIP to be copied again"

# 4/5. Power cuts after staging and after writing: the next boot resumes and completes.
for stage in staged written; do
  card "$work/c.img" 4096
  run_installer "$work/$stage.log" "stephania.install_fail_at=$stage" "$work/usb.img" "$work/c.img"
  grep -aq "TEST power cut after stage $stage" "$work/$stage.log" || fail "no power cut after $stage"
  run_installer "$work/$stage-resume.log" "" "$work/usb.img" "$work/c.img"
  grep -aq "resuming at stage $stage" "$work/$stage-resume.log" || fail "did not resume at $stage"
  grep -aq "RESULT PASS" "$work/$stage-resume.log" || fail "resume after $stage did not complete"
  pass "power cut after $stage: resumed and completed"
done
[ "$(digest "$work/usb.img")" = "$hu" ] || fail "the unrelated USB disk was modified"
pass "unrelated USB disk never touched"

# 6. The installed card: layout, selector content, and it boots.
sfdisk -J "$work/c.img" | python3 -c '
import json,sys; p=json.load(sys.stdin)["partitiontable"]
assert p["label"]=="gpt", p["label"]
names=[x.get("name") for x in p["partitions"]]
assert names==["selector","boot-a","boot-b","rootfs-a","rootfs-b","data"], names' || fail "unexpected partition table"
off=$(( $(sfdisk -J "$work/c.img" | python3 -c 'import json,sys;print(json.load(sys.stdin)["partitiontable"]["partitions"][0]["start"])') * 512 ))
mdir -b -i "$work/c.img@@$off" ::/ >"$work/selector.txt"
grep -qx '::/autoboot.txt' "$work/selector.txt" && ! grep -q 'installer.cpio.gz' "$work/selector.txt" \
  || { cat "$work/selector.txt"; fail "selector content"; }
mtype -i "$work/c.img@@$off" ::/stephania-install/state | grep -qx 'stage=done' || fail "install state not done"
pass "GPT A/B layout; selector holds autoboot.txt and the install record"
mkdir -p "$work/boot"; cp "$img/vmlinuz" "$img/initrd.img" "$work/boot/"; cp "$work/c.img" "$work/boot/stephania.img"
ci/qemu-smoke.sh "$work/boot" amd64 || fail "installed card does not boot"
pass "installed card boots and passes the image smoke test"
echo "installer end-to-end: passed"
