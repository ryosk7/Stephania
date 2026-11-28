#!/usr/bin/env bash
# Real Wi-Fi join through NetworkManager over D-Bus, on virtual radios:
#   wlan1: hostapd (WPA2-PSK) + dnsmasq DHCP
#   wlan2: hostapd, open, same SSID as wlan1 (an evil twin)
#   wlan3: hostapd (WPA3-Personal, SAE only) + dnsmasq DHCP
#   wlan0: managed by NetworkManager, used by stephania-provision (nm backend)
# Checks: scan lists the open twin separately and the device refuses to join
# it, a WPA3-only network is joined with key-mgmt sae, wrong-PSK failure
# leaves no profile, success stores the PSK system-owned, and
# NetworkManager reconnects on its own after a restart.
# Requires root on a disposable VM. Usage: tests/wifi/run-nm-e2e.sh <target-dir>
set -Eeuo pipefail
target=${1:?target dir with stephania-provision}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
logs=${NM_E2E_LOGS:-$work}
ssid=stephania-ci
psk=ci-passphrase-123
sae_ssid=stephania-ci-wpa3
sae_psk=ci-sae-passphrase-456
pids=()
cleanup() { for p in "${pids[@]}"; do sudo kill "$p" 2>/dev/null || true; done; sudo pkill -x hostapd 2>/dev/null || true; sudo pkill -x dnsmasq 2>/dev/null || true; }
trap cleanup EXIT

if ! sudo modprobe mac80211_hwsim radios=4 2>/dev/null; then
  sudo apt-get install -y -qq "linux-modules-extra-$(uname -r)" >/dev/null
  sudo modprobe mac80211_hwsim radios=4
fi
for _ in $(seq 50); do ip link show wlan3 >/dev/null 2>&1 && break; sleep 0.1; done
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
cat >"$work/hostapd-twin.conf" <<CONF
interface=wlan2
driver=nl80211
ssid=$ssid
hw_mode=g
channel=1
CONF
cat >"$work/hostapd-sae.conf" <<CONF
interface=wlan3
driver=nl80211
ssid=$sae_ssid
hw_mode=g
channel=11
wpa=2
wpa_key_mgmt=SAE
rsn_pairwise=CCMP
ieee80211w=2
sae_password=$sae_psk
CONF
sudo ip addr add 10.42.0.1/24 dev wlan1
sudo ip addr add 10.43.0.1/24 dev wlan3
for ap in "" -twin -sae; do
  sudo hostapd "$work/hostapd$ap.conf" >"$logs/hostapd$ap.log" 2>&1 & pids+=($!)
done
sudo dnsmasq --no-daemon --conf-file=/dev/null --interface=wlan1 --bind-interfaces --except-interface=lo \
  --port=0 --dhcp-range=10.42.0.10,10.42.0.50,12h --dhcp-leasefile="$work/leases" --log-dhcp >"$logs/dnsmasq.log" 2>&1 & pids+=($!)
sudo dnsmasq --no-daemon --conf-file=/dev/null --interface=wlan3 --bind-interfaces --except-interface=lo \
  --port=0 --dhcp-range=10.43.0.10,10.43.0.50,12h --dhcp-leasefile="$work/leases-sae" >"$logs/dnsmasq-sae.log" 2>&1 & pids+=($!)
for ap in "" -twin -sae; do
  for _ in $(seq 50); do grep -q "AP-ENABLED" "$logs/hostapd$ap.log" && break; sleep 0.2; done
  grep -q "AP-ENABLED" "$logs/hostapd$ap.log" || { cat "$logs/hostapd$ap.log"; exit 1; }
done

sudo env STEPHANIA_PROVISION_SOCKET="$work/p.sock" STEPHANIA_CREDENTIALS_DIR="$work" \
  STEPHANIA_NETWORK_BACKEND=nm STEPHANIA_WIFI_INTERFACE=wlan0 "$target/stephania-provision" >"$logs/provision.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do sudo test -S "$work/p.sock" && break; sleep 0.1; done

fail() { echo "FAIL $1"; cat "$logs/provision.log"; tail -40 "$logs/hostapd.log"; sudo journalctl -u NetworkManager --no-pager | tail -60; exit 1; }
sudo timeout 240 python3 "$here/socket_client.py" "$work/p.sock" "$ssid" "$psk" "$sae_ssid" "$sae_psk" || fail "socket client"

hexname() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
sae_profile="stephania-$(hexname "$sae_ssid")"
[ "$(nmcli -g 802-11-wireless-security.key-mgmt connection show "$sae_profile")" = sae ] || fail "WPA3-only profile must use key-mgmt sae"
echo "PASS WPA3-only network joined with key-mgmt sae"
# Leave only the WPA2 network for the reconnect checks below.
sudo nmcli connection delete "$sae_profile" >/dev/null
if nmcli -t -f NAME connection show | grep -q -- "-open$"; then fail "an open profile was created for the evil twin"; fi
echo "PASS no profile for the open twin"

profile="stephania-$(hexname "$ssid")"
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
