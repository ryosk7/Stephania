#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p transport-results
if [[ "$1" == controller || "$1" == combined ]]; then
  bash ci/smoke.sh | tee transport-results/controller.log
fi
if [[ "$1" == rtmps || "$1" == combined ]]; then
  bash ci/virtual-rtmps.sh
  mv e2e-results transport-results/baseline
  T3BOX_CI_WIDTH=1920 T3BOX_CI_HEIGHT=1080 T3BOX_CI_BITRATE=6000 T3BOX_CI_FRAMES=300 T3BOX_CI_DYNAMIC=1 T3BOX_CI_LOAD=1 bash ci/virtual-rtmps.sh
  mv e2e-results transport-results/venue
fi
