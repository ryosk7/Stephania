#!/usr/bin/env bash
# Verify release artifacts as a user or the release pipeline would.
# Usage: os/release/verify.sh <dist-dir> [<trusted release-signing.pub> [<RAUC keyring (CA PEM)>]]
# The trusted keys default to the copies committed in this repository
# (os/release/keys/, fingerprints in docs/RELEASE.md). Keys taken from the
# download itself prove nothing and are refused.
# Needs OpenSSL 3, unzip, cpio, xz, python3 and rauc. Without rauc the bundle
# signature check is skipped (its hash is still covered by SHA256SUMS) unless
# STEPHANIA_VERIFY_REQUIRE_RAUC=1, as in the release pipeline.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
dist=$(cd "$1" && pwd)
pub=${2:-$here/keys/release-signing.pub}
keyring=${3:-$here/keys/rauc-ca.pem}
name=$(ls "$dist" | sed -n 's/\.raucb$//p' | head -1)
fail() { echo "FAIL $*"; exit 1; }
for k in "$pub" "$keyring"; do
  [ -f "$k" ] || fail "trusted key $k not found"
  [ "$(cd "$(dirname "$k")" && pwd)" != "$dist" ] || fail "trusted key $k comes from the download being verified"
done
[ -n "$name" ] || fail "no .raucb in $dist"
openssl pkeyutl -verify -pubin -inkey "$pub" -rawin -in "$dist/SHA256SUMS" -sigfile "$dist/SHA256SUMS.sig" >/dev/null || fail "SHA256SUMS signature"
(cd "$dist" && sha256sum -c --quiet SHA256SUMS) || fail "artifact hashes"
cmp -s "$pub" "$dist/release-signing.pub" || fail "release-signing.pub differs from the trusted key"
echo "PASS SHA256SUMS signed by the release key; all artifacts match"
w=$(mktemp -d); trap 'rm -rf "$w"' EXIT
unzip -q "$dist/$name.zip" -d "$w/zip"
openssl pkeyutl -verify -pubin -inkey "$pub" -rawin -in "$w/zip/stephania-install/manifest.json" -sigfile "$w/zip/stephania-install/manifest.sig" >/dev/null || fail "installer manifest signature"
for f in stephania-install/boot.vfat stephania-install/rootfs.squashfs installer.cpio.gz; do
  want=$(sed -n "s/.*\"${f##*/}_sha256\": *\"\([0-9a-f]*\)\".*/\1/p" "$w/zip/stephania-install/manifest.json")
  [ -n "$want" ] && [ "$(sha256sum "$w/zip/$f" | cut -d' ' -f1)" = "$want" ] || fail "installer payload $f"
done
# The installer checks the card against the key built into it; that key
# must be the trusted one, or a re-signed card would pass on the device.
(cd "$w" && gzip -dc "$w/zip/installer.cpio.gz" | cpio -i --quiet --to-stdout etc/stephania/install.pub >"$w/install.pub")
cmp -s "$pub" "$w/install.pub" || fail "installer's built-in install.pub differs from the trusted key"
echo "PASS installer ZIP: manifest signed, payload and installer key match"
# Verify the way the device does (os/rauc/system.conf): the signer must
# hold a code-signing certificate (check-purpose=codesign); RAUC's default
# purpose would reject it.
if ! command -v rauc >/dev/null; then
  [ "${STEPHANIA_VERIFY_REQUIRE_RAUC:-}" != 1 ] || fail "rauc is not installed"
  echo "SKIP update bundle signature: rauc is not installed (the bundle is covered by SHA256SUMS above)"
else
  printf '[system]\ncompatible=stephania-pi5\nbootloader=noop\n[keyring]\npath=%s\ncheck-purpose=codesign\n' "$(cd "$(dirname "$keyring")" && pwd)/$(basename "$keyring")" >"$w/rauc.conf"
  rauc -c "$w/rauc.conf" info "$dist/$name.raucb" >"$w/rauc.txt" 2>&1 || { cat "$w/rauc.txt"; fail "RAUC bundle signature"; }
  grep -q "Compatible: *'stephania-pi5'" "$w/rauc.txt" || grep -q "stephania-pi5" "$w/rauc.txt" || fail "bundle compatible"
  echo "PASS update bundle verifies against the release keyring"
fi
python3 - "$dist/sbom.spdx.json" <<'PY' || fail "SBOM"
import json, sys
d = json.load(open(sys.argv[1]))
assert d["spdxVersion"] == "SPDX-2.3"
eco = {p["externalRefs"][0]["referenceLocator"].split(":")[1].split("/")[0] for p in d["packages"] if p.get("externalRefs")}
assert {"cargo", "npm", "deb"} <= eco, eco
names = {p["name"] for p in d["packages"]}
assert {"libx264-164", "rustls"} <= names, "expected components missing"
print(f"PASS SBOM: {len(d['packages'])} packages ({', '.join(sorted(eco))})")
PY
# List to a file first: grep -q exits at the first match, and under
# pipefail tar's SIGPIPE would then fail the check.
tar -tzf "$dist/LICENSES.tar.gz" >"$w/licences.txt"
grep -q 'LICENSES/deb/.*libx264.*/copyright' "$w/licences.txt" || fail "deb copyright files missing from LICENSES"
echo "PASS licence texts include the system packages' copyright files"
xz -t "$dist/$name.img.xz" || fail "image archive"
echo "release verification: passed"
