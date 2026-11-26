#!/usr/bin/env bash
set -Eeuo pipefail
# Build/install is deliberately outside the execution budget.
# All prebuilt test binaries are provided by the workflow.
mkdir -p fast-results
start=$SECONDS
/usr/bin/time -f "elapsed_seconds=%e" -o fast-results/precise-timing.txt timeout --kill-after=5s 60 "$1" accelerated_ci:: --nocapture --test-threads=2 | tee fast-results/simulation.txt
grep -q "3 passed; 0 failed" fast-results/simulation.txt
printf 'simulation_wall_seconds=%s\n' "$((SECONDS-start))" | tee fast-results/timing.txt
