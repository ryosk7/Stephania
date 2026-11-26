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

## Resource-constrained native media benchmark

On ARM64, the native YUYV conversion, x264, AAC encoder, PCM framing and
clock correction run together inside a Docker container limited to 4 vCPU
and 4 GiB RAM with swap disabled. This is an assumed 4 GB Pi 5 envelope,
not Cortex-A76 emulation: host CPU microarchitecture, cache, RAM bandwidth,
thermal behavior and GPU/USB are different. No Pi 5 throughput claim is
valid without calibration on physical hardware.

The deterministic 1080p30 four-second fixture combines moving test patterns
and small slide/code text. Four configurations compare ultrafast/veryfast
at 6/8 Mbps. Artifacts include decoded frame counts, codec metadata,
PSNR/SSIM, actual bitrate, offline throughput, per-input p95 wall time,
peak RSS, CPU metadata and preview images. The provisional integrity
floors (25 dB PSNR and 0.85 SSIM) are not a guarantee of readable text;
the reference is post-4:2:0 conversion. Inspect previews and compare
against a calibrated real Pi and representative presentations.
No VMAF, OCR, live RTMPS, device timing, thermal or soak test is implied.

## Readability and long-duration coverage

OCR uses Tesseract on four samples of the 14-pixel text line, reports the
worst normalized text similarity (not exact character recognition rate).
VMAF is reported only when the installed FFmpeg exposes libvmaf; otherwise
explicitly N/A. Manual CI workflow input soak=true runs 324000 paced frames
(three hours at 30 fps) with 4 vCPU / 4 GiB, samples container CPU and RAM,
and fails when processing falls more than ten seconds behind at minute
boundaries. It loops a short source and discards encoded bytes, so it is
a codec soak, not a full live-network or device soak.

## Virtual capture and local RTMPS fault injection

The optional ci-virtual feature supplies clock-paced YUYV and PCM frames
to the same native publishing/retry loop. It is a userspace substitute,
not a V4L2 or ALSA kernel device. A local Python TLS/RTMP peer records FLV,
forces a connection close, then checks new sequence headers, initial IDR,
monotonic per-track timestamps and decoded recovery. The test uses a
temporary private CA trusted only for t3box-ci.test in ci-virtual builds;
production certificate validation is not disabled. Linux netem on the
ephemeral runner adds delay, jitter, loss and rate limiting. The test uses
640x360 for bounded CI runtime; the separate offline benchmark uses 1080p.
Do not enable ci-virtual in production. Real kernel device and venue network
acceptance remain necessary.

## Venue scenario and runner limits

The hosted ARM64 runner actually exposed two CPUs in CI run 45720024622.
The codec and soak containers now use min(nproc,4), and record the actual
quota; a two-core run is not a four-core Pi 5 equivalent. The 4 GiB memory
limit is a ceiling, not a reservation. A separate 1080p30 scenario adds
changing network latency/loss/bandwidth and competing CPU/memory load.
The local receiver records first/last timestamps and maximum observed
cross-track packet timestamp skew. The provisional 500 ms end-timestamp
difference bound is not a lip-sync measurement (capture markers are needed).
The test does not emulate HDMI/USB replug, resolution renegotiation, YouTube
transcoding, real Wi-Fi radio, or a three-hour network stream.

## Accelerated gate

The default ARM64 workflow runs an optimized logical gate with a 60-second
execution timeout, excluding provisioning, compilation and artifact upload.
It evaluates five full three-hour audio sample-clock trajectories using the
same arithmetic as the live PCM corrector, checks scalar/PCM equivalence
and actual framing over a shorter window, and runs 16 seeded three-hour
video queue/retry scenarios without image payloads or sleeps. The queue is
a library component, not the native capture channel; network availability
and processing rate are models. No claim is made that the full video codec
or transport processed three hours of data. The extended manual workflow
retains physical-time network and objective quality tests, and soak=true
retains the optional paced codec test. Actual timing must be read from CI.

## Parallel fast CI

