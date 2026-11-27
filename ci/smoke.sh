#!/usr/bin/env bash
set -Eeuo pipefail
LAB_OUT=${STEPHANIA_SMOKE_OUT:-transport-results/controller} LAB_SECONDS=8 LAB_FAULT=none LAB_NETWORK=clean bash tests/virtual-lab/run.sh
