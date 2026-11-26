FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends qemu-system-arm cpio python3 gzip coreutils bash time && rm -rf /var/lib/apt/lists/*
