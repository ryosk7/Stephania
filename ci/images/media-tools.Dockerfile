FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg openssl curl python3 python3-numpy fonts-dejavu-core fonts-noto-cjk tesseract-ocr tesseract-ocr-eng tesseract-ocr-jpn poppler-utils ca-certificates time && rm -rf /var/lib/apt/lists/*
