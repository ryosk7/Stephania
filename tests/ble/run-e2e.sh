#!/usr/bin/env bash
# Real BLE end-to-end (protocol v2) on a Linux host with two virtual LE
# controllers:
#   hci0: t3box-ble (GATT server, supervised like systemd Restart=always)
#         -> t3box-provision (device core)
#   hci1: tests/ble/client.py (independent Python client via BlueZ/bleak)
# Phases: claim/scan/join, then bluetoothd is restarted and the client must
# find the same, still-owned device again (self-healing adapter).
# Requires root (vhci, bluetoothd, D-Bus). Usage:
#   tests/ble/run-e2e.sh <target-dir> [--min-mtu]
set -Eeuo pipefail
target=${1:?target dir with t3box-provision and t3box-ble}
mode=${2:-}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
logs=${BLE_E2E_LOGS:-$work}
pids=()
cleanup() {
  touch "$work/stop"
  for p in "${pids[@]}"; do sudo kill "$p" 2>/dev/null || true; done
  sudo pkill -x t3box-ble 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

# Hosted runner kernels differ; some ship hci_vhci only in modules-extra.
if ! sudo modprobe hci_vhci 2>/dev/null; then
  sudo apt-get install -y -qq "linux-modules-extra-$(uname -r)" >/dev/null
  sudo modprobe hci_vhci
fi

start_bluetoothd() {
  sudo /usr/libexec/bluetooth/bluetoothd -n -d >>"$logs/bluetoothd.log" 2>&1 &
  echo $! >"$work/bluetoothd.pid"
  for _ in $(seq 50); do sudo btmgmt --index 0 info >/dev/null 2>&1 && break; sleep 0.1; done
  for i in 0 1; do sudo btmgmt --index "$i" power on >/dev/null; done
}

sudo systemctl stop bluetooth 2>/dev/null || true
sudo pkill -x bluetoothd 2>/dev/null || true
sudo btvirt -L -l2 >"$logs/btvirt.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -e /sys/class/bluetooth/hci1 ] && break; sleep 0.1; done
test -e /sys/class/bluetooth/hci1 || { echo "virtual controllers missing"; cat "$logs/btvirt.log"; exit 1; }
sudo btmon -w "$logs/btmon.snoop" >/dev/null 2>&1 & pids+=($!)
start_bluetoothd

# Browser-equivalent setup secret, delivered through the boot-partition file.
secret=$(python3 -c 'import secrets;print(secrets.token_hex(16))')
uri="t3box-setup:v1:${secret}:$(python3 -c "import hashlib,sys;print(hashlib.sha256(bytes.fromhex(sys.argv[1])).hexdigest()[:8])" "$secret")"
printf '%s\n' "$uri" >"$work/t3box-setup.txt"

T3BOX_PROVISION_SOCKET="$work/provisioning.sock" T3BOX_CREDENTIALS_DIR="$work" \
  T3BOX_SETUP_FILE="$work/t3box-setup.txt" T3BOX_NETWORK_BACKEND=virtual \
  "$target/t3box-provision" >"$logs/provision.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$work/provisioning.sock" ] && break; sleep 0.1; done
test ! -e "$work/t3box-setup.txt" || { echo "setup file was not consumed"; exit 1; }

# Supervisor equivalent to systemd Restart=always RestartSec=1.
(
  while [ ! -e "$work/stop" ]; do
    sudo env T3BOX_PROVISION_SOCKET="$work/provisioning.sock" T3BOX_BLE_ADAPTER=hci0 \
      "$target/t3box-ble" >>"$logs/ble.log" 2>&1 || true
    echo "t3box-ble exited; restarting" >>"$logs/ble.log"
    sleep 1
  done
) & pids+=($!)
wait_adverts() {
  for _ in $(seq 100); do [ "$(grep -c advertising "$logs/ble.log" 2>/dev/null || true)" -ge "$1" ] && return 0; sleep 0.2; done
  return 1
}
wait_adverts 1

dump() {
  echo "---- t3box-ble"; cat "$logs/ble.log"
  echo "---- provision"; cat "$logs/provision.log"
  echo "---- bluetoothd (tail)"; sudo tail -60 "$logs/bluetoothd.log"
  echo "---- ATT (btmon)"; sudo btmon -r "$logs/btmon.snoop" 2>/dev/null | grep -E "ATT: |Data(\[|:)" | tail -80
}
run_client() {
  sudo timeout 180 python3 "$here/client.py" "$uri" "$1" --adapter hci1 ${mode:+$mode} || { dump; exit 1; }
}

run_client claim

echo "== restarting bluetoothd (adapter must self-heal)"
sudo kill "$(cat "$work/bluetoothd.pid")"; sleep 1
start_bluetoothd
wait_adverts 2 || { echo "t3box-ble did not re-register after bluetoothd restart"; dump; exit 1; }
grep -q "exited; restarting" "$logs/ble.log" && echo "PASS t3box-ble exited and was restarted by its supervisor"

run_client resume
echo "BLE end-to-end ${mode:-default}: passed"
