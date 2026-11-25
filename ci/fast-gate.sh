#!/usr/bin/env bash
set -Eeuo pipefail
# Build/install is deliberately outside the execution budget.
# All prebuilt test binaries are provided by the workflow.
mkdir -p fast-results
start=$SECONDS
timeout 60 "$1" accelerated_ci:: --nocapture --test-threads=2 | tee fast-results/simulation.txt
printf 'simulation_wall_seconds=%s\n' "$((SECONDS-start))" | tee fast-results/timing.txt