Normal PR/push CI uses independent matrix runners for x86 unit/Clippy,
ARM64 unit, three logical shards, three transport scenarios, and four
quality profiles. Quality and transport are default gates, not manual-only.
Each logical, transport and quality execution step has a 60s timeout;
install, compile, Docker preparation, scheduling and artifact upload are
outside that budget. Quality profiles run on separate hosts to avoid CPU
contention. Per-shard Cargo caches reduce repeated warm builds; cold builds
are deliberately independent and duplicate some work. GitHub account
concurrency limits may serialize jobs; a 60s whole-workflow SLA is not
guaranteed. The optional physical-time soak remains manual and independent.

## Cost-aware CI (supersedes 13-job matrix)

A conservative path planner selects no heavy jobs for Markdown-only PRs,
one representative quality profile for explicitly controller-only PRs,
and all four profiles for unknown or other code changes. Push to main and
manual runs always use full coverage. Full runs use six worker jobs:
x86 checks, ARM64 unit+logical, two transport and two quality jobs.
A lightweight planner and final aggregate gate add two jobs. The controller
and RTMPS tests are retained; related workloads share setup/build/fixtures.
Execution suites retain 60-second timeouts. Three-day artifacts exclude
raw quality bitstreams on success. Actual billable usage and wall time
depend on account limits, rounding and cache; job-count reduction is not a
measured cost reduction. Required check should be ci-gate after validating
branch protection; this change does not modify repository rules.

## Measured media acceptance (provisional)

Normal CI exposes Markdown and JSON reports even when threshold checks fail.
Native codec profiles measure 1080p30 frame counts, offline FPS, p95 processing
time, process CPU, peak RSS, bitrate, PSNR, SSIM, OCR and optional VMAF.
Production ultrafast gates: >=30 offline FPS, >=30 dB PSNR, >=0.90 SSIM,
>=90% normalized OCR similarity, <=1 GiB peak RSS, all frames decoded.
Veryfast speed is comparative. Local RTMPS receiver gates media FPS >=29,
arrival FPS >=27, estimated missing <=3%, final timestamp difference <=100ms,
max packet timestamp skew <=250ms and last-to-first recovery gap <=10s.
These are initial engineering thresholds, not hardware or YouTube guarantees.
No extra runner is required; each quality profile decodes once for PSNR/SSIM
and four preview samples. The existing 60s suite timeout remains.

## CI review safeguards

Quality jobs require exact expected profile names and per-profile completion
markers, collect partial failures, and emit evidence even when a profile
fails. VMAF is opt-in (T3BOX_BENCH_VMAF=1) to protect the fast gate budget.
Transport scenarios write to separate artifact directories, preserve partial
receives, require the dynamic impairment worker to finish successfully and
check competing load remains active. The planner runs inexpensive negative
contract tests before dispatching costly workers. The codec image verifies
runtime shared-library resolution before timing. No physical calibration or
unmeasured quality claims are made.

## Repository-wide CI audit

The invalid YAML plain scalar ending in a colon is replaced with a block.
Default-feature builds/tests are explicitly checked alongside all-feature
builds. Controller smoke now requires decoded H.264/AAC, correct status,
idempotent start/stop, 403 cross-origin and 404 unknown endpoints. Logical
fault tests require every modeled outage to recover, not merely a minimum
number of recoveries. Optional soak validates completed frames, elapsed
time, OOM and telemetry presence. Physical smoke now validates codec,
geometry, audio and decoded frame counts; it still tests the FFmpeg
compatibility route only. Uncovered: real device hotplug, native capture
on Pi, NetworkManager success paths, systemd deployment, YouTube ingest,
async RTMP controls, hardware-clock calibration and long-run leak proof.

## Verification assurance revision

All executable changes now run all four codec profiles and both transport
groups. Only documentation-only PRs may skip expensive jobs. Unknown
changes run full coverage. The aggregate gate includes explicitly requested
soak results. Workflow expressions are checked using pinned actionlint;
Python and shell syntax and negative measurement contracts run in the
planner. Quality evidence requires 120 PSNR and SSIM frame records, decoded
audio duration and non-silent unclipped signal, positive finite performance
metrics, and a unique expected profile manifest. Transport additionally
checks audio packet coverage and maximum video timestamp gap. Reports remain
provisional; neither hardware acceptance nor full production equivalence
is claimed. Real-device smoke is separate, manual and not a YouTube test.
