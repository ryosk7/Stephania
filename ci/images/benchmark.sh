#!/usr/bin/env bash
set -Eeuo pipefail
ref="$(bash ci/images/pull.sh runtime)"
docker build --build-arg "BASE=$ref" -t stephania-bench -f ci/Dockerfile.benchmark .
docker run --rm --entrypoint ldd stephania-bench /app/stephania-ci-encode | tee /tmp/encoder-libs.txt
! grep -q "not found" /tmp/encoder-libs.txt
