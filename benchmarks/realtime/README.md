# Paced media benchmark

Uses production NativeVideoPipeline (ultrafast) and AAC/RTMPS, with a benchmark-only file source and synthetic audio. Local TLS receiver, clean and fixed 40 ms/0.2%/12 Mbit/s netem. 30fps paced; no camera, display, internet ingest, reconnect or true end-to-end latency claim. Sources may already be compressed; metrics compare normalized YUV420 reference. No VMAF available by design, no production changes. No images/videos uploaded. Four scenes, two rates, two network conditions.

## Phase 1/2 diagnostics
Five independent repetitions per condition. Official Xiph lossy WebM sources (not uncompressed originals). Sender host and receiver namespace with veth and IFB ingress shaping; reverse ACK path also shaped. Per-frame receiver timestamps, payload bytes and scheduled sender wall time allow same-host scheduled-input-to-receive delay (not viewer latency). TCP ss and qdisc sampled every 100 ms approximately. Media-timestamp and arrival-time rolling peak rates exclude protocol overhead. Original CI receiver copied to benchmark-only receiver.py; CI unchanged.
