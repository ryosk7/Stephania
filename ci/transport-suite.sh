#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p transport-results
rc=0
case "$1" in controller|rtmps|combined) ;; *) exit 2 ;; esac
if [[ "$1" == controller || "$1" == combined ]]; then
  LAB_OUT=transport-results/controller LAB_SECONDS=8 LAB_FAULT=none LAB_NETWORK=clean bash tests/virtual-lab/run.sh 2>&1 | tee transport-results/controller.log || rc=1
fi
if [[ "$1" == rtmps || "$1" == combined ]]; then
  STEPHANIA_CI_OUT=transport-results/baseline bash ci/virtual-rtmps.sh || rc=1
  STEPHANIA_CI_OUT=transport-results/venue STEPHANIA_CI_WIDTH=1920 STEPHANIA_CI_HEIGHT=1080 STEPHANIA_CI_BITRATE=6000 STEPHANIA_CI_FRAMES=300 STEPHANIA_CI_DYNAMIC=1 STEPHANIA_CI_LOAD=1 bash ci/virtual-rtmps.sh || rc=1
fi
exit "$rc"
