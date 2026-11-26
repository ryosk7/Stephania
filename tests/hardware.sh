#!/usr/bin/env bash
set -Eeuo pipefail
LAB_INPUT=physical LAB_SECONDS=${LAB_SECONDS:-120} bash tests/virtual-lab/run.sh
