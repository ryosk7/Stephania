#!/usr/bin/env bash
# End-to-end test of maintenance SSH (docs/MAINTENANCE.md) with the real
# OpenSSH server and client: stephania-maint opens a window for a code,
# signs a certificate for a fresh key, starts sshd through a stand-in for
# systemctl, and the client logs in by certificate with host-certificate
# checking. Also checks what must fail. Runs unprivileged: sshd runs as the
# current user and maps its principals file to that user.
# Usage: ci/maint-ssh-e2e.sh <stephania-maint binary>
set -euo pipefail
bin=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
sshd_bin=$(command -v sshd || echo /usr/sbin/sshd)
[ -x "$sshd_bin" ] || { echo "sshd not found" >&2; exit 1; }
w=$(mktemp -d)
port=$((20000 + RANDOM % 20000))
me=$(id -un)
pass=0
fail() { echo "FAIL $*" >&2; [ -f "$w/sshd.log" ] && tail -20 "$w/sshd.log" >&2; exit 1; }
ok() { echo "PASS $*"; pass=$((pass + 1)); }
cleanup() { [ -f "$w/sshd.pid" ] && kill "$(cat "$w/sshd.pid")" 2>/dev/null; [ -n "${maint_pid:-}" ] && kill "$maint_pid" 2>/dev/null; rm -rf "$w"; }
trap cleanup EXIT

mkdir -p "$w/pki" "$w/principals"
printf '{"host":"stephania-e2e.local","port":9443,"ips":["127.0.0.1"],"urls":[],"ca_sha256":"%064d","not_after_unix":4102444800}\n' 0 >"$w/pki/endpoint.json"
# The device maps maint → "maint" and root → "maint-root"; here the one
# local user stands in for whichever the test names.
principal() { printf '%s\n' "$1" >"$w/principals/$me"; }
principal maint

# sshd with the device config's authentication settings, on a free port.
cat >"$w/sshd_config" <<EOF
Port $port
ListenAddress 127.0.0.1
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

STEPHANIA_MAINT_DIR="$w/maint" STEPHANIA_MAINT_SOCKET="$w/maint.sock" STEPHANIA_PKI_DIR="$w/pki" \
  STEPHANIA_MAINT_SYSTEMCTL="$w/systemctl" "$bin" 2>"$w/maint.log" &
maint_pid=$!
for _ in $(seq 50); do [ -S "$w/maint.sock" ] && break; sleep 0.1; done
[ -S "$w/maint.sock" ] || fail "stephania-maint did not listen: $(cat "$w/maint.log")"

