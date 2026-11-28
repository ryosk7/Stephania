#!/usr/bin/env bash
# Release artifacts (§7) from an image build and an installer build in one
# go: os/release/stage.sh (no keys) then os/release/sign.sh (keys). The
# release workflow runs the two halves in separate jobs so that no signing
# key exists where build dependencies run; CI uses this wrapper with
# throwaway keys (os/release/test-keys.sh).
#   <name>.zip  <name>.img.xz  <name>.raucb  sbom.spdx.json  THIRD_PARTY.md
#   LICENSES.tar.gz  SHA256SUMS  SHA256SUMS.sig  release-signing.pub
# Usage: os/release/assemble.sh <image-out-dir> <installer-out-dir> <dist-dir>
# Environment: as for stage.sh and sign.sh.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
dist=$(mkdir -p "$3" && cd "$3" && pwd)
stage=$(mktemp -d); trap 'rm -rf "$stage"' EXIT
"$here/stage.sh" "$1" "$2" "$stage"
"$here/sign.sh" "$stage" "$dist"
