#!/usr/bin/env bash
# stephctl end to end (docs/MAINTENANCE.md): the real CLI against the real
# control plane (Puma, TLS with the device PKI), the real stephania-maint
# and the real OpenSSH server and client. Pins the CA by fingerprint, signs
# in with a Studio admin account, opens a window with a code, logs in by
# certificate, copies files with scp, and checks what must be refused.
# sshd runs as the current user on a free port; STEPHCTL_SSH_USER and
# STEPHCTL_SSH_PORT point the CLI at it instead of maint@<device>:22.
# stephania-maint refuses loopback and the device's own addresses as
# clients, so the "device" (control plane and sshd) listens on one LAN
# address and connections to it leave from another, as from a laptop:
# STEPHANIA_E2E_LAN="<device> <client>" names existing ones; otherwise
# 10.213.0.1 and 10.213.0.2 are added to lo, with a local route that makes
# the second the source for the first (root or passwordless sudo), and
# removed again on exit.
# Usage: ci/stephctl-e2e.sh <dir with stephctl, stephania-maint, stephania-pki>
set -euo pipefail
bins=$(cd "$1" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
sshd_bin=$(command -v sshd || echo /usr/sbin/sshd)
[ -x "$sshd_bin" ] || { echo "sshd not found" >&2; exit 1; }
w=$(mktemp -d)
me=$(id -un)
pids=()
pass=0
read -r dev cli <<<"${STEPHANIA_E2E_LAN:-10.213.0.1 10.213.0.2}"
lan_added=
asroot() { if [ "$(id -u)" = 0 ]; then "$@"; else sudo -n "$@"; fi; }
if [ -z "${STEPHANIA_E2E_LAN:-}" ]; then
  lan_added=1
  { asroot ip addr replace "$cli/32" dev lo && asroot ip addr replace "$dev/32" dev lo &&
    asroot ip route replace local "$dev" dev lo table local src "$cli"; } \
    || { echo "need root or sudo -n to add $dev and $cli to lo (or set STEPHANIA_E2E_LAN)" >&2; exit 1; }
fi
free_port() { python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'; }
port=$(free_port); loop=$(free_port); sshport=$(free_port)
fail() {
  echo "FAIL $*" >&2
  for f in "$w"/*.log; do echo "--- $f" >&2; tail -30 "$f" >&2; done
  exit 1
}
ok() { echo "PASS $*"; pass=$((pass + 1)); }
cleanup() {
  [ -f "$w/sshd.pid" ] && kill "$(cat "$w/sshd.pid")" 2>/dev/null
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  [ -n "$lan_added" ] && { asroot ip addr del "$dev/32" dev lo; asroot ip addr del "$cli/32" dev lo; } 2>/dev/null
  rm -rf "$w"
}
trap cleanup EXIT

# Device PKI, as stephania-pki issues it on the device.
export STEPHANIA_PKI_DIR=$w/pki
"$bins/stephania-pki" ensure --host stephania-ci --ip "$dev" --port "$port" >/dev/null || [ $? = 10 ] || fail "pki"
fp=$(openssl x509 -in "$w/pki/ca.pem" -outform der | sha256sum | cut -c1-32 | tr a-f A-F | sed 's/..../& /g; s/ $//')

# stephania-maint with sshd started through a stand-in for systemctl.
mkdir -p "$w/principals"
printf 'maint\n' >"$w/principals/$me"
cat >"$w/sshd_config" <<EOF
Port $sshport
ListenAddress $dev
PidFile $w/sshd.pid
HostKey $w/maint/ssh_host_ed25519_key
HostCertificate $w/maint/ssh_host_ed25519_key-cert.pub
TrustedUserCAKeys $w/maint/user_ca.pub
AuthorizedPrincipalsFile $w/principals/%u
AuthorizedKeysFile none
AuthenticationMethods publickey
PubkeyAcceptedAlgorithms ssh-ed25519-cert-v01@openssh.com
HostKeyAlgorithms ssh-ed25519-cert-v01@openssh.com,ssh-ed25519
PasswordAuthentication no
KbdInteractiveAuthentication no
AllowUsers $me
DisableForwarding yes
PermitUserRC no
MaxAuthTries 2
StrictModes no
UsePAM no
RevokedKeys $w/maint/revoked.krl
LogLevel VERBOSE
EOF
cat >"$w/systemctl" <<EOF
#!/bin/sh
case "\$1" in
  start) "$sshd_bin" -t -f "$w/sshd_config" && "$sshd_bin" -f "$w/sshd_config" -E "$w/sshd.log" ;;
  stop) [ -f "$w/sshd.pid" ] && kill "\$(cat "$w/sshd.pid")" 2>/dev/null; rm -f "$w/sshd.pid"; exit 0 ;;
esac
EOF
chmod +x "$w/systemctl"
STEPHANIA_MAINT_DIR="$w/maint" STEPHANIA_MAINT_SOCKET="$w/maint.sock" STEPHANIA_MAINT_SYSTEMCTL="$w/systemctl" \
  "$bins/stephania-maint" 2>"$w/maint.log" & pids+=($!)
for _ in $(seq 50); do [ -S "$w/maint.sock" ] && break; sleep 0.1; done
[ -S "$w/maint.sock" ] || fail "stephania-maint did not listen"

# The control plane: plain HTTP on loopback (bootstrap token) and TLS for the LAN.
bootstrap=ci-bootstrap-token-0123456789abcdef
(cd "$root/services/control" && STEPHANIA_CONTROL_PORT=$loop STEPHANIA_CONTROL_TLS_PORT=$port STEPHANIA_CONTROL_TLS_BIND=$dev \
  STEPHANIA_CONTROL_TOKEN=$bootstrap STEPHANIA_CONTROL_TOKENS_FILE="$w/tokens.json" STEPHANIA_MAINT_SOCKET="$w/maint.sock" \
  STEPHANIA_ENGINE_SOCKET="$w/none.sock" STEPHANIA_CONTROL_DB="$w/control.db" RACK_ENV=production \
  bundle exec puma -C config/puma.rb config.ru >"$w/puma.log" 2>&1) & pids+=($!)
for _ in $(seq 150); do curl -s -o /dev/null "http://127.0.0.1:$loop/" && break; sleep 0.2; done
user() {
  curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $bootstrap" -H 'Content-Type: application/json' \
    -d "{\"username\":\"$1\",\"password\":\"$2\",\"role\":\"$3\"}" "http://127.0.0.1:$loop/api/v1/users"
}
[ "$(user alice 'correct horse battery staple' admin)" = 201 ] || fail "admin account"
[ "$(user olive 'operators read only here' operator)" = 201 ] || fail "operator account"
[ "$(user bob 'a second admin account' admin)" = 201 ] || fail "second admin account"

# Maintenance code from the device's secret, like Connect computes it.
code() {
  python3 -I - "$w/maint/state.json" "${1:-0}" <<'PY'
import hashlib, hmac, json, struct, sys, time
secret = bytes.fromhex(json.load(open(sys.argv[1]))["otp_secret"])
mac = hmac.new(secret, struct.pack(">Q", int(time.time()) // 30 + int(sys.argv[2])), hashlib.sha1).digest()
o = mac[19] & 15
print("%06d" % ((struct.unpack(">I", mac[o:o + 4])[0] & 0x7fffffff) % 1000000))
PY
}
window_open() {
  curl -sS --cacert "$w/pki/ca.pem" -H "Authorization: Bearer $bootstrap" "http://127.0.0.1:$loop/api/v1/maintenance" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["open"])'
}

export STEPHCTL_CONFIG_DIR=$w/stephctl STEPHCTL_SESSION_STORE=file STEPHCTL_SSH_USER=$me STEPHCTL_SSH_PORT=$sshport
ctl() { "$bins/stephctl" "$@"; }

rc=0; echo 'correct horse battery staple' | ctl login "$dev:$port" --username alice --password-stdin --trust "0000 0000 0000 0000 0000 0000 0000 0000" 2>"$w/ctl.err" || rc=$?
[ "$rc" = 3 ] && [ ! -f "$w/stephctl/devices.toml" ] && ok "a fingerprint that does not match Connect is refused and nothing is pinned" || fail "wrong --trust: exit $rc $(cat "$w/ctl.err")"

echo 'correct horse battery staple' | ctl login "$dev:$port" --username alice --password-stdin --trust "$fp" 2>"$w/ctl.err" \
  || fail "login: $(cat "$w/ctl.err")"
grep -q "ca_sha256" "$w/stephctl/devices.toml" && ok "login pins the CA by fingerprint and signs in" || fail "devices.toml"
[ "$(stat -c %a "$w/stephctl/sessions/$dev.json")" = 600 ] && ! grep -q correct "$w/stephctl/devices.toml" \
  && ok "session kept private (0600), no secrets in devices.toml" || fail "session file"
ctl devices | grep -q "signed in as alice" && ok "devices lists the session" || fail "devices: $(ctl devices)"

[ "$(window_open)" = False ] || fail "window open before ssh"
out=$(STEPHCTL_CODE=$(code) ctl ssh -- echo maint-ok 2>"$w/ctl.err") || fail "ssh: $(cat "$w/ctl.err")"
[ "$out" = maint-ok ] && ok "ssh: code → window → fresh key and certificate → OpenSSH login with host-certificate check" || fail "ssh output: $out"
sleep 0.5
[ "$(window_open)" = False ] && [ ! -f "$w/sshd.pid" ] && ok "the window it opened is closed when ssh exits" || fail "window left open"
[ -z "$(ls "${TMPDIR:-/tmp}" | grep '^stephctl-' || true)" ] && ok "no key or certificate left on disk" || fail "scratch left behind"

used=$(code 1)
rc=0; STEPHCTL_CODE=$used ctl ssh --keep-window -- true 2>"$w/ctl.err" || rc=$?
[ "$rc" = 0 ] || fail "second ssh: $(cat "$w/ctl.err")"
[ "$(window_open)" = True ] && ok "--keep-window leaves the window open" || fail "keep-window"
printf 'evidence %s\n' "$RANDOM" >"$w/local.txt"
ctl cp "$w/local.txt" "device:$w/remote.txt" 2>"$w/ctl.err" && cmp -s "$w/local.txt" "$w/remote.txt" \
  && ok "cp to the device reuses the open window (scp)" || fail "cp up: $(cat "$w/ctl.err")"
ctl cp "device:$w/remote.txt" "$w/back.txt" 2>"$w/ctl.err" && cmp -s "$w/local.txt" "$w/back.txt" \
  && ok "cp from the device" || fail "cp down: $(cat "$w/ctl.err")"
ctl window | grep -q "Maintenance window" && ok "window shows the open window" || fail "window status"
echo 'a second admin account' | STEPHCTL_CONFIG_DIR=$w/bob ctl login "$dev:$port" --username bob --password-stdin --trust "$fp" 2>/dev/null || fail "bob login"
rc=0; STEPHCTL_CONFIG_DIR=$w/bob ctl ssh -- true 2>"$w/ctl.err" || rc=$?
[ "$rc" != 0 ] && grep -q "opened by alice" "$w/ctl.err" && ! grep -q '"actor":"bob"' "$w/maint.log" \
  && ok "another admin cannot use the window alice opened" || fail "bob used alice's window: exit $rc $(cat "$w/ctl.err")"
ctl window --close >/dev/null 2>&1 && [ "$(window_open)" = False ] && ok "window --close closes it" || fail "window --close"

rc=0; STEPHCTL_CODE=$used ctl ssh -- true 2>"$w/ctl.err" || rc=$?
[ "$rc" = 4 ] && grep -q "code not accepted" "$w/ctl.err" && ok "a used code does not open a window again (exit 4)" || fail "replay: exit $rc $(cat "$w/ctl.err")"
wrong=000000; [ "$(code)" = 000000 ] && wrong=000001
rc=0; STEPHCTL_CODE=$wrong ctl ssh -- true 2>"$w/ctl.err" || rc=$?
[ "$rc" = 4 ] && ok "a wrong code is refused (exit 4)" || fail "wrong code: exit $rc $(cat "$w/ctl.err")"

# An operator account cannot open maintenance.
rc=0; echo 'operators read only here' | STEPHCTL_CONFIG_DIR=$w/op ctl login "$dev:$port" --username olive --password-stdin --trust "$fp" 2>/dev/null || rc=$?
[ "$rc" = 0 ] || fail "operator login: $rc"
rc=0; STEPHCTL_CONFIG_DIR=$w/op STEPHCTL_CODE=$(code) ctl ssh -- true 2>"$w/ctl.err" || rc=$?
[ "$rc" = 5 ] && ok "an operator account cannot open maintenance (exit 5)" || fail "operator: exit $rc $(cat "$w/ctl.err")"

# A different CA (factory reset, or someone else on that address) is refused.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -keyout "$w/other.key" -out "$w/other.pem" -days 1 -subj /CN=other 2>/dev/null
python3 -I - "$w/stephctl/devices.toml" "$w/other.pem" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
other = open(sys.argv[2]).read()
text = re.sub(r'ca_pem = """.*?"""', lambda m: 'ca_pem = """\n' + other + '"""', text, flags=re.S)
open(sys.argv[1], "w").write(text)
PY
rc=0; ctl window 2>"$w/ctl.err" || rc=$?
[ "$rc" = 3 ] && grep -q "device certificate changed" "$w/ctl.err" && ok "a server that does not chain to the pinned CA is refused (exit 3)" || fail "pin: exit $rc $(cat "$w/ctl.err")"
ctl forget "$dev" 2>/dev/null && ! grep -q "$dev" "$w/stephctl/devices.toml" && [ ! -f "$w/stephctl/sessions/$dev.json" ] \
  && ok "forget removes the pin and the session" || fail "forget"

grep -q '"event":"maintenance.certificate_issued"' "$w/maint.log" && grep -q "\"actor\":\"alice\",\"client_addr\":\"$cli\"" "$w/maint.log" \
  && ok "the device audit names the Studio account and the client's address" || fail "audit"
echo "stephctl-e2e: $pass checks passed"
