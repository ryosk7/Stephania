#!/usr/bin/env bash
# Real BLE end-to-end on a Linux host with two virtual LE controllers:
#   hci0: t3box-ble (GATT server) -> t3box-provision (device core)
#   hci1: tests/ble/client.py (independent Python client via BlueZ/bleak)
# Requires root (vhci, bluetoothd, D-Bus). Used by CI; safe to run on a
# disposable VM. Usage: tests/ble/run-e2e.sh <target-dir> [--encrypted]
set -Eeuo pipefail
target=${1:?target dir with t3box-provision and t3box-ble}
encrypted=${2:-}
work=$(mktemp -d)
logs=${BLE_E2E_LOGS:-$work}
pids=()
cleanup() {
  for p in "${pids[@]}"; do sudo kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

sudo modprobe hci_vhci
sudo systemctl stop bluetooth 2>/dev/null || true
sudo pkill -x bluetoothd 2>/dev/null || true
sudo btvirt -L -l2 >"$logs/btvirt.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -e /sys/class/bluetooth/hci1 ] && break; sleep 0.1; done
test -e /sys/class/bluetooth/hci1 || { echo "virtual controllers missing"; cat "$logs/btvirt.log"; exit 1; }
sudo btmon -w "$logs/btmon.snoop" >/dev/null 2>&1 & pids+=($!)
sudo /usr/libexec/bluetooth/bluetoothd -n -d >"$logs/bluetoothd.log" 2>&1 & pids+=($!)
sleep 2
for i in 0 1; do sudo btmgmt --index "$i" power on >/dev/null; done
sudo btmgmt info | grep -E "^hci|addr" || true

# Browser-equivalent setup secret, delivered through the boot-partition file.
secret=$(python3 -c 'import secrets;print(secrets.token_hex(16))')
uri="t3box-setup:v1:${secret}:$(python3 -c "import hashlib,sys;print(hashlib.sha256(bytes.fromhex(sys.argv[1])).hexdigest()[:8])" "$secret")"
printf '%s\n' "$uri" >"$work/t3box-setup.txt"

T3BOX_PROVISION_SOCKET="$work/provisioning.sock" T3BOX_CREDENTIALS_DIR="$work" \
  T3BOX_SETUP_FILE="$work/t3box-setup.txt" T3BOX_NETWORK_BACKEND=virtual \
  "$target/t3box-provision" >"$logs/provision.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$work/provisioning.sock" ] && break; sleep 0.1; done
test ! -e "$work/t3box-setup.txt" || { echo "setup file was not consumed"; exit 1; }

enc=0; pair=()
if [ "$encrypted" = "--encrypted" ]; then enc=1; pair=(--pair); fi
sudo env T3BOX_BLE_DEBUG=1 T3BOX_PROVISION_SOCKET="$work/provisioning.sock" T3BOX_BLE_ADAPTER=hci0 T3BOX_BLE_REQUIRE_ENCRYPTION=$enc \
  "$target/t3box-ble" >"$logs/ble.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do grep -q advertising "$logs/ble.log" && break; sleep 0.2; done
cat "$logs/ble.log"

status=0
sudo timeout 180 python3 "$(dirname "$0")/client.py" "$uri" --adapter hci1 "${pair[@]}" || status=$?
if [ $status -ne 0 ]; then
  echo "---- t3box-ble"; cat "$logs/ble.log"
  echo "---- provision"; cat "$logs/provision.log"
  echo "---- bluetoothd (tail)"; sudo tail -80 "$logs/bluetoothd.log"
  echo "---- ATT (btmon)"; sudo btmon -r "$logs/btmon.snoop" 2>/dev/null | grep -E "ATT: (Handle Value Notification|Write Request|Exchange MTU)|Data(\[|:)|MTU" | head -120
fi
exit $status
