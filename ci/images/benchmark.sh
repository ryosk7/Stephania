#!/usr/bin/env bash
set -Eeuo pipefail
ref="$(bash ci/images/pull.sh runtime)"
docker build --build-arg "BASE=$ref" -t t3box-bench -f ci/Dockerfile.benchmark .
docker run --rm --entrypoint ldd t3box-bench /app/t3box-ci-encode | tee /tmp/encoder-libs.txt
! grep -q "not found" /tmp/encoder-libs.txt
