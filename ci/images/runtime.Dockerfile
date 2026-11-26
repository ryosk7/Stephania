FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends libasound2t64 libx264-164 libavcodec60 libavutil58 time && rm -rf /var/lib/apt/lists/*
