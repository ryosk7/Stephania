#!/usr/bin/env bash
# Trusted-LAN end to end: device PKI → owner enrollment over the
# provisioning socket → TLS control plane (real Puma) verified against the
# CA pinned over BLE → factory-reset style revocation.
# Usage: ci/lan-tls-e2e.sh <target-dir>
set -euo pipefail
target=$1
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap cleanup EXIT
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; for f in "$work"/*.log; do echo "--- $f"; cat "$f"; done; exit 1; }

port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
loop=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
export STEPHANIA_PKI_DIR=$work/pki
mkdir -p "$work/tokens" "$work/cred"

set +e; "$target/stephania-pki" ensure --test-loopback --host stephania-ci --ip 127.0.0.1 --ip 8.8.8.8 --port "$port" >"$work/pki.json"; rc=$?; set -e
[ "$rc" = 10 ] || fail "first ensure must issue (exit $rc)"
python3 - "$work/pki.json" <<'PY' || fail "endpoint json"
import json,sys; e=json.load(open(sys.argv[1]))["endpoint"]; assert e["ips"]==["127.0.0.1"], e
PY
"$target/stephania-pki" ensure --test-loopback --host stephania-ci --ip 127.0.0.1 --port "$port" >/dev/null || fail "second ensure must be unchanged (exit $?)"
pass "PKI issued once, public address excluded"
openssl verify -CAfile "$work/pki/ca.pem" -verify_hostname stephania-ci.local <(awk '/BEGIN/{n++} n==1' "$work/pki/server.pem") >/dev/null || fail "openssl chain"
pass "openssl verifies leaf for stephania-ci.local"

STEPHANIA_PROVISION_SOCKET="$work/p.sock" STEPHANIA_CREDENTIALS_DIR="$work/cred" \
  STEPHANIA_NETWORK_BACKEND=virtual STEPHANIA_MANAGEMENT_TOKENS="$work/tokens/tokens.json" \
  "$target/stephania-provision" >"$work/provision.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$work/p.sock" ] && break; sleep 0.1; done

(cd "$root/services/control" && STEPHANIA_CONTROL_PORT=$loop STEPHANIA_CONTROL_TLS_PORT=$port STEPHANIA_CONTROL_TLS_BIND=127.0.0.1 \
  STEPHANIA_CONTROL_TOKEN=ci-bootstrap-token-0123456789abcdef STEPHANIA_CONTROL_TOKENS_FILE="$work/tokens/tokens.json" \
  STEPHANIA_ENGINE_SOCKET="$work/none.sock" STEPHANIA_CONTROL_DB="$work/control.db" RACK_ENV=production \
  bundle exec puma -C config/puma.rb config.ru >"$work/puma.log" 2>&1) & pids+=($!)
for _ in $(seq 100); do curl -s -o /dev/null "http://127.0.0.1:$loop/" && break; sleep 0.2; done

python3 "$root/ci/lan_enroll.py" "$work/p.sock" "$port" "$work" || fail "enrollment"
token=$(cat "$work/token")
[ "$(cat "$work/pinned-ca.sha256")" = "$(openssl x509 -in "$work/pki/ca.pem" -outform der | sha256sum | cut -d' ' -f1)" ] || fail "pin mismatch"
pass "pin delivered over BLE equals the device CA"

certs=$(openssl s_client -connect "127.0.0.1:$port" -servername stephania-ci.local -showcerts </dev/null 2>/dev/null | grep -c 'BEGIN CERTIFICATE')
[ "$certs" -ge 2 ] || fail "server must send leaf + CA ($certs certs) so phones can match the pin"
pass "server sends leaf and CA"

https() { curl -sS -o "$work/body" -w '%{http_code}' --cacert "$work/pki/ca.pem" --resolve "stephania-ci.local:$port:127.0.0.1" "$@"; }
code=$(https -H "Authorization: Bearer $token" "https://stephania-ci.local:$port/api/v1/capabilities") || fail "TLS request"
[ "$code" = 200 ] || fail "issued token over TLS: $code $(cat "$work/body")"
pass "issued token accepted over TLS with CA verification"
code=$(https -H "Authorization: Bearer ci-bootstrap-token-0123456789abcdef" "https://stephania-ci.local:$port/api/v1/capabilities")
[ "$code" = 401 ] || fail "bootstrap token must be loopback-only: $code"
pass "bootstrap token rejected on the LAN listener"
code=$(https -H "Host: localhost" -H "Authorization: Bearer ci-bootstrap-token-0123456789abcdef" "https://stephania-ci.local:$port/api/v1/capabilities")
[ "$code" = 403 ] || fail "bootstrap token with Host: localhost on the LAN listener: $code"
code=$(https -H "X-Forwarded-Host: localhost" -H "Authorization: Bearer ci-bootstrap-token-0123456789abcdef" "https://stephania-ci.local:$port/api/v1/capabilities")
[ "$code" = 401 ] || fail "bootstrap token with X-Forwarded-Host: localhost on the LAN listener: $code"
pass "Host / X-Forwarded-Host cannot make a LAN request local"
code=$(curl -sS -o /dev/null -w '%{http_code}' --cacert "$work/pki/ca.pem" --resolve "evil.example:$port:127.0.0.1" -H "Authorization: Bearer $token" "https://evil.example:$port/api/v1/capabilities" || true)
[ "$code" = 000 ] || [ "$code" = 403 ] || fail "uncertified name must fail TLS or Host check: $code"
pass "uncertified host name refused ($code)"
code=$(curl -sS -o /dev/null -w '%{http_code}' -k --resolve "evil.example:$port:127.0.0.1" -H "Host: evil.example" -H "Authorization: Bearer $token" "https://evil.example:$port/api/v1/capabilities")
[ "$code" = 403 ] || fail "DNS-rebinding Host must be 403: $code"
pass "Host outside the certificate rejected even when TLS is bypassed"
proto=$(curl -sS -o /dev/null -w '%{http_code}' --tls-max 1.1 --cacert "$work/pki/ca.pem" --resolve "stephania-ci.local:$port:127.0.0.1" "https://stephania-ci.local:$port/" 2>/dev/null || true)
[ "$proto" = 000 ] || fail "TLS <= 1.1 must be refused ($proto)"
pass "TLS 1.0/1.1 refused"
code=$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $token" "http://127.0.0.1:$loop/api/v1/capabilities")
[ "$code" = 200 ] || fail "loopback plain HTTP: $code"

# Revocation: a factory reset clears the token file (provisioning core
# rule, unit-tested); the control plane must notice without a restart.
python3 -c 'import json,sys;json.dump({"tokens":[]},open(sys.argv[1],"w"))' "$work/tokens/tokens.json"
touch -d '+5 seconds' "$work/tokens/tokens.json"
code=$(https -H "Authorization: Bearer $token" "https://stephania-ci.local:$port/api/v1/capabilities")
[ "$code" = 401 ] || fail "revoked token: $code"
pass "revoked token rejected without restart"
echo "LAN TLS e2e: all checks passed"
