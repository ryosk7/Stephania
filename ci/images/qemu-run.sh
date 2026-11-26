#!/usr/bin/env bash
set -Eeuo pipefail
ref="$(bash ci/images/pull.sh qemu-tools)"
args=()
if [[ -c /dev/kvm && -r /dev/kvm && -w /dev/kvm ]]; then args+=(--device /dev/kvm); fi
docker run --rm --network none "${args[@]}" --user "$(id -u):$(id -g)" -v "$PWD:/work" -w /work "$ref" bash tests/virtual-lab/qemu.sh
