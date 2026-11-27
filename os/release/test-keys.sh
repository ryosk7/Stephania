#!/usr/bin/env bash
# Throwaway release keys for CI and local trials (never for a release):
# Ed25519 release-signing key, and a RAUC CA + code-signing certificate.
# Usage: os/release/test-keys.sh <dir>
set -euo pipefail
d=$1; mkdir -p "$d"; cd "$d"
openssl genpkey -algorithm ed25519 -out release-signing.pem 2>/dev/null
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 7 -subj /CN=t3box-test-release-ca \
  -keyout rauc-ca.key -out rauc-ca.pem -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign >/dev/null 2>&1
openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj /CN=t3box-test-release -keyout rauc-signing.key -out rauc-signing.csr >/dev/null 2>&1
openssl x509 -req -in rauc-signing.csr -CA rauc-ca.pem -CAkey rauc-ca.key -CAcreateserial -days 7 -out rauc-signing.pem \
  -extfile <(printf 'keyUsage=critical,digitalSignature\nextendedKeyUsage=codeSigning\n') >/dev/null 2>&1
chmod 0644 ./*.pem ./*.key
echo "test keys in $d"
