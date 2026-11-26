#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p transport-results
rc=0
case "$1" in controller|rtmps|combined) ;; *) exit 2 ;; esac
if [[ "$1" == controller || "$1" == combined ]]; then
  bash ci/smoke.sh 2>&1 | tee transport-results/controller.log || rc=1
fi
if [[ "$1" == rtmps || "$1" == combined ]]; then
  T3BOX_CI_OUT=transport-results/baseline bash ci/virtual-rtmps.sh || rc=1
  T3BOX_CI_OUT=transport-results/venue T3BOX_CI_WIDTH=1920 T3BOX_CI_HEIGHT=1080 T3BOX_CI_BITRATE=6000 T3BOX_CI_FRAMES=300 T3BOX_CI_DYNAMIC=1 T3BOX_CI_LOAD=1 bash ci/virtual-rtmps.sh || rc=1
fi
exit "$rc"
