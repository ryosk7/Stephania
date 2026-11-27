#!/usr/bin/env bash
# Release artifacts (§7) from an image build and an installer build:
#   <name>.zip  <name>.img.xz  <name>.raucb  sbom.spdx.json  THIRD_PARTY.md
#   LICENSES.tar.gz  SHA256SUMS  SHA256SUMS.sig  release-signing.pub
# Usage: os/release/assemble.sh <image-out-dir> <installer-out-dir> <dist-dir>
# Environment (all required — a release is never assembled unsigned):
#   STEPHANIA_RELEASE_VERSION      e.g. 1.2.0
#   STEPHANIA_RELEASE_NAME         default stephania-os-pi5
#   STEPHANIA_RELEASE_SIGNING_KEY  Ed25519 PEM; signs SHA256SUMS (and signed the
#                              installer manifest, os/installer/build.sh)
#   STEPHANIA_RAUC_CERT, STEPHANIA_RAUC_KEY  code-signing certificate and key for
#                              the update bundle (chain to the image keyring)
#   SOURCE_DATE_EPOCH          fixed timestamps (tag commit time)
set -euo pipefail
img=$(cd "$1" && pwd)
inst=$(cd "$2" && pwd)
dist=$(mkdir -p "$3" && cd "$3" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
version=${STEPHANIA_RELEASE_VERSION:?STEPHANIA_RELEASE_VERSION is required}
name=${STEPHANIA_RELEASE_NAME:-stephania-os-pi5}
key=${STEPHANIA_RELEASE_SIGNING_KEY:?STEPHANIA_RELEASE_SIGNING_KEY is required}
rauc_cert=${STEPHANIA_RAUC_CERT:?STEPHANIA_RAUC_CERT is required}
rauc_key=${STEPHANIA_RAUC_KEY:?STEPHANIA_RAUC_KEY is required}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:?SOURCE_DATE_EPOCH is required}
rm -rf "${dist:?}"/*

# The installer manifest must be signed by the same release key.
openssl pkey -in "$key" -pubout -out "$dist/release-signing.pub"
openssl pkeyutl -verify -pubin -inkey "$dist/release-signing.pub" -rawin \
  -in "$inst/zip/stephania-install/manifest.json" -sigfile "$inst/zip/stephania-install/manifest.sig" >/dev/null \
  || { echo "installer manifest is not signed by the release key" >&2; exit 1; }
cp "$inst/stephania-os.zip" "$dist/$name.zip"

# Raw image for recovery flashing.
# STEPHANIA_RELEASE_XZ_LEVEL lowers the level for CI checks only (default 9).
xz -T0 "-${STEPHANIA_RELEASE_XZ_LEVEL:-9}" -c --keep "$img/stephania.img" >"$dist/$name.img.xz"

# Signed verity update bundle (os/rauc/system.conf, docs/UPDATE.md).
b=$(mktemp -d)
cp "$img/rootfs.squashfs" "$b/rootfs.img"
cp "$img/boot-a.vfat" "$b/boot.vfat"
printf '[update]\ncompatible=stephania-pi5\nversion=%s\n[bundle]\nformat=verity\n[image.rootfs]\nfilename=rootfs.img\n[image.boot]\nfilename=boot.vfat\n' "$version" >"$b/manifest.raucm"
rauc bundle --cert="$rauc_cert" --key="$rauc_key" "$b" "$dist/$name.raucb" >/dev/null
rm -rf "$b"

# SBOM and licence texts for what the device ships.
python3 "$repo/ci/sbom.py" "$dist/sbom" --scope device --dpkg "$img/dpkg.tsv" --version "$version" ${STEPHANIA_SBOM_GEMS:+--gems-from-bundle}
mv "$dist/sbom/sbom.spdx.json" "$dist/sbom/THIRD_PARTY.md" "$dist/"
mkdir -p "$dist/sbom/LICENSES/deb"
if [ -d "$img/rootfs/usr/share/doc" ]; then
  (cd "$img/rootfs/usr/share/doc" && find . -name copyright -print0 | LC_ALL=C sort -z | while IFS= read -r -d '' f; do
     install -D -m 0644 "$f" "$dist/sbom/LICENSES/deb/$f"; done)
fi
cp "$repo/LICENSE" "$dist/sbom/LICENSES/STEPHANIA-LICENSE"
tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner -C "$dist/sbom" -czf "$dist/LICENSES.tar.gz" LICENSES
rm -rf "$dist/sbom"

(cd "$dist" && sha256sum "$name.zip" "$name.img.xz" "$name.raucb" sbom.spdx.json THIRD_PARTY.md LICENSES.tar.gz release-signing.pub >SHA256SUMS)
openssl pkeyutl -sign -inkey "$key" -rawin -in "$dist/SHA256SUMS" -out "$dist/SHA256SUMS.sig"
ls -la "$dist"
