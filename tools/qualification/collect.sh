#!/usr/bin/env bash
# Collects device evidence for docs/QUALIFICATION.md. Read-only: changes
# nothing on the device, never copies secrets (tokens, keys, credentials).
# Usage (on the device): collect.sh <out-dir>
set -uo pipefail
out=${1:?usage: collect.sh <out-dir>}
stamp=$(date -u +%Y%m%dT%H%M%SZ)
d="$out/stephania-evidence-$stamp"
mkdir -p "$d"
run() { local name=$1; shift; { echo "\$ $*"; "$@"; } >"$d/$name.txt" 2>&1 || true; }
run uname uname -a
run os-release cat /etc/os-release
run stephania-version cat /etc/stephania-version
run eeprom vcgencmd bootloader_version
run bootloader-config vcgencmd bootloader_config
run temp vcgencmd measure_temp
run throttled vcgencmd get_throttled
run tryboot sh -c 'for f in partition tryboot; do printf "%s=" $f; od -An -tu4 --endian=big /proc/device-tree/chosen/bootloader/$f 2>/dev/null || echo n/a; done'
run autoboot cat /boot/selector/autoboot.txt
run rauc rauc status --detailed
run lsusb lsusb -t
run v4l2 v4l2-ctl --list-devices
run alsa arecord -l
run bluetooth sh -c 'btmgmt info; bluetoothctl --version'
run network nmcli -f GENERAL,IP4 device show
run endpoint cat /var/lib/stephania/pki/endpoint.json
run units systemctl --no-pager status stephania stephania-control stephania-provision stephania-ble stephania-pki.timer stephania-update-confirm
run journal journalctl -b --no-pager -u stephania -u stephania-control -u stephania-provision -u stephania-ble -u bluetooth -u stephania-update-confirm -n 2000
# Never include: /var/lib/stephania/tokens, credentials, pki/*-key.pem, /etc/stephania/control.env.
tar -C "$out" -czf "$d.tar.gz" "$(basename "$d")" && rm -rf "$d"
sha256sum "$d.tar.gz"
