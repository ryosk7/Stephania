#!/usr/bin/env bash
# Unsigned half of a release (§7): everything that needs the build tree or
# third-party tooling but no signing key. os/release/sign.sh finishes it.
# Usage: os/release/stage.sh <image-out-dir> <installer-out-dir> <stage-dir>
#   <installer-out-dir> may hold a signed or an unsigned card tree
#   (os/installer/build.sh with STEPHANIA_INSTALL_PUBLIC_KEY); its signature
#   is dropped and made again by sign.sh.
# Environment:
#   STEPHANIA_RELEASE_VERSION      e.g. 1.2.0 (required)
#   STEPHANIA_RELEASE_NAME         default stephania-os-pi5
#   STEPHANIA_RELEASE_XZ_LEVEL     lowers the xz level for CI checks only (default 9)
#   STEPHANIA_SBOM_GEMS            set: licences from the installed gems
#   SOURCE_DATE_EPOCH              fixed timestamps (tag commit time, required)
# Output: <stage-dir>/{card/,<name>.img.xz,sbom.spdx.json,THIRD_PARTY.md,
#         LICENSES.tar.gz,STAGE.sha256}. STAGE.sha256 lists every file, so the
#         signing job can refuse a stage that changed in transit.
set -euo pipefail
img=$(cd "$1" && pwd)
inst=$(cd "$2" && pwd)
stage=$(mkdir -p "$3" && cd "$3" && pwd)
repo=$(cd "$(dirname "$0")/../.." && pwd)
version=${STEPHANIA_RELEASE_VERSION:?STEPHANIA_RELEASE_VERSION is required}
name=${STEPHANIA_RELEASE_NAME:-stephania-os-pi5}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:?SOURCE_DATE_EPOCH is required}
rm -rf "${stage:?}"/*

# Installer card tree; sign.sh signs its manifest and packs the ZIP.
cp -a "$inst/zip" "$stage/card"
rm -f "$stage/card/stephania-install/manifest.sig"

# Raw image for recovery flashing.
xz -T0 "-${STEPHANIA_RELEASE_XZ_LEVEL:-9}" -c --keep "$img/stephania.img" >"$stage/$name.img.xz"

# SBOM and licence texts for what the device ships.
python3 "$repo/ci/sbom.py" "$stage/sbom" --scope device --dpkg "$img/dpkg.tsv" --version "$version" ${STEPHANIA_SBOM_GEMS:+--gems-from-bundle}
mv "$stage/sbom/sbom.spdx.json" "$stage/sbom/THIRD_PARTY.md" "$stage/"
mkdir -p "$stage/sbom/LICENSES/deb"
if [ -d "$img/rootfs/usr/share/doc" ]; then
  (cd "$img/rootfs/usr/share/doc" && find . -name copyright -print0 | LC_ALL=C sort -z | while IFS= read -r -d '' f; do
     install -D -m 0644 "$f" "$stage/sbom/LICENSES/deb/$f"; done)
fi
cp "$repo/LICENSE" "$stage/sbom/LICENSES/STEPHANIA-LICENSE"
tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner -C "$stage/sbom" -czf "$stage/LICENSES.tar.gz" LICENSES
rm -rf "$stage/sbom"

(cd "$stage" && find . -type f ! -name STAGE.sha256 -print0 | LC_ALL=C sort -z | xargs -0 sha256sum >STAGE.sha256)
echo "release stage $version: $stage (STAGE.sha256 $(sha256sum "$stage/STAGE.sha256" | cut -d' ' -f1))"
