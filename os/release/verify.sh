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
openssl pkeyutl -verify -pubin -inkey "$pub" -rawin -in "$w/zip/stephania-install/manifest.json" -sigfile "$w/zip/stephania-install/manifest.sig" >/dev/null || fail "installer manifest signature"
for f in boot.vfat rootfs.squashfs; do
  want=$(sed -n "s/.*\"$f\_sha256\": *\"\([0-9a-f]*\)\".*/\1/p" "$w/zip/stephania-install/manifest.json")
  [ "$(sha256sum "$w/zip/stephania-install/$f" | cut -d' ' -f1)" = "$want" ] || fail "installer payload $f"
done
echo "PASS installer ZIP: manifest signed, payload matches"
# Verify the way the device does (os/rauc/system.conf): the signer must
# hold a code-signing certificate (check-purpose=codesign); RAUC's default
# purpose would reject it.
printf '[system]\ncompatible=stephania-pi5\nbootloader=noop\n[keyring]\npath=%s\ncheck-purpose=codesign\n' "$(cd "$(dirname "$keyring")" && pwd)/$(basename "$keyring")" >"$w/rauc.conf"
rauc -c "$w/rauc.conf" info "$dist/$name.raucb" >"$w/rauc.txt" 2>&1 || { cat "$w/rauc.txt"; fail "RAUC bundle signature"; }
grep -q "Compatible: *'stephania-pi5'" "$w/rauc.txt" || grep -q "stephania-pi5" "$w/rauc.txt" || fail "bundle compatible"
echo "PASS update bundle verifies against the release keyring"
# Each boot slot gets the boot image of its own pair (os/rauc/bundle-hook):
# a B install must boot rootfs-b with rauc.slot=B.
unsquashfs -q -n -d "$w/bundle" "$dist/$name.raucb" manifest.raucm hook boot-a.vfat boot-b.vfat >/dev/null 2>&1 || fail "bundle content"
grep -qx 'filename=hook' "$w/bundle/manifest.raucm" && grep -qE '^hooks=install;?$' "$w/bundle/manifest.raucm" && [ -x "$w/bundle/hook" ] \
  || fail "bundle installs the boot slots without the per-slot hook"
for slot in a b; do
  upper=$(echo "$slot" | tr a-z A-Z)
  cmd=$(mtype -i "$w/bundle/boot-$slot.vfat" ::/cmdline.txt) || fail "bundle boot-$slot.vfat"
  for want in "root=PARTLABEL=rootfs-$slot" "rauc.slot=$upper"; do
    case " $cmd " in *" $want "*) ;; *) fail "bundle boot-$slot.vfat cmdline lacks $want: $cmd" ;; esac
  done
done
echo "PASS update bundle carries a boot image per slot, each naming its own rootfs and slot"
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
