#!/usr/bin/env bash
# Sign an installer card tree and pack it as the release ZIP.
# Usage: os/installer/sign.sh <card-dir> <Ed25519 private key PEM> <out.zip>
#   <card-dir> is <zip-out-dir>/zip from os/installer/build.sh. The key must
#   match the public key built into its installer.cpio.gz
#   (etc/stephania/install.pub), or the installer would refuse the card.
# The ZIP is reproducible: sorted entries, no extra attributes, stored.
set -euo pipefail
z=$(cd "$1" && pwd); key=$2; zip_out=$3
m="$z/stephania-install/manifest.json"
[ -f "$m" ] || { echo "missing $m" >&2; exit 1; }
w=$(mktemp -d); trap 'rm -rf "$w"' EXIT
openssl pkey -in "$key" -pubout -out "$w/key.pub"
(cd "$w" && gzip -dc "$z/installer.cpio.gz" | cpio -i --quiet --to-stdout etc/stephania/install.pub >"$w/install.pub")
cmp -s "$w/key.pub" "$w/install.pub" || { echo "signing key does not match the installer's built-in install.pub" >&2; exit 1; }
openssl pkeyutl -sign -inkey "$key" -rawin -in "$m" -out "$z/stephania-install/manifest.sig"
openssl pkeyutl -verify -pubin -inkey "$w/install.pub" -rawin -in "$m" -sigfile "$z/stephania-install/manifest.sig" >/dev/null
rm -f "$zip_out"
(cd "$z" && find . -type f | LC_ALL=C sort | TZ=UTC zip -X -q -0 "$zip_out" -@)
