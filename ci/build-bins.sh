#!/usr/bin/env bash
# Builds every x86_64 binary the e2e jobs and the PR image smoke test need,
# once, into ci-bins/ (debug profile, stripped). Identical command ⇒
# identical artifacts, so jobs using cache profile `x64-bins` share one
# warm target directory (.github/actions/cached-build).
set -Eeuo pipefail
out=${1:-ci-bins}
mkdir -p "$out"
# Full engine (native capture/x264/AAC/H.264 decode) and the diagnostics
# engine without native media (control/Studio integration tests).
cargo build --locked --bin stephania
cp target/debug/stephania "$out/stephania"
cargo build --locked --no-default-features --bin stephania
cp target/debug/stephania "$out/stephania-diag"
# Device core with both the virtual and the NetworkManager backend, and the
# maintenance service (control integration test).
cargo build --locked --no-default-features --features ci-virtual,networkmanager --bin stephania-provision --bin stephania-pki --bin stephania-bootctl --bin stephania-maint
cp target/debug/stephania-provision target/debug/stephania-pki target/debug/stephania-bootctl target/debug/stephania-maint "$out/"
cargo build --locked -p stephania-phone
cp target/debug/stephania-phone "$out/"
# The maintenance CLI (ci/stephctl-e2e.sh).
cargo build --locked -p stephctl
cp target/debug/stephctl "$out/"
strip "$out"/*
ls -la "$out"
