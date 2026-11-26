#!/usr/bin/env bash
# All processes share one container network namespace and monotonic clock.
# Preserve native CPU execution; no host networking or host qdisc mutation.
set -Eeuo pipefail
ref="$(bash ci/images/pull.sh media-tools)"
args=()
for key in LAB_MEDIA LAB_INPUT LAB_SECONDS LAB_FAULT LAB_NETWORK LAB_FIXTURE_CACHE LAB_OUT LAB_WIDTH LAB_HEIGHT LAB_BITRATE; do
  if [[ -v "$key" ]]; then args+=(-e "$key"); fi
done
docker run --rm --cap-add NET_ADMIN --user 0:0 -e HOME=/tmp \
  "${args[@]}" -v "$PWD:/work" -w /work "$ref" \
  bash -c 'trap "chown -R $1:$2 evidence .lab-fixture 2>/dev/null || true" EXIT; bash tests/virtual-lab/run.sh' _ "$(id -u)" "$(id -g)"