# rpc <method> <json params> → result JSON (or the error object) on stdout.
rpc() {
  python3 -I - "$w/maint.sock" "$1" "$2" <<'PY'
import json, socket, struct, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
body = json.dumps({"version": 1, "request_id": "e2e", "method": sys.argv[2], "params": json.loads(sys.argv[3])}).encode()
s.sendall(struct.pack(">I", len(body)) + body)
n = struct.unpack(">I", s.recv(4, socket.MSG_WAITALL))[0]
r = json.loads(s.recv(n, socket.MSG_WAITALL))
print(json.dumps(r["result"] if r["ok"] else {"error": r["error"]["code"]}))
PY
}
# TOTP code from the device's secret, at an offset in 30 s steps.
code() {
  python3 -I - "$w/maint/state.json" "${1:-0}" <<'PY'
import hashlib, hmac, json, struct, sys, time
secret = bytes.fromhex(json.load(open(sys.argv[1]))["otp_secret"])
mac = hmac.new(secret, struct.pack(">Q", int(time.time()) // 30 + int(sys.argv[2])), hashlib.sha1).digest()
o = mac[19] & 15
print("%06d" % ((struct.unpack(">I", mac[o:o + 4])[0] & 0x7fffffff) % 1000000))
PY
}
field() { python3 -I -c 'import json,sys; v=json.loads(sys.argv[1]).get(sys.argv[2]); print("" if v is None else v)' "$1" "$2"; }

# login <window_id> <client_addr> → runs `echo maint-ok` over ssh with a fresh key and its certificate.
login() {
  rm -f "$w/id" "$w/id.pub" "$w/id-cert.pub"
  ssh-keygen -q -t ed25519 -N '' -f "$w/id"
  r=$(rpc cert.sign "{\"window_id\":\"$1\",\"public_key\":\"$(cat "$w/id.pub")\",\"client_addr\":\"$2\"}")
  [ -n "$(field "$r" certificate)" ] || { echo "sign: $r" >&2; return 2; }
  field "$r" certificate >"$w/id-cert.pub"
  echo "@cert-authority stephania-e2e.local,127.0.0.1 $(field "$r" host_ca)" >"$w/known_hosts"
  ssh -F none -p "$port" -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none \
    -o IdentityFile="$w/id" -o CertificateFile="$w/id-cert.pub" \
    -o UserKnownHostsFile="$w/known_hosts" -o GlobalKnownHostsFile=/dev/null \
    -o StrictHostKeyChecking=yes -o HostKeyAlias=stephania-e2e.local -o ConnectTimeout=5 \
    "$me@127.0.0.1" echo maint-ok 2>>"$w/ssh.err"
}

[ "$(field "$(rpc status '{}')" open)" = False ] && ok "no window and no sshd at start" || fail "status at start"
[ ! -f "$w/sshd.pid" ] || fail "sshd running without a window"

r=$(rpc window.open '{"actor":"alice","client_addr":"127.0.0.1","code":"000000"}')
[ "$(field "$r" error)" = invalid_code ] || [ "$(code)" = 000000 ] && ok "a wrong code is refused" || fail "wrong code: $r"

c=$(code)
r=$(rpc window.open "{\"actor\":\"alice\",\"client_addr\":\"127.0.0.1\",\"code\":\"$c\"}")
win=$(field "$r" window_id)
[ -n "$win" ] && ok "a correct code opens a window" || fail "open: $r"
for _ in $(seq 50); do [ -f "$w/sshd.pid" ] && break; sleep 0.1; done
[ -f "$w/sshd.pid" ] && ok "sshd started for the window" || fail "sshd not started: $(cat "$w/maint.log")"

[ "$(login "$win" 127.0.0.1)" = maint-ok ] && ok "login with a fresh key and a 5-minute certificate; host certificate checked" || fail "login: $(cat "$w/ssh.err")"

r=$(rpc window.open "{\"actor\":\"bob\",\"client_addr\":\"127.0.0.1\",\"code\":\"$(code 1)\"}")
[ "$(field "$r" error)" = window_open ] && ok "only one window at a time" || fail "second window: $r"

if login "$win" 10.0.0.9 >/dev/null; then fail "certificate for another address logged in"; else ok "certificate bound to another client address is refused"; fi

principal maint-root
if login "$win" 127.0.0.1 >/dev/null; then fail "maint certificate accepted for a maint-root user"; else ok "a maint certificate cannot log in where only maint-root is allowed"; fi
principal maint

r=$(rpc cert.sign "{\"window_id\":\"$win\",\"public_key\":\"ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQ x\",\"client_addr\":\"127.0.0.1\"}")
[ "$(field "$r" error)" = invalid_params ] && ok "non-Ed25519 keys are refused" || fail "rsa: $r"

r=$(rpc window.close "{\"window_id\":\"$win\",\"actor\":\"alice\"}")
[ "$(field "$r" closed)" = True ] && ok "window closed" || fail "close: $r"
sleep 0.5
[ ! -f "$w/sshd.pid" ] && ok "sshd stopped with the window" || fail "sshd still running"
r=$(rpc cert.sign "{\"window_id\":\"$win\",\"public_key\":\"$(cat "$w/id.pub")\",\"client_addr\":\"127.0.0.1\"}")
[ "$(field "$r" error)" = window_closed ] && ok "a closed window signs nothing" || fail "sign after close: $r"

r=$(rpc window.open "{\"actor\":\"alice\",\"client_addr\":\"127.0.0.1\",\"code\":\"$c\"}")
[ "$(field "$r" error)" = invalid_code ] && ok "a used code does not work twice" || fail "replay: $r"

r=$(rpc window.open "{\"actor\":\"alice\",\"client_addr\":\"127.0.0.1\",\"code\":\"$(code 1)\",\"root\":true}")
win=$(field "$r" window_id)
[ "$(field "$r" user)" = root ] && ok "a root window is opened explicitly" || fail "root window: $r"
principal maint-root
[ "$(login "$win" 127.0.0.1)" = maint-ok ] && ok "root window certificate logs in where maint-root is allowed" || fail "root login: $(cat "$w/ssh.err")"
rpc window.close "{\"actor\":\"alice\"}" >/dev/null

# A new Connect enrollment rotates the code secret (otp.rotate, called by
# stephania-provision): old codes stop working, new ones work at once, and
# an open window is closed.
secret() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["otp_secret"])' "$w/maint/state.json"; }
old=$(secret)
r=$(rpc otp.rotate '{}')
new=$(field "$r" otp_secret)
[ -n "$new" ] && [ "$new" != "$old" ] && [ "$new" = "$(secret)" ] && ok "otp.rotate returns a new secret and stores it" || fail "rotate: $r"
r=$(rpc window.open "{\"actor\":\"alice\",\"client_addr\":\"::ffff:127.0.0.1\",\"code\":\"$(code)\"}")
win=$(field "$r" window_id)
[ -n "$win" ] && ok "a code from the new secret opens a window (IPv4-mapped client address)" || fail "open after rotate: $r"
for _ in $(seq 50); do [ -f "$w/sshd.pid" ] && break; sleep 0.1; done
rpc otp.rotate '{}' >/dev/null
sleep 0.5
[ "$(field "$(rpc status '{}')" open)" = False ] && [ ! -f "$w/sshd.pid" ] && ok "rotating the secret closes the open window and stops sshd" || fail "window survived rotation"

grep -q '"event":"maintenance.root_certificate_issued"' "$w/maint.log" && ok "audit events in the journal" || fail "audit: $(cat "$w/maint.log")"
echo "maint-ssh-e2e: $pass checks passed"
