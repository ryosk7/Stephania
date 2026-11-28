#!/usr/bin/env bash
# Reference image build (docs/IMAGE.md): Raspberry Pi OS Lite reference
# (docs/MASTER-ARCHITECTURE.md §6) — Debian 13 trixie plus, for the Pi
# variant, the Raspberry Pi archive (kernel, device trees), the way pi-gen
# builds Raspberry Pi OS Lite — with the Stephania services, assembled into the
# A/B GPT layout of docs/UPDATE.md.
#
# Usage (root): os/image/build.sh <out-dir> <bin-dir>
#   <bin-dir> holds stephania, stephania-phone, stephania-ble, stephania-provision, stephania-pki, stephania-bootctl, stephania-maint, steph built
#   for the target architecture against Debian trixie
#   (ci/images/device-build.Dockerfile), so library sonames match.
# Environment:
#   STEPHANIA_IMAGE_ARCH   arm64 (default) | amd64 (QEMU smoke tests)
#   STEPHANIA_IMAGE_SUITE  trixie (default)
#   STEPHANIA_IMAGE_MIRROR http://deb.debian.org/debian
#   STEPHANIA_IMAGE_DEBIAN_KEYRING Debian archive keyring (default: the host's
#                      debian-archive-keyring; CI takes it from debian:trixie
#                      because older hosts lack the trixie keys)
#   STEPHANIA_IMAGE_RPI_MIRROR http://archive.raspberrypi.com/debian (pi variant)
#   STEPHANIA_IMAGE_RPI_KEYRING Raspberry Pi archive keyring (binary); default:
#                      downloaded and checked against RPI_KEY_FINGERPRINT
#   STEPHANIA_IMAGE_COMPONENTS main
#   STEPHANIA_IMAGE_KERNEL linux-image-<arch> (QEMU) | linux-image-rpi-2712 (pi)
#   STEPHANIA_IMAGE_EXTRA  runtime libraries of the native engine
#   STEPHANIA_IMAGE_VERSION version string written to /etc/stephania-version
#   STEPHANIA_IMAGE_VARIANT qemu (default: Debian kernel, direct kernel boot) |
#                      pi (Raspberry Pi 5: Raspberry Pi kernel, DTB, overlays,
#                      config.txt for the Pi firmware; docs/IMAGE.md)
#   STEPHANIA_RAUC_KEYRING  release CA certificate (PEM) RAUC trusts; without it
#                      the image refuses every bundle
#   SOURCE_DATE_EPOCH   fixed timestamps for reproducible filesystems
#
# Outputs: rootfs.squashfs, boot-a.vfat, boot-b.vfat, vmlinuz, initrd.img, stephania.img
# (GPT: selector, boot-a, boot-b, rootfs-a, rootfs-b, data).
set -euo pipefail
out=$(mkdir -p "$1" && cd "$1" && pwd)
bins=$(cd "$2" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
arch=${STEPHANIA_IMAGE_ARCH:-arm64}
suite=${STEPHANIA_IMAGE_SUITE:-trixie}
mirror=${STEPHANIA_IMAGE_MIRROR:-http://deb.debian.org/debian}
variant=${STEPHANIA_IMAGE_VARIANT:-qemu}
case $variant in
  qemu) default_kernel=linux-image-$arch ;;
  pi) [ "$arch" = arm64 ] || { echo "the pi variant is arm64 only" >&2; exit 1; }; default_kernel=linux-image-rpi-2712 ;;
  *) echo "unknown STEPHANIA_IMAGE_VARIANT $variant" >&2; exit 1 ;;
esac
kernel=${STEPHANIA_IMAGE_KERNEL:-$default_kernel}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$(dirname "$0")" log -1 --format=%ct 2>/dev/null || date +%s)}
extra=${STEPHANIA_IMAGE_EXTRA:-libasound2t64,libx264-164,libavcodec61,libavutil59}
version=${STEPHANIA_IMAGE_VERSION:-0.0.0-dev}
components=${STEPHANIA_IMAGE_COMPONENTS:-main}
root="$out/rootfs"
. "$repo/os/layout.env"

for b in stephania stephania-phone stephania-ble stephania-provision stephania-pki stephania-bootctl stephania-maint steph; do
  [ -x "$bins/$b" ] || { echo "missing $bins/$b" >&2; exit 1; }
done

# Runtime packages only. Ruby gems with native extensions are built with
# temporary build packages that are purged again (no compilers on device).
# systemd-timesyncd is only a Recommends of systemd, so it is listed (and
# thereby kept by the autoremove in customize.sh): RTMPS certificate checks
# need a real clock. libdbus-1-3 is what stephania-ble links against.
runtime=systemd,systemd-sysv,systemd-timesyncd,udev,dbus,libdbus-1-3,kmod,iproute2,util-linux,mount,network-manager,bluez,avahi-daemon,avahi-utils,rauc,rauc-service,sqlite3,openssh-server,nftables,ca-certificates,ruby,curl,openssl,polkitd,$kernel,initramfs-tools${extra:+,$extra}
build=build-essential,ruby-dev,libsqlite3-dev,libssl-dev,pkgconf

