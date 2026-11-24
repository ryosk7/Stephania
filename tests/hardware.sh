#!/usr/bin/env bash
# Requires actual Raspberry Pi 5, HDMI UVC device, and USB audio device.
set -Eeuo pipefail
: "${T3BOX_VIDEO:=/dev/video0}"
: "${T3BOX_AUDIO:=plughw:1,0}"
: "${T3BOX_WIDTH:=1920}"
: "${T3BOX_HEIGHT:=1080}"
: "${T3BOX_FPS:=30}"
v4l2-ctl -d "$T3BOX_VIDEO" --all
v4l2-ctl -d "$T3BOX_VIDEO" --list-formats-ext
arecord -l
ffmpeg -hide_banner -loglevel warning -nostdin -y \
  -thread_queue_size 512 -f v4l2 -framerate "$T3BOX_FPS" \
  -video_size "${T3BOX_WIDTH}x${T3BOX_HEIGHT}" -i "$T3BOX_VIDEO" \
  -thread_queue_size 512 -f alsa -i "$T3BOX_AUDIO" \
  -t 20 -c:v libx264 -preset ultrafast -g 60 -pix_fmt yuv420p \
  -c:a aac -ar 48000 -b:a 160k -f flv /tmp/t3box-hardware.flv
test -s /tmp/t3box-hardware.flv
ffprobe -v error -show_entries stream=codec_name,width,height -of compact /tmp/t3box-hardware.flv
echo "Real Pi HDMI/audio smoke test passed; continuous 3h testing is still required."
