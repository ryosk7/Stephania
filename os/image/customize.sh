#!/usr/bin/env bash
# mmdebstrap customize hook (os/image/build.sh). Runs on the build host.
# Usage: customize.sh <rootdir> <repo> <bin-dir> <version> <build-packages> <runtime-packages>
set -euo pipefail
r=$1 repo=$2 bins=$3 version=$4 build=$5 runtime=$6
in_root() { chroot "$r" /usr/bin/env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin HOME=/root LANG=C.UTF-8 "$@"; }

echo "$version" >"$r/etc/stephania-version"
echo stephania >"$r/etc/hostname"

# Binaries and helpers.
install -m 0755 "$bins"/stephania "$bins"/stephania-phone "$bins"/stephania-provision "$bins"/stephania-pki "$bins"/stephania-bootctl "$bins"/stephania-maint "$bins"/steph "$r/usr/local/bin/"
install -m 0755 "$repo/os/rauc/stephania-update" "$r/usr/local/bin/stephania-update"
install -d "$r/usr/local/lib/stephania" "$r/usr/local/sbin"
install -m 0755 "$repo/os/rauc/rauc-pre-install" "$r/usr/local/lib/stephania/rauc-pre-install"
install -m 0755 "$repo/os/image/stephania-overlay-init" "$r/usr/local/sbin/stephania-overlay-init"
install -m 0755 "$repo/os/image/stephania-firstboot" "$repo/os/image/stephania-qemu-smoke" "$repo/os/image/stephania-hostname" "$r/usr/local/lib/stephania/"

# Control plane with production gems only; native extensions are built
# here and the build toolchain is purged below.
install -d "$r/opt/stephania"
cp -a "$repo/services/control" "$r/opt/stephania/control"
rm -rf "$r/opt/stephania/control/test" "$r/opt/stephania/control/vendor" "$r/opt/stephania/control/.bundle"
if [ -d "$repo/apps/studio/dist" ]; then
  cp -a "$repo/apps/studio/dist" "$r/opt/stephania/studio"
fi
# STEPHANIA_IMAGE_SKIP_GEMS=1 only for local iteration without rubygems.org
# access; such an image cannot run the control plane.
[ "${STEPHANIA_IMAGE_SKIP_GEMS:-}" = 1 ] || in_root sh -c 'cd /opt/stephania/control && gem install --no-document bundler -v "$(tail -1 Gemfile.lock | tr -d " ")" >/dev/null &&
  bundle config set --local deployment true && bundle config set --local without test &&
  bundle install --jobs 4 --quiet'
# mmdebstrap marks every installed package manual; keep only the runtime
# set (and essential packages) so autoremove drops the whole toolchain.
in_root sh -c 'apt-mark showmanual | xargs -r apt-mark auto' >/dev/null
in_root apt-mark manual $(echo "$runtime" | tr , ' ') >/dev/null
in_root apt-get purge -y -qq $(echo "$build" | tr , ' ') >/dev/null
in_root apt-get -o APT::AutoRemove::RecommendsImportant=false -o APT::AutoRemove::SuggestsImportant=false autoremove -y -qq --purge >/dev/null
in_root apt-get clean
rm -rf "$r/var/lib/apt/lists/"* "$r/usr/share/man/"*
# Documentation goes, licences stay: /usr/share/doc/*/copyright must ship
# with the GPL/LGPL components (docs/LICENSING.md).
find "$r/usr/share/doc" -mindepth 1 -type f ! -name copyright -delete
find "$r/usr/share/doc" -mindepth 1 -type d -empty -delete
# No compilers on the device (§6).
for c in gcc cc g++ make; do
  if [ -e "$r/usr/bin/$c" ]; then echo "compiler $c left in image" >&2; exit 1; fi
done

