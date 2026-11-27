#!/usr/bin/env bash
# Release ZIP with the RAM installer (docs/INSTALL.md).
#
# Usage (root): os/installer/build.sh <image-out-dir> <zip-out-dir>
#   <image-out-dir> is the output of os/image/build.sh (boot-a.vfat,
#   rootfs.squashfs, vmlinuz, rootfs/ for kernel modules).
# Environment:
#   T3BOX_INSTALL_SIGNING_KEY  Ed25519 private key (PEM) that signs the
#                              manifest; its public key is built into the
#                              installer. Required: nothing is ever unsigned.
#   T3BOX_IMAGE_VERSION, T3BOX_IMAGE_ARCH, T3BOX_IMAGE_SUITE, T3BOX_IMAGE_MIRROR as for the image.
#   T3BOX_INSTALL_ID           fixed install id (tests); random otherwise.
# Output: <zip-out-dir>/zip/ (the files to copy onto a FAT32 card) and
#         <zip-out-dir>/t3box-os.zip.
set -euo pipefail
img=$(cd "$1" && pwd)
out=$(mkdir -p "$2" && cd "$2" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
key=${T3BOX_INSTALL_SIGNING_KEY:?T3BOX_INSTALL_SIGNING_KEY (Ed25519 PEM) is required}
arch=${T3BOX_IMAGE_ARCH:-arm64}
suite=${T3BOX_IMAGE_SUITE:-noble}
if [ "$arch" = arm64 ]; then default_mirror=http://ports.ubuntu.com/ubuntu-ports; else default_mirror=http://archive.ubuntu.com/ubuntu; fi
mirror=${T3BOX_IMAGE_MIRROR:-$default_mirror}
version=${T3BOX_IMAGE_VERSION:-0.0.0-dev}
id=${T3BOX_INSTALL_ID:-$(openssl rand -hex 8)}
. "$repo/os/layout.env"

for f in boot-a.vfat rootfs.squashfs vmlinuz; do [ -f "$img/$f" ] || { echo "missing $img/$f" >&2; exit 1; }; done
kver=$(ls "$img/rootfs/lib/modules" | sort -V | tail -1)

# Tool chroot of the target architecture; only binaries and their
# libraries go into the initramfs.
tools="$out/tools-root"
rm -rf "$tools"
mmdebstrap --arch="$arch" --variant=essential --include=busybox-static,fdisk,dosfstools,e2fsprogs,openssl,kmod,libzstd1 \
  "$suite" "$tools" "$mirror" >/dev/null
ir="$out/initramfs"
rm -rf "$ir"; mkdir -p "$ir"/{bin,sbin,etc/t3box,proc,sys,dev,run,mnt,lib,lib64,usr}
cp "$tools/bin/busybox" "$ir/bin/busybox"
for a in $(chroot "$tools" /bin/busybox --list); do [ -e "$ir/bin/$a" ] || ln -s busybox "$ir/bin/$a"; done
copy_bin() { # path in tools root
  install -D -m 0755 "$tools$1" "$ir$1"
  chroot "$tools" ldd "$1" 2>/dev/null | grep -oE '/[^ ]+\.so[^ ]*' | while read -r lib; do
    [ -e "$ir$lib" ] || install -D -m 0644 "$tools$(readlink -f "$tools$lib" | sed "s#^$tools##")" "$ir$lib"
  done
}
for b in /usr/sbin/sfdisk /usr/sbin/mkfs.fat /usr/sbin/mke2fs /usr/bin/openssl /usr/bin/kmod; do copy_bin "$b"; done
# Real tools take precedence over busybox applets (zstd modules, FAT32, GPT).
rm -f "$ir"/bin/{modprobe,insmod,rmmod,lsmod,depmod,mkfs.vfat,mkfs.ext2,fdisk}
ln -sf /usr/sbin/mkfs.fat "$ir/sbin/mkfs.vfat"
ln -sf /usr/sbin/mke2fs "$ir/sbin/mkfs.ext4"
ln -sf /usr/sbin/sfdisk "$ir/sbin/sfdisk"
ln -sf /usr/bin/openssl "$ir/bin/openssl"
for t in modprobe insmod depmod; do ln -sf /usr/bin/kmod "$ir/sbin/$t"; done
[ -d "$tools/usr/lib/ssl" ] && mkdir -p "$ir/usr/lib/ssl" && cp -a "$tools/usr/lib/ssl/openssl.cnf" "$ir/usr/lib/ssl/" 2>/dev/null || true

# Kernel modules for SD/USB/virtio storage and the filesystems used.
mods="vfat nls_cp437 nls_iso8859_1 ext4 virtio_blk virtio_pci virtio_scsi sd_mod usb_storage uas mmc_block sdhci_brcmstb sdhci_iproc"
: >"$ir/etc/t3box/modules"
for m in $mods; do
  deps=$(chroot "$img/rootfs" modprobe -S "$kver" --show-depends "$m" 2>/dev/null | awk '$1=="insmod"{print $2}') || true
  [ -n "$deps" ] || continue
  echo "$m" >>"$ir/etc/t3box/modules"
  for d in $deps; do install -D -m 0644 "$img/rootfs$d" "$ir$d"; done
done
cp "$img/rootfs/lib/modules/$kver/modules.order" "$img/rootfs/lib/modules/$kver/modules.builtin"* "$ir/lib/modules/$kver/" 2>/dev/null || true
# Store modules uncompressed: the initramfs is itself compressed, and a
# plain .ko loads with any modprobe/insmod regardless of zstd/xz support.
find "$ir/lib/modules/$kver" -name '*.ko.zst' -exec zstd -dq --rm {} \;
find "$ir/lib/modules/$kver" -name '*.ko.xz' -exec xz -d {} \;
sed -i -E 's/\.ko\.(zst|xz)$/.ko/' "$ir/lib/modules/$kver/modules.order" 2>/dev/null || true
depmod -b "$ir" "$kver"

install -m 0755 "$repo/os/installer/t3box-install" "$ir/init"
cp "$repo/os/layout.env" "$ir/etc/t3box/layout.env"
echo "$version" >"$ir/etc/t3box/version"
openssl pkey -in "$key" -pubout -out "$ir/etc/t3box/install.pub"
(cd "$ir" && find . | LC_ALL=C sort | cpio -o -H newc --quiet --reproducible | gzip -9n) >"$out/installer.cpio.gz"

# Card contents.
z="$out/zip"
rm -rf "$z"; mkdir -p "$z/t3box-install"
cp "$img/vmlinuz" "$z/vmlinuz"
cp "$out/installer.cpio.gz" "$z/installer.cpio.gz"
cp "$img/boot-a.vfat" "$z/t3box-install/boot.vfat"
cp "$img/rootfs.squashfs" "$z/t3box-install/rootfs.squashfs"
# Raspberry Pi firmware: boot the installer kernel + initramfs from FAT32.
cat >"$z/config.txt" <<CFG
# T3Box installer (docs/INSTALL.md). Replaced after installation.
[all]
arm_64bit=1
kernel=vmlinuz
initramfs installer.cpio.gz followkernel
CFG
[ -f "$img/bcm2712-rpi-5-b.dtb" ] && cp "$img/bcm2712-rpi-5-b.dtb" "$z/" && [ -d "$img/overlays" ] && cp -r "$img/overlays" "$z/"
printf 'console=serial0,115200 console=tty1 t3box.install_id=%s quiet\n' "$id" >"$z/cmdline.txt"
sha() { sha256sum "$1" | cut -d' ' -f1; }
cat >"$z/t3box-install/manifest.json" <<JSON
{
 "format": "t3box-install",
 "version": "$version",
 "install_id": "$id",
 "compatible": "t3box-pi5",
 "selector_mib": $T3BOX_SELECTOR_MIB,
 "boot_mib": $T3BOX_BOOT_MIB,
 "root_mib": $T3BOX_ROOT_MIB,
 "data_min_mib": $T3BOX_DATA_MIN_MIB,
 "boot.vfat_sha256": "$(sha "$z/t3box-install/boot.vfat")",
 "rootfs.squashfs_sha256": "$(sha "$z/t3box-install/rootfs.squashfs")",
 "installer.cpio.gz_sha256": "$(sha "$z/installer.cpio.gz")"
}
JSON
openssl pkeyutl -sign -inkey "$key" -rawin -in "$z/t3box-install/manifest.json" -out "$z/t3box-install/manifest.sig"
openssl pkeyutl -verify -pubin -inkey "$ir/etc/t3box/install.pub" -rawin -in "$z/t3box-install/manifest.json" -sigfile "$z/t3box-install/manifest.sig"
rm -f "$out/t3box-os.zip"
(cd "$z" && find . -type f | LC_ALL=C sort | TZ=UTC zip -X -q -0 "$out/t3box-os.zip" -@)
rm -rf "$tools"
echo "installer ZIP $version (install_id $id): $out/t3box-os.zip"
