# Performance measurements

Only measurements that were actually taken are reported, with where they
were taken. Nothing here is a Raspberry Pi 5 number unless it says so.

## Control plane under load (`ci/control-load.sh`, CI job `control`)

Real Puma in production mode (config/puma.rb) against the real engine over
IPC, started inside the §9 cgroup experiment with `systemd-run`
(`CPUQuota=50%`, `MemoryHigh=96M`, `MemoryMax=128M`), once with YJIT off
and once on. Four keep-alive clients run a Studio-like mix for 20 s after
a 3 s warm-up: status (engine IPC) 40 %, scene list 25 %, audio 10 %,
capabilities 10 %, scene saves with optimistic revisions 15 %. Throughout,
the maximum of 8 event streams is held open and a ninth must get 429.

The run fails on any error, a dropped stream, a ninth stream that is not
refused, or p99 above 250 ms. Each run's table (req/s, p50/p95/p99/max,
memory peak of the cgroup) is in the job summary and the `control-load`
artifact. These are GitHub-hosted x86_64 numbers: they show regressions and
the effect of YJIT and the cgroup limits on the same machine, not device
capacity.

Findings so far:
- One IPC connection per request made `/status` fail under load: the
  engine frees a connection slot only when its thread sees EOF, so bursts
  exceeded its limit of 8 (`busy`, and `EPIPE` on the client). The control
  client now pools at most 6 connections and reuses them; the same load
  runs without errors.
- An event stream whose client vanished keeps its slot until the next
  keepalive write fails (at most 15 s); a new client can get 429 in that
  window.

## Not measured yet (needs hardware)
- Control plane on a Pi 5 while the engine streams (R06): run
  `ci/control-load.sh` on the device with the engine live and record it in
  docs/QUALIFICATION.md.
- Media path: per-core CPU, frame time, A/V sync, temperature/throttling
  for HDMI-only and three-input composition (§25, R01).
