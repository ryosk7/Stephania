# CI strategy — native AArch64

The normal CI workflow now uses a GitHub-hosted `ubuntu-24.04-arm`
runner for real ARM64 user-space execution, instead of repeatedly compiling
Rust inside an emulated ARM64 Docker image with QEMU.

The standard ARM64 runner is available to private repositories as of
January 2026. This is still **not an actual Raspberry Pi 5**.

- `rust-native`: x86_64 Rust unit tests and Clippy, with Cargo cache.
- `arm64-native`: ARM64 unit tests, native release build, and synthetic
  FFmpeg H.264/AAC + HTTP controller integration test.
- Cache: cargo registry/build artifacts (`Swatinem/rust-cache`).
- Duplicate push and PR checks: avoided with push-on-main, PR-on-features.
- Superseded checks: canceled via a per-branch/workflow concurrency group.
- The old `ci/Dockerfile.arm64` remains as an optional manual reproduction
  route and is no longer built on every PR.

## Required hardware acceptance

Native ARM64 CI cannot validate UVC/ALSA peripheral timing, Pi 5 RP1,
HDMI loop-through or EDID, Raspberry Pi power, GPIO, thermal throttling or
actual venue Wi-Fi. See [the physical acceptance criteria](../docs/ACCEPTANCE.md)
and the manual `hardware.yml` workflow with a Pi 5 self-hosted runner.

## Measuring improvements

Compare job durations in the GitHub Actions run API (each job's
`started_at` and `completed_at`), not just the commit-to-run completion time.
Cache-warm and cache-cold runs should be distinguished.
