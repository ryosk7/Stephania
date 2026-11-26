# Paced media benchmark

Uses production NativeVideoPipeline (ultrafast) and AAC/RTMPS, with a benchmark-only file source and synthetic audio. Local TLS receiver, clean and fixed 40 ms/0.2%/12 Mbit/s netem. 30fps paced; no camera, display, internet ingest, reconnect or true end-to-end latency claim. Sources may already be compressed; metrics compare normalized YUV420 reference. No VMAF available by design, no production changes. No images/videos uploaded. Four scenes, two rates, two network conditions.
