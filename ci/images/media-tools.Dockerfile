FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg openssl curl python3 python3-numpy python3-pil fonts-dejavu-core fonts-noto-cjk tesseract-ocr tesseract-ocr-eng tesseract-ocr-jpn poppler-utils ca-certificates time libasound2t64 libx264-164 libavcodec60 libavutil58 iproute2 stress-ng sudo bash git procps util-linux && rm -rf /var/lib/apt/lists/*
