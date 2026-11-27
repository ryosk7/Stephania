#!/usr/bin/env bash
# Verify release artifacts as a user or the release pipeline would.
# Usage: os/release/verify.sh <dist-dir> <trusted release-signing.pub> <RAUC keyring (CA PEM)>
set -euo pipefail
dist=$(cd "$1" && pwd); pub=$2; keyring=$3
name=$(ls "$dist" | sed -n 's/\.raucb$//p' | head -1)
fail() { echo "FAIL $*"; exit 1; }
openssl pkeyutl -verify -pubin -inkey "$pub" -rawin -in "$dist/SHA256SUMS" -sigfile "$dist/SHA256SUMS.sig" >/dev/null || fail "SHA256SUMS signature"
(cd "$dist" && sha256sum -c --quiet SHA256SUMS) || fail "artifact hashes"
cmp -s "$pub" "$dist/release-signing.pub" || fail "release-signing.pub differs from the trusted key"
echo "PASS SHA256SUMS signed by the release key; all artifacts match"
w=$(mktemp -d); trap 'rm -rf "$w"' EXIT
unzip -q "$dist/$name.zip" -d "$w/zip"
openssl pkeyutl -verify -pubin -inkey "$pub" -rawin -in "$w/zip/t3box-install/manifest.json" -sigfile "$w/zip/t3box-install/manifest.sig" >/dev/null || fail "installer manifest signature"
for f in boot.vfat rootfs.squashfs; do
  want=$(sed -n "s/.*\"$f\_sha256\": *\"\([0-9a-f]*\)\".*/\1/p" "$w/zip/t3box-install/manifest.json")
  [ "$(sha256sum "$w/zip/t3box-install/$f" | cut -d' ' -f1)" = "$want" ] || fail "installer payload $f"
done
echo "PASS installer ZIP: manifest signed, payload matches"
# Verify the way the device does (os/rauc/system.conf): the signer must
# hold a code-signing certificate (check-purpose=codesign); RAUC's default
# purpose would reject it.
printf '[system]\ncompatible=t3box-pi5\nbootloader=noop\n[keyring]\npath=%s\ncheck-purpose=codesign\n' "$(cd "$(dirname "$keyring")" && pwd)/$(basename "$keyring")" >"$w/rauc.conf"
rauc -c "$w/rauc.conf" info "$dist/$name.raucb" >"$w/rauc.txt" 2>&1 || { cat "$w/rauc.txt"; fail "RAUC bundle signature"; }
grep -q "Compatible: *'t3box-pi5'" "$w/rauc.txt" || grep -q "t3box-pi5" "$w/rauc.txt" || fail "bundle compatible"
echo "PASS update bundle verifies against the release keyring"
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
tar -tzf "$dist/LICENSES.tar.gz" | grep -q 'LICENSES/deb/.*libx264.*/copyright' || fail "deb copyright files missing from LICENSES"
echo "PASS licence texts include the system packages' copyright files"
xz -t "$dist/$name.img.xz" || fail "image archive"
echo "release verification: passed"
