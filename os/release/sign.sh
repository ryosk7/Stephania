#!/usr/bin/env bash
# Signed half of a release (§7): turns a stage from os/release/stage.sh into
#   <name>.zip  <name>.img.xz  <name>.raucb  sbom.spdx.json  THIRD_PARTY.md
#   LICENSES.tar.gz  [extra files]  SHA256SUMS  SHA256SUMS.sig  release-signing.pub
# It runs only first-party scripts, openssl, rauc, zip and coreutils: the
# release workflow runs it in a job that installs no dependencies.
# Usage: os/release/sign.sh <stage-dir> <dist-dir>
# Environment (keys required — a release is never assembled unsigned):
#   STEPHANIA_RELEASE_VERSION      e.g. 1.2.0
#   STEPHANIA_RELEASE_NAME         default stephania-os-pi5
#   STEPHANIA_RELEASE_SIGNING_KEY  Ed25519 PEM; signs the installer manifest and SHA256SUMS
#   STEPHANIA_RAUC_CERT, STEPHANIA_RAUC_KEY  code-signing certificate and key for
#                              the update bundle (chain to the image keyring)
#   STEPHANIA_STAGE_DIGEST         optional SHA-256 of STAGE.sha256 as the build
#                              job reported it; refused if it differs
#   STEPHANIA_RELEASE_EXTRA        optional directory of further release files
#                              (stephctl archives), listed in SHA256SUMS too
#   SOURCE_DATE_EPOCH              fixed timestamps (tag commit time)
set -euo pipefail
stage=$(cd "$1" && pwd)
dist=$(mkdir -p "$2" && cd "$2" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
version=${STEPHANIA_RELEASE_VERSION:?STEPHANIA_RELEASE_VERSION is required}
name=${STEPHANIA_RELEASE_NAME:-stephania-os-pi5}
key=${STEPHANIA_RELEASE_SIGNING_KEY:?STEPHANIA_RELEASE_SIGNING_KEY is required}
rauc_cert=${STEPHANIA_RAUC_CERT:?STEPHANIA_RAUC_CERT is required}
rauc_key=${STEPHANIA_RAUC_KEY:?STEPHANIA_RAUC_KEY is required}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:?SOURCE_DATE_EPOCH is required}
fail() { echo "sign.sh: $*" >&2; exit 1; }

# The stage must be exactly what the build listed: no changed, missing or
# added files.
if [ -n "${STEPHANIA_STAGE_DIGEST:-}" ]; then
  [ "$(sha256sum "$stage/STAGE.sha256" | cut -d' ' -f1)" = "$STEPHANIA_STAGE_DIGEST" ] || fail "STAGE.sha256 is not the one the build reported"
fi
(cd "$stage" && sha256sum -c --strict --quiet STAGE.sha256) || fail "stage files do not match STAGE.sha256"
diff <(cd "$stage" && find . -type f ! -name STAGE.sha256 | LC_ALL=C sort) <(sed 's/^[0-9a-f]\{64\}  //' "$stage/STAGE.sha256" | LC_ALL=C sort) >/dev/null \
  || fail "stage holds files that STAGE.sha256 does not list"

# The installer manifest is what the device trusts: check it names the
# payload actually on the card before signing it.
card="$stage/card"
m="$card/stephania-install/manifest.json"
for f in stephania-install/boot.vfat stephania-install/rootfs.squashfs installer.cpio.gz; do
  want=$(sed -n "s/.*\"${f##*/}_sha256\": *\"\([0-9a-f]*\)\".*/\1/p" "$m")
  [ -n "$want" ] && [ "$(sha256sum "$card/$f" | cut -d' ' -f1)" = "$want" ] || fail "installer manifest does not match $f"
done
grep -q "\"version\": *\"$version\"" "$m" || fail "installer manifest is not version $version"

rm -rf "${dist:?}"/*
openssl pkey -in "$key" -pubout -out "$dist/release-signing.pub"
"$repo/os/installer/sign.sh" "$card" "$key" "$dist/$name.zip"
rm -f "$card/stephania-install/manifest.sig"

# Signed verity update bundle (os/rauc/system.conf, docs/UPDATE.md) of the
# same root and boot filesystems the installer writes.
b=$(mktemp -d); trap 'rm -rf "$b"' EXIT
cp "$card/stephania-install/rootfs.squashfs" "$b/rootfs.img"
cp "$card/stephania-install/boot.vfat" "$b/boot.vfat"
printf '[update]\ncompatible=stephania-pi5\nversion=%s\n[bundle]\nformat=verity\n[image.rootfs]\nfilename=rootfs.img\n[image.boot]\nfilename=boot.vfat\n' "$version" >"$b/manifest.raucm"
rauc bundle --cert="$rauc_cert" --key="$rauc_key" "$b" "$dist/$name.raucb" >/dev/null

cp "$stage/$name.img.xz" "$stage/sbom.spdx.json" "$stage/THIRD_PARTY.md" "$stage/LICENSES.tar.gz" "$dist/"
extra=()
if [ -n "${STEPHANIA_RELEASE_EXTRA:-}" ]; then
  for f in "$STEPHANIA_RELEASE_EXTRA"/*; do
    [ -f "$f" ] || continue
    n=$(basename "$f")
    [ -e "$dist/$n" ] && fail "extra file $n would replace a release artifact"
    cp "$f" "$dist/$n"; extra+=("$n")
  done
fi

(cd "$dist" && sha256sum "$name.zip" "$name.img.xz" "$name.raucb" sbom.spdx.json THIRD_PARTY.md LICENSES.tar.gz "${extra[@]}" release-signing.pub >SHA256SUMS)
openssl pkeyutl -sign -inkey "$key" -rawin -in "$dist/SHA256SUMS" -out "$dist/SHA256SUMS.sig"
ls -la "$dist"
