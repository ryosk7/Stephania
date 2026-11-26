#!/usr/bin/env bash
# Actual Pi 5 capture smoke, not a three-hour or YouTube acceptance test.
set -Eeuo pipefail
: "${T3BOX_VIDEO:=/dev/video0}"
: "${T3BOX_AUDIO:=plughw:1,0}"
: "${T3BOX_WIDTH:=1920}"
: "${T3BOX_HEIGHT:=1080}"
: "${T3BOX_FPS:=30}"
mkdir -p hardware-results
work="$(mktemp -d)"
trap 'if [[ -f "$work/capture.flv" ]]; then cp "$work/capture.flv" hardware-results/capture.flv; fi; rm -rf "$work"' EXIT
v4l2-ctl -d "$T3BOX_VIDEO" --all >hardware-results/video-device.txt
v4l2-ctl -d "$T3BOX_VIDEO" --list-formats-ext >>hardware-results/video-device.txt
arecord -l >hardware-results/audio-device.txt
timeout 45 ffmpeg -hide_banner -loglevel warning -nostdin -y \
  -thread_queue_size 64 -f v4l2 -framerate "$T3BOX_FPS" \
  -video_size "${T3BOX_WIDTH}x${T3BOX_HEIGHT}" -i "$T3BOX_VIDEO" \
  -thread_queue_size 64 -f alsa -i "$T3BOX_AUDIO" \
  -map 0:v:0 -map 1:a:0 -t 20 -c:v libx264 -preset ultrafast -g 60 -pix_fmt yuv420p \
  -c:a aac -ar 48000 -ac 2 -b:a 160k -f flv "$work/capture.flv" 2>hardware-results/capture.log
ffprobe -v error -count_frames -show_streams -show_format -of json "$work/capture.flv" >hardware-results/probe.json
ffmpeg -v error -xerror -i "$work/capture.flv" -f null -
python3 - "$T3BOX_WIDTH" "$T3BOX_HEIGHT" "$T3BOX_FPS" <<'PY'
import json,sys
x=json.load(open('hardware-results/probe.json'))
v=next(s for s in x['streams'] if s['codec_name']=='h264')
a=next(s for s in x['streams'] if s['codec_name']=='aac')
assert (v['width'],v['height'])==(int(sys.argv[1]),int(sys.argv[2]))
assert int(v['nb_read_frames'])>=int(sys.argv[3])*19
assert int(a['sample_rate'])==48000 and a['channels']==2 and int(a['nb_read_frames'])>=890
assert float(x['format']['duration'])>=19
print('Physical FFmpeg capture smoke PASS; native/YouTube/3h acceptance not covered')
PY
