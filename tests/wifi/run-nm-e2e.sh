#!/usr/bin/env bash
# Real Wi-Fi join through NetworkManager over D-Bus, on virtual radios:
#   wlan1: hostapd (WPA2-PSK) + dnsmasq DHCP
#   wlan0: managed by NetworkManager, used by stephania-provision (nm backend)
# Checks: scan, wrong-PSK failure leaves no profile, success stores the PSK
# system-owned, and NetworkManager reconnects on its own after a restart.
# Requires root on a disposable VM. Usage: tests/wifi/run-nm-e2e.sh <target-dir>
set -Eeuo pipefail
target=${1:?target dir with stephania-provision}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
logs=${NM_E2E_LOGS:-$work}
ssid=stephania-ci
psk=ci-passphrase-123
pids=()
cleanup() { for p in "${pids[@]}"; do sudo kill "$p" 2>/dev/null || true; done; sudo pkill -x hostapd 2>/dev/null || true; sudo pkill -x dnsmasq 2>/dev/null || true; }
trap cleanup EXIT

if ! sudo modprobe mac80211_hwsim radios=2 2>/dev/null; then
  sudo apt-get install -y -qq "linux-modules-extra-$(uname -r)" >/dev/null
  sudo modprobe mac80211_hwsim radios=2
fi
for _ in $(seq 50); do ip link show wlan1 >/dev/null 2>&1 && break; sleep 0.1; done
ip -br link | grep wlan

# NetworkManager may only touch wlan0 (never the runner's uplink).
sudo mkdir -p /etc/NetworkManager/conf.d
printf '[keyfile]\nunmanaged-devices=*,except:interface-name:wlan0\n[device]\nwifi.scan-rand-mac-address=no\n' |
  sudo tee /etc/NetworkManager/conf.d/99-stephania-ci.conf >/dev/null
sudo systemctl restart NetworkManager
for _ in $(seq 50); do nmcli -t -f DEVICE,STATE dev | grep -q '^wlan0:disconnected' && break; sleep 0.2; done
nmcli dev status

cat >"$work/hostapd.conf" <<CONF
interface=wlan1
driver=nl80211
ssid=$ssid
hw_mode=g
channel=6
wpa=2
wpa_key_mgmt=WPA-PSK
rsn_pairwise=CCMP
wpa_passphrase=$psk
CONF
sudo ip addr add 10.42.0.1/24 dev wlan1
sudo hostapd "$work/hostapd.conf" >"$logs/hostapd.log" 2>&1 & pids+=($!)
sudo dnsmasq --no-daemon --conf-file=/dev/null --interface=wlan1 --bind-interfaces --except-interface=lo \
  --port=0 --dhcp-range=10.42.0.10,10.42.0.50,12h --dhcp-leasefile="$work/leases" --log-dhcp >"$logs/dnsmasq.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do grep -q "AP-ENABLED" "$logs/hostapd.log" && break; sleep 0.2; done
grep -q "AP-ENABLED" "$logs/hostapd.log" || { cat "$logs/hostapd.log"; exit 1; }

sudo env STEPHANIA_PROVISION_SOCKET="$work/p.sock" STEPHANIA_CREDENTIALS_DIR="$work" \
  STEPHANIA_NETWORK_BACKEND=nm STEPHANIA_WIFI_INTERFACE=wlan0 "$target/stephania-provision" >"$logs/provision.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do sudo test -S "$work/p.sock" && break; sleep 0.1; done

fail() { echo "FAIL $1"; cat "$logs/provision.log"; tail -40 "$logs/hostapd.log"; sudo journalctl -u NetworkManager --no-pager | tail -60; exit 1; }
sudo timeout 240 python3 "$here/socket_client.py" "$work/p.sock" "$ssid" "$psk" || fail "socket client"

profile="stephania-$(printf '%s' "$ssid" | od -An -tx1 | tr -d ' \n')"
count=$(nmcli -t -f NAME connection show | grep -cx "$profile" || true)
[ "$count" = 1 ] || fail "expected exactly one profile $profile, got $count (failed attempt must not leave one)"
echo "PASS exactly one profile after a failed and a successful join"
stored=$(sudo nmcli -s -g 802-11-wireless-security.psk connection show "$profile")
[ "$stored" = "$psk" ] || fail "PSK not stored system-owned"
echo "PASS PSK stored in NetworkManager's system profile"
[ "$(nmcli -g connection.autoconnect connection show "$profile")" = yes ] || fail "autoconnect off"
ip -4 -br addr show wlan0 | grep -q '10\.42\.0\.' || fail "no DHCP lease on wlan0"
echo "PASS wlan0 has an address from the AP"

sudo systemctl restart NetworkManager
for _ in $(seq 120); do nmcli -t -f DEVICE,STATE dev | grep -q '^wlan0:connected$' && break; sleep 0.5; done
nmcli -t -f DEVICE,STATE dev | grep -q '^wlan0:connected$' || fail "no automatic reconnection after NetworkManager restart"
echo "PASS reconnected automatically after NetworkManager restart (PSK persisted)"
echo "NetworkManager end-to-end: passed"
