#!/usr/bin/env bash
# End-to-end test of maintenance SSH (docs/MAINTENANCE.md) with the real
# OpenSSH server and client: stephania-maint opens a window for a code,
# signs a certificate for a fresh key, starts sshd through a stand-in for
# systemctl, and the client logs in by certificate with host-certificate
# checking. Also checks what must fail. Runs unprivileged: sshd runs as the
# current user and maps its principals file to that user.
# With steph: the broker socket (ops.sock) for its changes, run inside a
# real session so the actor comes from the session certificate.
# Usage: ci/maint-ssh-e2e.sh <stephania-maint binary> [<steph binary>]
set -euo pipefail
bin=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
steph=${2:+$(cd "$(dirname "${2:-.}")" && pwd)/$(basename "${2:-.}")}
here=$(cd "$(dirname "$0")" && pwd)
sshd_bin=$(command -v sshd || echo /usr/sbin/sshd)
[ -x "$sshd_bin" ] || { echo "sshd not found" >&2; exit 1; }
w=$(mktemp -d)
port=$((20000 + RANDOM % 20000))
me=$(id -un)
pass=0
fail() { echo "FAIL $*" >&2; [ -f "$w/sshd.log" ] && tail -20 "$w/sshd.log" >&2; tail -5 "$w/maint.log" >&2; exit 1; }
ok() { echo "PASS $*"; pass=$((pass + 1)); }
cleanup() { [ -f "$w/sshd.pid" ] && kill "$(cat "$w/sshd.pid")" 2>/dev/null; [ -n "${maint_pid:-}" ] && kill "$maint_pid" 2>/dev/null; [ -n "${engine_pid:-}" ] && kill "$engine_pid" 2>/dev/null; rm -rf "$w"; }
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
ExposeAuthInfo yes
LogLevel VERBOSE
EOF
cat >"$w/systemctl" <<EOF
#!/bin/sh
case "\$1" in
  start) "$sshd_bin" -t -f "$w/sshd_config" && "$sshd_bin" -f "$w/sshd_config" -E "$w/sshd.log" ;;
  stop) [ -f "$w/sshd.pid" ] && kill "\$(cat "$w/sshd.pid")" 2>/dev/null; rm -f "$w/sshd.pid"; exit 0 ;;
  *) echo "\$*" >>"$w/systemctl.log" ;;
esac
EOF
chmod +x "$w/systemctl"
mkdir -p "$w/inbox"
cat >"$w/update" <<EOF
#!/bin/sh
echo "\$1" >>"$w/update.log"
grep -q good "\$1" || { echo "bundle signature not valid" >&2; exit 1; }
EOF
chmod +x "$w/update"
engine() {
  if [ -n "${engine_pid:-}" ]; then kill "$engine_pid" 2>/dev/null; wait "$engine_pid" 2>/dev/null || true; fi
  rm -f "$w/engine.sock"
  python3 -I "$here/fake_engine.py" "$w/engine.sock" "$1" & engine_pid=$!
  for _ in $(seq 30); do [ -S "$w/engine.sock" ] && break; sleep 0.1; done
}
engine idle

STEPHANIA_MAINT_DIR="$w/maint" STEPHANIA_MAINT_SOCKET="$w/maint.sock" STEPHANIA_PKI_DIR="$w/pki" \
  STEPHANIA_MAINT_SYSTEMCTL="$w/systemctl" STEPHANIA_MAINT_OPS_SOCKET="$w/ops.sock" STEPHANIA_ENGINE_SOCKET="$w/engine.sock" \
  STEPHANIA_MAINT_UPDATE="$w/update" STEPHANIA_MAINT_INBOX="$w/inbox" "$bin" 2>"$w/maint.log" &
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


