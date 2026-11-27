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
cargo build --locked --bin t3box
cp target/debug/t3box "$out/t3box"
cargo build --locked --no-default-features --bin t3box
cp target/debug/t3box "$out/t3box-diag"
# Device core with both the virtual and the NetworkManager backend.
cargo build --locked --no-default-features --features ci-virtual,networkmanager --bin t3box-provision --bin t3box-pki --bin t3box-bootctl
cp target/debug/t3box-provision target/debug/t3box-pki target/debug/t3box-bootctl "$out/"
cargo build --locked -p t3box-phone
cp target/debug/t3box-phone "$out/"
strip "$out"/*
ls -la "$out"
