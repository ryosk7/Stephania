#!/usr/bin/env bash
# x86_64 binaries for the PR image smoke test (os.yml image-qemu-x64), built
# in ci/images/device-build.Dockerfile (Debian trixie, like the image) with
# the same features as ci/build-bins.sh: the provisioning daemon carries the
# virtual BLE backend because QEMU has no radio.
set -Eeuo pipefail
out=${1:-device-bins}
mkdir -p "$out"
cargo build --locked --bin t3box --bin t3box-pki --bin t3box-bootctl
cargo build --locked -p t3box-phone
cp target/debug/t3box target/debug/t3box-pki target/debug/t3box-bootctl target/debug/t3box-phone "$out/"
cargo build --locked --no-default-features --features ci-virtual,networkmanager --bin t3box-provision
cp target/debug/t3box-provision "$out/"
strip "$out"/*
ls -la "$out"
