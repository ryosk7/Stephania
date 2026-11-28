#!/usr/bin/env bash
# Pull a recipe-tagged image; authenticated fallback supplied by workflow.
# The tag names the Dockerfile's hash, but the registry content behind it is
# mutable. STEPHANIA_CI_IMAGE_LOCAL=1 never pulls: the image is built here
# from the committed recipe (release builds, .github/workflows/release.yml).
set -Eeuo pipefail
kind="$1"
arch="$(uname -m)"
case "$arch" in aarch64) arch=arm64;; x86_64) arch=amd64;; *) exit 2;; esac
case "$kind" in build|device-build|runtime|media-tools|qemu-tools) ;; *) exit 2;; esac
hash="$(sha256sum "ci/images/$kind.Dockerfile" | cut -d' ' -f1)"
ref="ghcr.io/ryosk7/stephania-ci-$kind:$arch-$hash"
if [ "${STEPHANIA_CI_IMAGE_LOCAL:-}" = 1 ]; then
  ref="stephania-ci-local/$kind:$arch-$hash"
  docker build --pull --platform "linux/$arch" -t "$ref" - <"ci/images/$kind.Dockerfile" >&2
  printf '%s\n' "$ref"
  exit 0
fi
if ! docker image inspect "$ref" >/dev/null 2>&1; then
  if ! docker pull "$ref" >&2; then
    echo "Registry image unavailable; building generic recipe locally (not pushing)" >&2
    ctx="$(mktemp -d)"
    trap 'rm -rf "$ctx"' EXIT
    cp "ci/images/$kind.Dockerfile" "$ctx/Dockerfile"
    # A concurrent publisher may finish while the consumer is starting.
    for attempt in 1 2; do
      if docker pull "$ref" >&2; then break; fi
      sleep 3
    done
    if ! docker image inspect "$ref" >/dev/null 2>&1; then
      docker build --platform "linux/$arch" -t "$ref" "$ctx" >&2
    fi
  fi
fi
digest="$(docker inspect --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}' "$ref")"
printf '%s\n' "${digest:-$ref}"