# steph's changes through the broker, from inside a real session.
if [ -n "$steph" ]; then
  st() { STEPHANIA_MAINT_OPS_SOCKET="$w/ops.sock" "$steph" "$@"; }
  principal maint
  rc=0; st restart control 2>/dev/null || rc=$?
  [ "$rc" = 7 ] && ok "steph changes nothing without an open window (exit 7)" || fail "no window: exit $rc"
  c=$(code -1)
  r=$(rpc window.open "{\"actor\":\"alice\",\"client_addr\":\"127.0.0.1\",\"code\":\"$c\"}")
  win=$(field "$r" window_id)
  [ -n "$win" ] || fail "window for steph: $r"
  for _ in $(seq 50); do [ -f "$w/sshd.pid" ] && break; sleep 0.1; done
  # sess <command…>: run in the maintenance session (SSH_USER_AUTH set by sshd).
  sess() {
    rm -f "$w/id" "$w/id.pub" "$w/id-cert.pub"
    ssh-keygen -q -t ed25519 -N '' -f "$w/id"
    r=$(rpc cert.sign "{\"window_id\":\"$win\",\"public_key\":\"$(cat "$w/id.pub")\",\"client_addr\":\"127.0.0.1\"}")
    field "$r" certificate >"$w/id-cert.pub"
    echo "@cert-authority stephania-e2e.local,127.0.0.1 $(field "$r" host_ca)" >"$w/known_hosts"
    ssh -F none -p "$port" -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none \
      -o IdentityFile="$w/id" -o CertificateFile="$w/id-cert.pub" \
      -o UserKnownHostsFile="$w/known_hosts" -o GlobalKnownHostsFile=/dev/null \
      -o StrictHostKeyChecking=yes -o HostKeyAlias=stephania-e2e.local -o ConnectTimeout=5 \
      "$me@127.0.0.1" "export STEPHANIA_MAINT_OPS_SOCKET=$w/ops.sock STEPHANIA_MAINT_INBOX=$w/inbox; $*"
  }
  who=$(sess "$steph" whoami --json | python3 -c 'import json,sys; w=json.load(sys.stdin); print(w["account"], w["window_id"])' 2>/dev/null || true)
  [ "$who" = "alice $win" ] && ok "steph whoami names the account and window from the session certificate" || fail "whoami: $who"
  sess "$steph" restart control 2>"$w/st.err" && grep -qx "restart stephania-control.service" "$w/systemctl.log" \
    && ok "steph restart control goes through the broker" || fail "restart control: $(cat "$w/st.err")"
  grep -q "\"event\":\"maintenance.ops_restart\".*\"actor\":\"alice@$win\"\|\"actor\":\"alice@$win\".*\"event\":\"maintenance.ops_restart\"" "$w/maint.log" \
    && ok "the broker audits the change with the certificate's key ID" || fail "ops audit: $(grep ops_ "$w/maint.log")"
  rc=0; sess "$steph" restart sshd 2>"$w/st.err" || rc=$?
  [ "$rc" = 4 ] && grep -q "steph cannot restart sshd" "$w/st.err" && ok "units outside the allowlist are refused (exit 4)" || fail "allowlist: exit $rc $(cat "$w/st.err")"
  engine streaming
  rc=0; sess "$steph" restart engine 2>"$w/st.err" || rc=$?
  [ "$rc" = 3 ] && grep -q "on air" "$w/st.err" && ok "restarting the engine is refused while on air (exit 3)" || fail "on air: exit $rc $(cat "$w/st.err")"
  rc=0; sess "$steph" reboot 2>/dev/null || rc=$?
  [ "$rc" = 3 ] && ! grep -qx reboot "$w/systemctl.log" && ok "reboot is refused while on air" || fail "reboot on air: exit $rc"
  sess "echo wrong-name | $steph restart engine --while-live" 2>/dev/null && fail "a wrong name was accepted" || true
  ! grep -qx "restart stephania.service" "$w/systemctl.log" && ok "--while-live needs the device name typed back" || fail "while-live without the name"
  host=$(cat /etc/hostname 2>/dev/null || hostname)
  sess "echo $host | $steph restart engine --while-live" 2>"$w/st.err" && grep -qx "restart stephania.service" "$w/systemctl.log" \
    && ok "--while-live with the device name restarts the engine on air" || fail "while-live: $(cat "$w/st.err") $(cat "$w/systemctl.log")"
  engine idle
  echo bad >"$w/outside.raucb"
  rc=0; sess "$steph" update apply "$w/outside.raucb" 2>"$w/st.err" || rc=$?
  [ "$rc" = 1 ] && [ ! -f "$w/update.log" ] && ok "bundles outside the inbox are refused" || fail "outside inbox: exit $rc $(cat "$w/st.err")"
  echo bad >"$w/inbox/bad.raucb"
  rc=0; sess "$steph" update apply bad.raucb 2>"$w/st.err" || rc=$?
  [ "$rc" = 5 ] && grep -q "bundle signature not valid" "$w/st.err" && ok "RAUC's refusal is shown (exit 5)" || fail "bad bundle: exit $rc $(cat "$w/st.err")"
  echo good >"$w/inbox/good.raucb"
  sess "$steph" update apply good.raucb 2>"$w/st.err" && grep -q "inbox/good.raucb" "$w/update.log" \
    && ok "steph update apply installs a bundle from the inbox" || fail "update apply: $(cat "$w/st.err")"
  st status --json | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["window"]["open"] is True and s["window"]["on_air"] is False, s["window"]' \
    && ok "steph status shows the window and on-air state" || fail "status window"
  sess "$steph" window close 2>/dev/null || true
  sleep 0.5
  [ "$(field "$(rpc status '{}')" open)" = False ] && [ ! -f "$w/sshd.pid" ] && [ -z "$(ls "$w/inbox")" ] \
    && ok "steph window close ends the window, stops sshd and empties the inbox" || fail "window close"
fi
grep -q '"event":"maintenance.root_certificate_issued"' "$w/maint.log" && ok "audit events in the journal" || fail "audit: $(cat "$w/maint.log")"
echo "maint-ssh-e2e: $pass checks passed"
