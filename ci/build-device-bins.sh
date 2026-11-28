#!/usr/bin/env bash
# x86_64 binaries for the PR image smoke test (os.yml image-qemu-x64), built
# in ci/images/device-build.Dockerfile (Debian trixie, like the image) with
# the same features as ci/build-bins.sh: the provisioning daemon carries the
# virtual BLE backend because QEMU has no radio. stephania-ble is built and
# installed like on the Pi (the smoke test checks it is enabled); without a
# Bluetooth controller it does not start.
set -Eeuo pipefail
out=${1:-device-bins}
mkdir -p "$out"
cargo build --locked --bin stephania --bin stephania-pki --bin stephania-bootctl --bin stephania-maint --bin steph
cargo build --locked -p stephania-phone -p stephania-ble
cp target/debug/stephania target/debug/stephania-pki target/debug/stephania-bootctl target/debug/stephania-maint target/debug/steph target/debug/stephania-phone target/debug/stephania-ble "$out/"
cargo build --locked --no-default-features --features ci-virtual,networkmanager --bin stephania-provision
cp target/debug/stephania-provision "$out/"
strip "$out"/*
ls -la "$out"