rm -rf "$root"
# Optional package cache across builds (CI): downloads are reused, the image
# content is unchanged (apt-get clean still runs in customize.sh).
cache_args=()
if [ -n "${STEPHANIA_IMAGE_APT_CACHE:-}" ]; then
  mkdir -p "$STEPHANIA_IMAGE_APT_CACHE"
  cache_args=(--skip=download/empty --skip=essential/unlink
    --setup-hook='mkdir -p "$1"/var/cache/apt/archives/'
    --setup-hook="sync-in '$STEPHANIA_IMAGE_APT_CACHE' /var/cache/apt/archives/"
    --customize-hook="sync-out /var/cache/apt/archives '$STEPHANIA_IMAGE_APT_CACHE'")
fi
# Package sources: Debian, plus the Raspberry Pi archive for the Pi
# variant. The Raspberry Pi key is pinned by fingerprint.
RPI_KEY_FINGERPRINT=CF8A1AF502A2AA2D763BAE7E82B129927FA3303E
sources=("deb $mirror $suite $components")
# One directory of keyrings: mmdebstrap passes a single keyring *file* to
# apt as the only trusted file, so a second --keyring would replace the
# Debian keys instead of adding to them.
keydir="$out/apt-keyrings"; rm -rf "$keydir"; mkdir -p "$keydir"
cp "${STEPHANIA_IMAGE_DEBIAN_KEYRING:-/usr/share/keyrings/debian-archive-keyring.gpg}" "$keydir/debian-archive-keyring.gpg"
if [ "$variant" = pi ]; then
  rpi_mirror=${STEPHANIA_IMAGE_RPI_MIRROR:-http://archive.raspberrypi.com/debian}
  rpi_keyring=${STEPHANIA_IMAGE_RPI_KEYRING:-$out/raspberrypi-archive.gpg}
  if [ -z "${STEPHANIA_IMAGE_RPI_KEYRING:-}" ]; then
    curl -fsSL --retry 3 https://archive.raspberrypi.com/debian/raspberrypi.gpg.key | gpg --dearmor >"$rpi_keyring"
  fi
  got=$(gpg --show-keys --with-colons "$rpi_keyring" | awk -F: '$1=="fpr"{print $10; exit}')
  [ "$got" = "$RPI_KEY_FINGERPRINT" ] || { echo "Raspberry Pi archive key fingerprint $got, expected $RPI_KEY_FINGERPRINT" >&2; exit 1; }
  sources+=("deb $rpi_mirror $suite main")
  cp "$rpi_keyring" "$keydir/raspberrypi-archive.gpg"
fi
keyrings=(--keyring="$keydir")
# force-unsafe-io: no fsync per package; the tree is packed into images afterwards.
mmdebstrap --arch="$arch" --variant=apt --include="$runtime,$build" "${keyrings[@]}" "${cache_args[@]}" --dpkgopt=force-unsafe-io \
  --customize-hook="$repo/os/image/customize.sh \"\$1\" '$repo' '$bins' '$version' '$build' '$runtime'" \
  "$suite" "$root" "${sources[@]}"

if [ -n "${STEPHANIA_RAUC_KEYRING:-}" ]; then
  install -m 0644 "$STEPHANIA_RAUC_KEYRING" "$root/etc/rauc/keyring.pem"
fi
# Package inventory for the SBOM (ci/sbom.py --dpkg) and source obligations
# (docs/LICENSING.md).
chroot "$root" dpkg-query -W -f='${Package}\t${Version}\t${source:Package}\n' | LC_ALL=C sort >"$out/dpkg.tsv"

# Kernel and initramfs (direct kernel boot in QEMU; Pi firmware loads the
# same files from the boot partition).
kver=$(ls "$root/lib/modules" | sort -V | tail -1)
cp -L "$root/boot/vmlinuz-$kver" "$out/vmlinuz"
cp -L "$root/boot/initrd.img-$kver" "$out/initrd.img"
if [ "$variant" = pi ]; then
  # Raspberry Pi kernel packages ship device trees and overlays under
  # /usr/lib/linux-image-<version>.
  dt="$root/usr/lib/linux-image-$kver"
  [ -f "$dt/broadcom/bcm2712-rpi-5-b.dtb" ] || { echo "no Pi 5 device tree in $dt" >&2; exit 1; }
  cp "$dt/broadcom/bcm2712-rpi-5-b.dtb" "$out/"
  rm -rf "$out/overlays"; cp -r "$dt/overlays" "$out/overlays"
fi

# Reproducible: fixed timestamps, no xattr ordering surprises.
mksquashfs "$root" "$out/rootfs.squashfs" -noappend -comp zstd -quiet -e boot/efi \
  -mkfs-time "$SOURCE_DATE_EPOCH" -all-time "$SOURCE_DATE_EPOCH" -no-xattrs
# Fixed slot sizes (os/layout.env) so every future update fits its slot.
rootfs_mib=$STEPHANIA_ROOT_MIB
[ "$(stat -c %s "$out/rootfs.squashfs")" -le $((rootfs_mib * 1048576)) ] || { echo "rootfs.squashfs exceeds the ${rootfs_mib} MiB slot" >&2; exit 1; }

# Boot partitions. QEMU: cmdline.txt/config.txt only (the harness passes the
# kernel). Pi: kernel, initramfs, DTB and overlays for the firmware.
boot_dir=$(mktemp -d)
vfat() { # file label size-MiB
  truncate -s "${3}M" "$1"
  mkfs.vfat --invariant -i 7e3b0000 -n "$2" "$1" >/dev/null 2>&1 || mkfs.vfat -i 7e3b0000 -n "$2" "$1" >/dev/null
}
for slot in a b; do
  upper=$(echo "$slot" | tr a-z A-Z)
  cmd="root=PARTLABEL=rootfs-$slot ro rootfstype=squashfs init=/usr/local/sbin/stephania-overlay-init rauc.slot=$upper"
  {
    if [ "$variant" = pi ]; then
      printf '# Stephania boot slot %s (Raspberry Pi 5)\n[all]\narm_64bit=1\nkernel=vmlinuz\ninitramfs initrd.img followkernel\nenable_uart=1\n' "$upper"
      cmd="console=serial0,115200 console=tty1 $cmd rootwait"
    fi
    cat "$repo/os/firmware/config.txt.fragment"
  } >"$boot_dir/config.txt"
  echo "$cmd" >"$boot_dir/cmdline.txt"
  vfat "$out/boot-$slot.vfat" "BOOT-$upper" "$STEPHANIA_BOOT_MIB"
  touch -d "@$SOURCE_DATE_EPOCH" "$boot_dir"/*
  mcopy -m -i "$out/boot-$slot.vfat" "$boot_dir/cmdline.txt" "$boot_dir/config.txt" ::
  if [ "$variant" = pi ]; then
    mcopy -i "$out/boot-$slot.vfat" "$out/vmlinuz" "$out/initrd.img" "$out/bcm2712-rpi-5-b.dtb" ::
    mcopy -s -i "$out/boot-$slot.vfat" "$out/overlays" ::
  fi
done
vfat "$out/selector.vfat" SELECTOR "$STEPHANIA_SELECTOR_MIB"
printf '[all]\ntryboot_a_b=1\nboot_partition=2\n[tryboot]\nboot_partition=3\n' >"$boot_dir/autoboot.txt"
mcopy -i "$out/selector.vfat" "$boot_dir/autoboot.txt" ::autoboot.txt

# Persistent data partition, pre-populated with the directory skeleton the
# read-only root bind-mounts (os/image/customize.sh writes fstab).
data_dir=$(mktemp -d)
mkdir -p "$data_dir/stephania/varlib" "$data_dir/stephania/config" "$data_dir/stephania/update" "$data_dir/stephania/rauc"
truncate -s "${STEPHANIA_DATA_MIN_MIB}M" "$out/data.ext4"
E2FSPROGS_FAKE_TIME=$SOURCE_DATE_EPOCH mkfs.ext4 -q -L data -U 7e3b0000-0000-4000-8000-000000000006 -E hash_seed=7e3b0000-0000-4000-8000-000000000006,root_owner=0:0 -d "$data_dir" "$out/data.ext4"

# GPT disk image.
img="$out/stephania.img"
rm -f "$img"
sizes=("$STEPHANIA_SELECTOR_MIB" "$STEPHANIA_BOOT_MIB" "$STEPHANIA_BOOT_MIB" "$rootfs_mib" "$rootfs_mib" "$STEPHANIA_DATA_MIN_MIB")
total=2; for s in "${sizes[@]}"; do total=$((total + s)); done
truncate -s "${total}M" "$img"
sfdisk -q "$img" <<EOF
label: gpt
size=${STEPHANIA_SELECTOR_MIB}MiB, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name=selector
size=${STEPHANIA_BOOT_MIB}MiB, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name=boot-a
size=${STEPHANIA_BOOT_MIB}MiB, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, name=boot-b
size=${rootfs_mib}MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=rootfs-a
size=${rootfs_mib}MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=rootfs-b
size=${STEPHANIA_DATA_MIN_MIB}MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=data
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
echo "stephania image $version ($arch, $suite, $variant): $img"
