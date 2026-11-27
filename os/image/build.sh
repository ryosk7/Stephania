#!/usr/bin/env bash
# Reference image build (docs/IMAGE.md): Debian-family root filesystem with
# the T3Box services, assembled into the A/B GPT layout of docs/UPDATE.md.
#
# Usage (root): os/image/build.sh <out-dir> <bin-dir>
#   <bin-dir> holds t3box, t3box-phone, t3box-provision, t3box-pki, t3box-bootctl built
#   for the target architecture.
# Environment:
#   T3BOX_IMAGE_ARCH   arm64 (default) | amd64 (local iteration only)
#   T3BOX_IMAGE_SUITE  noble (default: same distribution as the CI runner
#                      that builds the binaries, so library sonames match;
#                      see docs/IMAGE.md for the Raspberry Pi OS variant)
#   T3BOX_IMAGE_MIRROR ports.ubuntu.com (arm64) / archive.ubuntu.com (amd64)
#   T3BOX_IMAGE_COMPONENTS main,universe
#   T3BOX_IMAGE_KERNEL linux-image-virtual (generic kernel, QEMU variant)
#   T3BOX_IMAGE_EXTRA  runtime libraries of the native engine
#   T3BOX_IMAGE_VERSION version string written to /etc/t3box-version
#
# Outputs: rootfs.squashfs, boot-a.vfat, boot-b.vfat, vmlinuz, initrd.img, t3box.img
# (GPT: selector, boot-a, boot-b, rootfs-a, rootfs-b, data).
set -euo pipefail
out=$(mkdir -p "$1" && cd "$1" && pwd)
bins=$(cd "$2" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
arch=${T3BOX_IMAGE_ARCH:-arm64}
suite=${T3BOX_IMAGE_SUITE:-noble}
if [ "$arch" = arm64 ]; then default_mirror=http://ports.ubuntu.com/ubuntu-ports; else default_mirror=http://archive.ubuntu.com/ubuntu; fi
mirror=${T3BOX_IMAGE_MIRROR:-$default_mirror}
kernel=${T3BOX_IMAGE_KERNEL:-linux-image-virtual}
extra=${T3BOX_IMAGE_EXTRA:-libasound2t64,libx264-164,libavcodec60,libavutil58}
version=${T3BOX_IMAGE_VERSION:-0.0.0-dev}
components=${T3BOX_IMAGE_COMPONENTS:-main,universe}
root="$out/rootfs"
. "$repo/os/layout.env"

for b in t3box t3box-phone t3box-provision t3box-pki t3box-bootctl; do
  [ -x "$bins/$b" ] || { echo "missing $bins/$b" >&2; exit 1; }
done

# Runtime packages only. Ruby gems with native extensions are built with
# temporary build packages that are purged again (no compilers on device).
runtime=systemd,systemd-sysv,udev,dbus,kmod,iproute2,util-linux,mount,network-manager,bluez,avahi-daemon,avahi-utils,rauc,rauc-service,sqlite3,ca-certificates,ruby,curl,openssl,polkitd,$kernel,initramfs-tools${extra:+,$extra}
build=build-essential,ruby-dev,libsqlite3-dev,libssl-dev,pkgconf

rm -rf "$root"
# Optional package cache across builds (CI): downloads are reused, the image
# content is unchanged (apt-get clean still runs in customize.sh).
cache_args=()
if [ -n "${T3BOX_IMAGE_APT_CACHE:-}" ]; then
  mkdir -p "$T3BOX_IMAGE_APT_CACHE"
  cache_args=(--skip=download/empty --skip=essential/unlink
    --setup-hook='mkdir -p "$1"/var/cache/apt/archives/'
    --setup-hook="sync-in '$T3BOX_IMAGE_APT_CACHE' /var/cache/apt/archives/"
    --customize-hook="sync-out /var/cache/apt/archives '$T3BOX_IMAGE_APT_CACHE'")
fi
mmdebstrap --arch="$arch" --variant=apt --components="$components" --include="$runtime,$build" "${cache_args[@]}" \
  --customize-hook="$repo/os/image/customize.sh \"\$1\" '$repo' '$bins' '$version' '$build' '$runtime'" \
  "$suite" "$root" "$mirror"

# Kernel and initramfs for direct kernel boot (QEMU variant).
kver=$(ls "$root/lib/modules" | sort -V | tail -1)
cp -L "$root/boot/vmlinuz-$kver" "$out/vmlinuz"
cp -L "$root/boot/initrd.img-$kver" "$out/initrd.img"

mksquashfs "$root" "$out/rootfs.squashfs" -noappend -comp zstd -quiet -e boot/efi
# Fixed slot sizes (os/layout.env) so every future update fits its slot.
rootfs_mib=$T3BOX_ROOT_MIB
[ "$(stat -c %s "$out/rootfs.squashfs")" -le $((rootfs_mib * 1048576)) ] || { echo "rootfs.squashfs exceeds the ${rootfs_mib} MiB slot" >&2; exit 1; }

# Boot partition content (identical for A and B in the QEMU variant; the
# Pi variant adds firmware, kernel, DTBs and config.txt).
boot_dir=$(mktemp -d)
cat "$repo/os/firmware/config.txt.fragment" >"$boot_dir/config.txt"
for slot in a b; do
  upper=$(echo "$slot" | tr a-z A-Z)
  printf 'root=PARTLABEL=rootfs-%s ro rootfstype=squashfs init=/usr/local/sbin/t3box-overlay-init rauc.slot=%s\n' "$slot" "$upper" >"$boot_dir/cmdline.txt"
  truncate -s 64M "$out/boot-$slot.vfat"; mkfs.vfat -n "BOOT-$upper" "$out/boot-$slot.vfat" >/dev/null
  mcopy -i "$out/boot-$slot.vfat" "$boot_dir/cmdline.txt" ::cmdline.txt
  mcopy -i "$out/boot-$slot.vfat" "$boot_dir/config.txt" ::config.txt
done
truncate -s "${T3BOX_SELECTOR_MIB}M" "$out/selector.vfat"; mkfs.vfat -n SELECTOR "$out/selector.vfat" >/dev/null
printf '[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=3\n' >"$boot_dir/autoboot.txt"
mcopy -i "$out/selector.vfat" "$boot_dir/autoboot.txt" ::autoboot.txt

# Persistent data partition, pre-populated with the directory skeleton the
# read-only root bind-mounts (os/image/customize.sh writes fstab).
data_dir=$(mktemp -d)
mkdir -p "$data_dir/t3box/varlib" "$data_dir/t3box/config" "$data_dir/t3box/update" "$data_dir/t3box/rauc"
truncate -s "${T3BOX_DATA_MIN_MIB}M" "$out/data.ext4"; mkfs.ext4 -q -L data -d "$data_dir" "$out/data.ext4"

# GPT disk image.
img="$out/t3box.img"
rm -f "$img"
sizes=("$T3BOX_SELECTOR_MIB" "$T3BOX_BOOT_MIB" "$T3BOX_BOOT_MIB" "$rootfs_mib" "$rootfs_mib" "$T3BOX_DATA_MIN_MIB")
total=2; for s in "${sizes[@]}"; do total=$((total + s)); done
truncate -s "${total}M" "$img"
sfdisk -q "$img" <<EOF
label: gpt
size=${T3BOX_SELECTOR_MIB}MiB, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name=selector
size=${T3BOX_BOOT_MIB}MiB, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name=boot-a
size=${T3BOX_BOOT_MIB}MiB, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name=boot-b
size=${rootfs_mib}MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=rootfs-a
size=${rootfs_mib}MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=rootfs-b
size=${T3BOX_DATA_MIN_MIB}MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=data
EOF
write_part() { # $1 partition number, $2 file
  local start; start=$(sfdisk -J "$img" | python3 -c 'import json,sys;print(json.load(sys.stdin)["partitiontable"]["partitions"][int(sys.argv[1])-1]["start"])' "$1")
  dd if="$2" of="$img" bs=512 seek="$start" conv=notrunc,sparse status=none
}
write_part 1 "$out/selector.vfat"
write_part 2 "$out/boot-a.vfat"
write_part 3 "$out/boot-b.vfat"
write_part 4 "$out/rootfs.squashfs"
write_part 6 "$out/data.ext4"
# rootfs-b stays empty: the first update fills it (RAUC writes the inactive slot).
rm -rf "$boot_dir" "$data_dir"
sfdisk -l "$img"
echo "t3box image $version ($arch, $suite): $img"
