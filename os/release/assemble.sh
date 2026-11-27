#!/usr/bin/env bash
# Release artifacts (§7) from an image build and an installer build:
#   <name>.zip  <name>.img.xz  <name>.raucb  sbom.spdx.json  THIRD_PARTY.md
#   LICENSES.tar.gz  SHA256SUMS  SHA256SUMS.sig  release-signing.pub
# Usage: os/release/assemble.sh <image-out-dir> <installer-out-dir> <dist-dir>
# Environment (all required — a release is never assembled unsigned):
#   T3BOX_RELEASE_VERSION      e.g. 1.2.0
#   T3BOX_RELEASE_NAME         default t3box-os-pi5
#   T3BOX_RELEASE_SIGNING_KEY  Ed25519 PEM; signs SHA256SUMS (and signed the
#                              installer manifest, os/installer/build.sh)
#   T3BOX_RAUC_CERT, T3BOX_RAUC_KEY  code-signing certificate and key for
#                              the update bundle (chain to the image keyring)
#   SOURCE_DATE_EPOCH          fixed timestamps (tag commit time)
set -euo pipefail
img=$(cd "$1" && pwd)
inst=$(cd "$2" && pwd)
dist=$(mkdir -p "$3" && cd "$3" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
version=${T3BOX_RELEASE_VERSION:?T3BOX_RELEASE_VERSION is required}
name=${T3BOX_RELEASE_NAME:-t3box-os-pi5}
key=${T3BOX_RELEASE_SIGNING_KEY:?T3BOX_RELEASE_SIGNING_KEY is required}
rauc_cert=${T3BOX_RAUC_CERT:?T3BOX_RAUC_CERT is required}
rauc_key=${T3BOX_RAUC_KEY:?T3BOX_RAUC_KEY is required}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:?SOURCE_DATE_EPOCH is required}
rm -rf "${dist:?}"/*

# The installer manifest must be signed by the same release key.
openssl pkey -in "$key" -pubout -out "$dist/release-signing.pub"
openssl pkeyutl -verify -pubin -inkey "$dist/release-signing.pub" -rawin \
  -in "$inst/zip/t3box-install/manifest.json" -sigfile "$inst/zip/t3box-install/manifest.sig" >/dev/null \
  || { echo "installer manifest is not signed by the release key" >&2; exit 1; }
cp "$inst/t3box-os.zip" "$dist/$name.zip"

# Raw image for recovery flashing.
xz -T0 -9 -c --keep "$img/t3box.img" >"$dist/$name.img.xz"

# Signed verity update bundle (os/rauc/system.conf, docs/UPDATE.md).
b=$(mktemp -d)
cp "$img/rootfs.squashfs" "$b/rootfs.img"
cp "$img/boot-a.vfat" "$b/boot.vfat"
printf '[update]\ncompatible=t3box-pi5\nversion=%s\n[bundle]\nformat=verity\n[image.rootfs]\nfilename=rootfs.img\n[image.boot]\nfilename=boot.vfat\n' "$version" >"$b/manifest.raucm"
rauc bundle --cert="$rauc_cert" --key="$rauc_key" "$b" "$dist/$name.raucb" >/dev/null
rm -rf "$b"

# SBOM and licence texts for what the device ships.
python3 "$repo/ci/sbom.py" "$dist/sbom" --scope device --dpkg "$img/dpkg.tsv" --version "$version" ${T3BOX_SBOM_GEMS:+--gems-from-bundle}
mv "$dist/sbom/sbom.spdx.json" "$dist/sbom/THIRD_PARTY.md" "$dist/"
mkdir -p "$dist/sbom/LICENSES/deb"
if [ -d "$img/rootfs/usr/share/doc" ]; then
  (cd "$img/rootfs/usr/share/doc" && find . -name copyright -print0 | LC_ALL=C sort -z | while IFS= read -r -d '' f; do
     install -D -m 0644 "$f" "$dist/sbom/LICENSES/deb/$f"; done)
fi
cp "$repo/LICENSE" "$dist/sbom/LICENSES/T3BOX-LICENSE"
tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner -C "$dist/sbom" -czf "$dist/LICENSES.tar.gz" LICENSES
rm -rf "$dist/sbom"

(cd "$dist" && sha256sum "$name.zip" "$name.img.xz" "$name.raucb" sbom.spdx.json THIRD_PARTY.md LICENSES.tar.gz release-signing.pub >SHA256SUMS)
openssl pkeyutl -sign -inkey "$key" -rawin -in "$dist/SHA256SUMS" -out "$dist/SHA256SUMS.sig"
ls -la "$dist"
