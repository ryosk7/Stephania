#!/usr/bin/env bash
# Pull an immutable recipe-tagged image; authenticated fallback supplied by workflow.
set -Eeuo pipefail
kind="$1"
arch="$(uname -m)"
case "$arch" in aarch64) arch=arm64;; x86_64) arch=amd64;; *) exit 2;; esac
case "$kind" in build|runtime|media-tools|qemu-tools) ;; *) exit 2;; esac
hash="$(sha256sum "ci/images/$kind.Dockerfile" | cut -d' ' -f1)"
ref="ghcr.io/ryosk7/t3box-ci-$kind:$arch-$hash"
if ! docker image inspect "$ref" >/dev/null 2>&1; then
  docker pull "$ref" >&2
fi
docker inspect --format '{{index .RepoDigests 0}}' "$ref"
