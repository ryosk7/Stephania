#!/usr/bin/env bash
# mmdebstrap customize hook (os/image/build.sh). Runs on the build host.
# Usage: customize.sh <rootdir> <repo> <bin-dir> <version> <build-packages> <runtime-packages>
set -euo pipefail
r=$1 repo=$2 bins=$3 version=$4 build=$5 runtime=$6
in_root() { chroot "$r" /usr/bin/env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin HOME=/root LANG=C.UTF-8 "$@"; }

echo "$version" >"$r/etc/t3box-version"
echo t3box >"$r/etc/hostname"

# Binaries and helpers.
install -m 0755 "$bins"/t3box "$bins"/t3box-phone "$bins"/t3box-provision "$bins"/t3box-pki "$bins"/t3box-bootctl "$r/usr/local/bin/"
install -m 0755 "$repo/os/rauc/t3box-update" "$r/usr/local/bin/t3box-update"
install -d "$r/usr/local/lib/t3box" "$r/usr/local/sbin"
install -m 0755 "$repo/os/rauc/rauc-pre-install" "$r/usr/local/lib/t3box/rauc-pre-install"
install -m 0755 "$repo/os/image/t3box-overlay-init" "$r/usr/local/sbin/t3box-overlay-init"
install -m 0755 "$repo/os/image/t3box-firstboot" "$repo/os/image/t3box-qemu-smoke" "$r/usr/local/lib/t3box/"

# Control plane with production gems only; native extensions are built
# here and the build toolchain is purged below.
install -d "$r/opt/t3box"
cp -a "$repo/services/control" "$r/opt/t3box/control"
rm -rf "$r/opt/t3box/control/test" "$r/opt/t3box/control/vendor" "$r/opt/t3box/control/.bundle"
if [ -d "$repo/apps/studio/dist" ]; then
  cp -a "$repo/apps/studio/dist" "$r/opt/t3box/studio"
fi
# T3BOX_IMAGE_SKIP_GEMS=1 only for local iteration without rubygems.org
# access; such an image cannot run the control plane.
[ "${T3BOX_IMAGE_SKIP_GEMS:-}" = 1 ] || in_root sh -c 'cd /opt/t3box/control && gem install --no-document bundler -v "$(tail -1 Gemfile.lock | tr -d " ")" >/dev/null &&
  bundle config set --local deployment true && bundle config set --local without test &&
  bundle install --jobs 4 --quiet'
# mmdebstrap marks every installed package manual; keep only the runtime
# set (and essential packages) so autoremove drops the whole toolchain.
in_root sh -c 'apt-mark showmanual | xargs -r apt-mark auto' >/dev/null
in_root apt-mark manual $(echo "$runtime" | tr , ' ') >/dev/null
in_root apt-get purge -y -qq $(echo "$build" | tr , ' ') >/dev/null
in_root apt-get -o APT::AutoRemove::RecommendsImportant=false -o APT::AutoRemove::SuggestsImportant=false autoremove -y -qq --purge >/dev/null
in_root apt-get clean
rm -rf "$r/var/lib/apt/lists/"* "$r/usr/share/doc/"* "$r/usr/share/man/"*
# No compilers on the device (§6).
for c in gcc cc g++ make; do
  if [ -e "$r/usr/bin/$c" ]; then echo "compiler $c left in image" >&2; exit 1; fi
done

# Users, units and configuration from the repository.
install -m 0644 "$repo/systemd/t3box.sysusers.conf" "$r/usr/lib/sysusers.d/t3box.conf"
install -m 0644 "$repo/systemd/t3box.tmpfiles.conf" "$r/usr/lib/tmpfiles.d/t3box.conf"
for u in "$repo"/systemd/*.service "$repo"/systemd/*.timer; do install -m 0644 "$u" "$r/etc/systemd/system/"; done
install -d "$r/etc/systemd/system.conf.d" "$r/etc/rauc" "$r/etc/dbus-1/system.d" "$r/etc/polkit-1/rules.d" "$r/etc/bluetooth"
install -m 0644 "$repo/os/systemd/10-t3box-watchdog.conf" "$r/etc/systemd/system.conf.d/"
install -m 0644 "$repo/os/rauc/system.conf" "$r/etc/rauc/system.conf"
install -m 0644 "$repo/systemd/t3box-ble.dbus.conf" "$r/etc/dbus-1/system.d/t3box-ble.conf"
install -m 0644 "$repo/systemd/t3box-provision.polkit.rules" "$r/etc/polkit-1/rules.d/50-t3box-provision.rules"
install -m 0644 "$repo/os/bluez/main.conf" "$r/etc/bluetooth/main.conf"
# Release keyring is provisioned by the release pipeline; the reference
# image ships an empty placeholder so RAUC refuses every bundle until then.
: >"$r/etc/rauc/keyring.pem"

# Read-only root: persistent state lives on the data partition.
cat >"$r/etc/fstab" <<'FSTAB'
PARTLABEL=data      /data          ext4  defaults,noatime  0 2
PARTLABEL=selector  /boot/selector vfat  defaults,umask=0077  0 2
/data/t3box/varlib  /var/lib/t3box none  bind,x-systemd.requires-mounts-for=/data  0 0
FSTAB
install -d "$r/data" "$r/boot/selector" "$r/var/lib/t3box" "$r/overlay" "$r/etc/t3box"
# The bootstrap token is generated on first boot into the data partition.
ln -sf /data/t3box/config/control.env "$r/etc/t3box/control.env"
install -m 0644 "$repo/os/image/t3box-firstboot.service" "$repo/os/image/t3box-qemu-smoke.service" "$r/etc/systemd/system/"
# Control code location used by t3box-control.service.
sed -i 's#^WorkingDirectory=.*#WorkingDirectory=/opt/t3box/control#' "$r/etc/systemd/system/t3box-control.service"
sed -i 's#^ExecStart=/usr/bin/bundle#ExecStart=/usr/local/bin/bundle#' "$r/etc/systemd/system/t3box-control.service"
[ -x "$r/usr/local/bin/bundle" ] || sed -i 's#^ExecStart=/usr/local/bin/bundle#ExecStart=/usr/bin/bundle#' "$r/etc/systemd/system/t3box-control.service"

in_root systemctl enable t3box-firstboot.service t3box.service t3box-control.service t3box-provision.service \
  t3box-phone.service t3box-pki.timer t3box-update-confirm.service t3box-data-restore.service t3box-update-deadline.timer \
  t3box-qemu-smoke.service
# Initramfs must mount a SquashFS root from virtio or SD.
printf 'squashfs\noverlay\nvirtio_blk\nvirtio_pci\n' >>"$r/etc/initramfs-tools/modules"
in_root update-initramfs -u -k all >/dev/null
