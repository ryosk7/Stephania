#!/usr/bin/env bash
# Executed inside a disposable Ubuntu ARM64 image, never on the CI host.
set -Eeuo pipefail
mkdir -p /image
work="$(mktemp -d)"
cd "$work"
apt-get download linux-image-generic
meta="$(find . -name 'linux-image-generic_*.deb' -print -quit)"
image="$(dpkg-deb -f "$meta" Depends | grep -oE 'linux-image-[0-9][^ ,(]*-generic' | head -1)"
test -n "$image"
version="${image#linux-image-}"
apt-get download "$image" "linux-modules-$version" "linux-modules-extra-$version"
mkdir root
for pkg in linux-image-[0-9]*.deb linux-modules-*.deb; do dpkg-deb -x "$pkg" root; done
if gzip -t "root/boot/vmlinuz-$version" 2>/dev/null; then gzip -dc "root/boot/vmlinuz-$version" > /image/Image; else cp "root/boot/vmlinuz-$version" /image/Image; fi
mkdir -p root/{bin,sbin,usr/bin,usr/sbin,proc,sys,dev,tmp,etc,run}
copy_tool() {
  local src="$1" dest="$2" lib
  install -D -m 755 "$(readlink -f "$src")" "root/$dest"
  while read -r lib; do [[ -z "$lib" ]] || cp -L --parents "$lib" root; done < <(ldd "$src" 2>/dev/null | awk '/=> \/|^[[:space:]]*\// {for(i=1;i<=NF;i++) if($i ~ /^\//) {print $i;break}}')
}
copy_tool "$(command -v busybox)" bin/busybox
copy_tool "$(command -v kmod)" bin/kmod
copy_tool "$(command -v v4l2-ctl)" bin/v4l2-ctl
copy_tool "$(command -v aplay)" bin/aplay
for name in sh mount mkdir sleep cat grep awk head poweroff timeout dd seq ls uname; do ln -sf busybox "root/bin/$name"; done
ln -sf kmod root/bin/modprobe
# Normalize usrmerged module paths before depmod.
if [[ -d root/usr/lib/modules ]]; then
  mkdir -p root/lib/modules
  cp -a root/usr/lib/modules/. root/lib/modules/
  rm -rf root/usr/lib/modules
fi
depmod -b "$work/root" "$version"
mkdir selected
for mod in vivid snd-aloop; do
  modprobe -d "$work/root" -S "$version" --show-depends "$mod" > "$mod.deps"
  while read -r op file rest; do
    if [[ "$op" == insmod ]]; then
      rel="${file#"$work/root/"}"
      mkdir -p "selected/$(dirname "$rel")"
      cp "$file" "selected/$rel"
    fi
  done < "$mod.deps"
done
test -n "$(find selected -name 'vivid.ko*' -print -quit)"
test -n "$(find selected -name 'snd-aloop.ko*' -print -quit)"
rm -rf root/lib/modules
mkdir -p root/lib/modules
cp -a selected/lib/modules/. root/lib/modules/
find root/lib/modules -name "*.ko.zst" -print0 | while IFS= read -r -d '' mod; do zstd -d --rm "$mod"; done
find root/lib/modules -name "*.ko.xz" -print0 | while IFS= read -r -d '' mod; do xz -d "$mod"; done
find root/lib/modules -name "*.ko.gz" -print0 | while IFS= read -r -d '' mod; do gzip -d "$mod"; done
depmod -b "$work/root" "$version"
# Keep ALSA configuration and package licensing metadata.
mkdir -p root/usr/share
cp -a /usr/share/alsa root/usr/share/
rm -rf root/boot root/usr/src
printf 'kernel=%s\n' "$version" > /image/kernel.txt
dpkg-query -W > /image/packages.txt
mkdir -p /image/licenses
cp -a /usr/share/doc/. /image/licenses/
(cd root; find . -print0 | cpio --null -o -H newc --quiet | gzip -1) > /image/root.cpio.gz
(cd /image; sha256sum Image root.cpio.gz > manifest.txt)
