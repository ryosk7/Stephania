#!/usr/bin/env bash
# Release signing split (os/release/sign.sh, verify.sh, os/installer/sign.sh)
# on a small synthetic stage: the signed result verifies, and a changed
# stage, a foreign installer key, keys taken from the download and an
# attacker-signed release are all refused.
# Needs openssl 3, rauc, squashfs-tools, cpio, zip, unzip, xz, python3.
# Usage: ci/release-sign-e2e.sh
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
export STEPHANIA_RELEASE_VERSION=9.9.9 SOURCE_DATE_EPOCH=1700000000 STEPHANIA_VERIFY_REQUIRE_RAUC=1
pass() { echo "ok - $*"; }
refused() { local what=$1; shift; if "$@" >"$t/out.log" 2>&1; then cat "$t/out.log"; echo "not ok - $what was accepted"; exit 1; fi; pass "$what refused ($(tail -1 "$t/out.log"))"; }

keys() { bash "$repo/os/release/test-keys.sh" "$1" >/dev/null; openssl pkey -in "$1/release-signing.pem" -pubout -out "$1/release-signing.pub"; }
keys "$t/k"; keys "$t/evil"

# stage <dir> <install.pub>: what os/release/stage.sh would hand over.
stage() {
  local s=$1 z=$1/card
  mkdir -p "$z/stephania-install" "$t/sq" "$t/ir/etc/stephania" "$t/lic/LICENSES/deb/libx264-164"
  echo root >"$t/sq/f"; mksquashfs "$t/sq" "$z/stephania-install/rootfs.squashfs" -quiet -noappend >/dev/null
  head -c 65536 /dev/urandom >"$z/stephania-install/boot.vfat"
  cp "$2" "$t/ir/etc/stephania/install.pub"
  (cd "$t/ir" && find . | LC_ALL=C sort | cpio -o -H newc --quiet --reproducible | gzip -9n) >"$z/installer.cpio.gz"
  sha() { sha256sum "$1" | cut -d' ' -f1; }
  printf '{\n "format": "stephania-install",\n "version": "%s",\n "boot.vfat_sha256": "%s",\n "rootfs.squashfs_sha256": "%s",\n "installer.cpio.gz_sha256": "%s"\n}\n' \
    "$STEPHANIA_RELEASE_VERSION" "$(sha "$z/stephania-install/boot.vfat")" "$(sha "$z/stephania-install/rootfs.squashfs")" "$(sha "$z/installer.cpio.gz")" \
    >"$z/stephania-install/manifest.json"
  echo image | xz -c >"$s/stephania-os-pi5.img.xz"
  python3 - "$s/sbom.spdx.json" <<'PY'
import json, sys
pk = lambda n, eco: {"name": n, "externalRefs": [{"referenceLocator": f"pkg:{eco}/{n}@1"}]}
json.dump({"spdxVersion": "SPDX-2.3", "packages": [pk("rustls", "cargo"), pk("react", "npm"), pk("libx264-164", "deb")]}, open(sys.argv[1], "w"))
PY
  echo inventory >"$s/THIRD_PARTY.md"
  echo text >"$t/lic/LICENSES/deb/libx264-164/copyright"
  tar -C "$t/lic" -czf "$s/LICENSES.tar.gz" LICENSES
  (cd "$s" && find . -type f ! -name STAGE.sha256 -print0 | LC_ALL=C sort -z | xargs -0 sha256sum >STAGE.sha256)
}

sign() { # sign <stage> <dist> <keys-dir> [env...]
  local s=$1 d=$2 k=$3; shift 3
  env STEPHANIA_RELEASE_SIGNING_KEY="$k/release-signing.pem" STEPHANIA_RAUC_CERT="$k/rauc-signing.pem" STEPHANIA_RAUC_KEY="$k/rauc-signing.key" \
    "$@" bash "$repo/os/release/sign.sh" "$s" "$d"
}

stage "$t/s" "$t/k/release-signing.pub"
digest=$(sha256sum "$t/s/STAGE.sha256" | cut -d' ' -f1)
mkdir -p "$t/extra"; echo bin >"$t/extra/stephctl-9.9.9-x86_64-unknown-linux-gnu.tar.gz"
sign "$t/s" "$t/d" "$t/k" STEPHANIA_STAGE_DIGEST="$digest" STEPHANIA_RELEASE_EXTRA="$t/extra" >/dev/null
grep -q ' stephctl-9.9.9-x86_64-unknown-linux-gnu.tar.gz$' "$t/d/SHA256SUMS" || { echo "not ok - stephctl missing from SHA256SUMS"; exit 1; }
bash "$repo/os/release/verify.sh" "$t/d" "$t/k/release-signing.pub" "$t/k/rauc-ca.pem" >"$t/out.log" || { cat "$t/out.log"; echo "not ok - verify"; exit 1; }
pass "signed stage verifies, stephctl listed in SHA256SUMS"

# A stage that changed after the build listed it.
cp -a "$t/s" "$t/s2"; echo changed >>"$t/s2/THIRD_PARTY.md"
refused "changed stage file" sign "$t/s2" "$t/x" "$t/k"
cp -a "$t/s" "$t/s3"; echo extra >"$t/s3/card/extra.txt"
refused "unlisted stage file" sign "$t/s3" "$t/x" "$t/k"
refused "stage digest mismatch" sign "$t/s" "$t/x" "$t/k" STEPHANIA_STAGE_DIGEST=0000
refused "extra file shadowing an artifact" sign "$t/s" "$t/x" "$t/k" STEPHANIA_RELEASE_EXTRA="$t/d"
# The installer must carry the key that signs the card.
stage "$t/s4" "$t/evil/release-signing.pub"
refused "installer built with a foreign install.pub" sign "$t/s4" "$t/x" "$t/k"

# Verification needs a trust anchor from outside the download.
refused "trusted key taken from the download" bash "$repo/os/release/verify.sh" "$t/d" "$t/d/release-signing.pub" "$t/k/rauc-ca.pem"
# A complete release re-signed with an attacker's keys.
sign "$t/s4" "$t/e" "$t/evil" >/dev/null
refused "attacker-signed release" bash "$repo/os/release/verify.sh" "$t/e" "$t/k/release-signing.pub" "$t/k/rauc-ca.pem"
# A ZIP whose installer key differs while SHA256SUMS is genuinely signed.
cp -a "$t/d" "$t/m"; cp "$t/e/stephania-os-pi5.zip" "$t/m/"
(cd "$t/m" && sha256sum stephania-os-pi5.zip stephania-os-pi5.img.xz stephania-os-pi5.raucb sbom.spdx.json THIRD_PARTY.md LICENSES.tar.gz release-signing.pub >SHA256SUMS)
openssl pkeyutl -sign -inkey "$t/k/release-signing.pem" -rawin -in "$t/m/SHA256SUMS" -out "$t/m/SHA256SUMS.sig"
refused "installer ZIP signed by a foreign key" bash "$repo/os/release/verify.sh" "$t/m" "$t/k/release-signing.pub" "$t/k/rauc-ca.pem"
echo "release signing e2e: passed"