# Users, units and configuration from the repository.
install -m 0644 "$repo/systemd/stephania.sysusers.conf" "$r/usr/lib/sysusers.d/stephania.conf"
install -m 0644 "$repo/systemd/stephania.tmpfiles.conf" "$r/usr/lib/tmpfiles.d/stephania.conf"
for u in "$repo"/systemd/*.service "$repo"/systemd/*.timer; do install -m 0644 "$u" "$r/etc/systemd/system/"; done
install -d "$r/etc/systemd/system.conf.d" "$r/etc/rauc" "$r/etc/dbus-1/system.d" "$r/etc/polkit-1/rules.d" "$r/etc/bluetooth"
install -m 0644 "$repo/os/systemd/10-stephania-watchdog.conf" "$r/etc/systemd/system.conf.d/"
install -m 0644 "$repo/os/rauc/system.conf" "$r/etc/rauc/system.conf"
install -m 0644 "$repo/systemd/stephania-ble.dbus.conf" "$r/etc/dbus-1/system.d/stephania-ble.conf"
install -m 0644 "$repo/systemd/stephania-provision.polkit.rules" "$r/etc/polkit-1/rules.d/50-stephania-provision.rules"
install -m 0644 "$repo/systemd/stephania-maint.polkit.rules" "$r/etc/polkit-1/rules.d/50-stephania-maint.rules"
install -m 0644 "$repo/systemd/stephania-maint-rauc.dbus.conf" "$r/etc/dbus-1/system.d/stephania-maint-rauc.conf"
# Maintenance SSH (docs/MAINTENANCE.md): only stephania-sshd.service, run by
# stephania-maint for a window. Debian's own ssh units stay masked and the
# package's host keys are dropped (the device's host key is on data).
install -D -m 0644 "$repo/os/ssh/sshd_config" "$r/etc/stephania/sshd_config"
install -d -m 0755 "$r/etc/stephania/ssh_principals"
install -m 0644 "$repo/os/ssh/principals/maint" "$repo/os/ssh/principals/root" "$r/etc/stephania/ssh_principals/"
rm -f "$r"/etc/ssh/ssh_host_*
in_root systemctl mask ssh.service ssh.socket >/dev/null 2>&1
install -m 0644 "$repo/os/nftables/stephania.nft" "$r/etc/nftables.conf"
install -m 0644 "$repo/os/bluez/main.conf" "$r/etc/bluetooth/main.conf"
# Release keyring is provisioned by the release pipeline; the reference
# image ships an empty placeholder so RAUC refuses every bundle until then.
: >"$r/etc/rauc/keyring.pem"

# Read-only root: persistent state lives on the data partition.
cat >"$r/etc/fstab" <<'FSTAB'
PARTLABEL=data      /data          ext4  defaults,noatime  0 2
PARTLABEL=selector  /boot/selector vfat  defaults,uid=0,gid=853,fmask=0117,dmask=0007  0 2
/data/stephania/varlib  /var/lib/stephania none  bind,x-systemd.requires-mounts-for=/data  0 0
FSTAB
install -d "$r/data" "$r/boot/selector" "$r/var/lib/stephania" "$r/overlay" "$r/etc/stephania"
# The bootstrap token is generated on first boot into the data partition.
ln -sf /data/stephania/config/control.env "$r/etc/stephania/control.env"
install -m 0644 "$repo/os/image/stephania-firstboot.service" "$repo/os/image/stephania-qemu-smoke.service" "$repo/os/image/stephania-hostname.service" "$r/etc/systemd/system/"
install -D -m 0644 "$repo/os/avahi/stephania.service" "$r/etc/avahi/services/stephania.service"
# Control code location used by stephania-control.service.
sed -i 's#^WorkingDirectory=.*#WorkingDirectory=/opt/stephania/control#' "$r/etc/systemd/system/stephania-control.service"
sed -i 's#^ExecStart=/usr/bin/bundle#ExecStart=/usr/local/bin/bundle#' "$r/etc/systemd/system/stephania-control.service"
[ -x "$r/usr/local/bin/bundle" ] || sed -i 's#^ExecStart=/usr/local/bin/bundle#ExecStart=/usr/bin/bundle#' "$r/etc/systemd/system/stephania-control.service"

in_root systemctl enable stephania-hostname.service avahi-daemon.service stephania-firstboot.service stephania.service stephania-control.service stephania-provision.service \
  stephania-phone.service stephania-pki.timer stephania-update-confirm.service stephania-data-restore.service stephania-update-deadline.timer \
  stephania-maint.service nftables.service \
  stephania-qemu-smoke.service
# Initramfs must mount a SquashFS root from virtio or SD.
printf 'squashfs\noverlay\nvirtio_blk\nvirtio_pci\n' >>"$r/etc/initramfs-tools/modules"
in_root update-initramfs -u -k all >/dev/null
