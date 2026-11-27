#!/usr/bin/env bash
# x86_64 binaries for the PR image smoke test (os.yml image-qemu-x64), built
# in ci/images/device-build.Dockerfile (Debian trixie, like the image) with
# the same features as ci/build-bins.sh: the provisioning daemon carries the
# virtual BLE backend because QEMU has no radio.
set -Eeuo pipefail
out=${1:-device-bins}
mkdir -p "$out"
cargo build --locked --bin stephania --bin stephania-pki --bin stephania-bootctl
cargo build --locked -p stephania-phone
cp target/debug/stephania target/debug/stephania-pki target/debug/stephania-bootctl target/debug/stephania-phone "$out/"
cargo build --locked --no-default-features --features ci-virtual,networkmanager --bin stephania-provision
cp target/debug/stephania-provision "$out/"
strip "$out"/*
ls -la "$out"
