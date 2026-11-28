#!/usr/bin/env bash
# Android native build of Stephania Connect plus the pinned HTTPS client's unit
# tests against certificates issued by the real device PKI (stephania-pki).
# Usage: ci/mobile-android.sh <dir with stephania-pki>
set -Eeuo pipefail
bins=$(realpath "${1:?dir with stephania-pki}")
work=$(mktemp -d)
export STEPHANIA_PKI_DIR="$work/pki"
"$bins/stephania-pki" ensure --test-loopback --host stephania-ci --ip 127.0.0.1 --port 9443 > "$work/pki.json" || [ $? = 10 ]
pin=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["endpoint"]["ca_sha256"])' "$work/pki.json")
openssl pkcs12 -export -inkey "$STEPHANIA_PKI_DIR/server-key.pem" -in "$STEPHANIA_PKI_DIR/server.pem" -passout pass:stephania -out "$work/server.p12"
export STEPHANIA_PKI_P12="$work/server.p12" STEPHANIA_PKI_CA_SHA256="$pin"

cd apps/mobile
CI=1 npx expo prebuild --platform android --no-install --clean
cd android
./gradlew --no-daemon --stacktrace :stephania-pinned-fetch:testDebugUnitTest :app:assembleDebug -PreactNativeArchitectures=x86_64
report=$(find .. -path '*stephania-pinned-fetch*' -name 'TEST-*PinnedHttpTest.xml' -not -path '*/node_modules/*' | head -1)
test -n "$report" || { echo "PinnedHttpTest report missing"; exit 1; }
python3 - "$report" <<'PY'
import sys, xml.etree.ElementTree as ET
s = ET.parse(sys.argv[1]).getroot()
tests, fail, err, skip = (int(s.get(k, 0)) for k in ("tests", "failures", "errors", "skipped"))
print(f"PinnedHttpTest: {tests} tests, {fail} failures, {err} errors, {skip} skipped")
sys.exit(0 if tests >= 4 and fail == err == skip == 0 else 1)
PY
ls -la app/build/outputs/apk/debug/
