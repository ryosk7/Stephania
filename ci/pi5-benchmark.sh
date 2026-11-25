#!/usr/bin/env bash
set -Eeuo pipefail
out="$(realpath "${1:-ci-results}")"
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Generate 4 seconds of moving content and small text. Fixture preparation
# and objective decoding are outside the measured encoder resource budget.
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=1920x1080:rate=30' \
  -vf "drawbox=x=50:y=50:w=1820:h=450:color=white:t=fill,drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='T3Box LT  |  fn main() { println!(1234567890); }':fontsize=22:fontcolor=black:x=75:y=90,drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='Small code  0123456789  ABCDEFGHIJKLMNOPQRSTUVWXYZ':fontsize=14:fontcolor=black:x=75:y=160" \
  -frames:v 120 -threads 1 -pix_fmt yuyv422 -f rawvideo "$work/input.yuv"
ffmpeg -v error -y -f rawvideo -pixel_format yuyv422 -video_size 1920x1080 -framerate 30 -i "$work/input.yuv" -pix_fmt yuv420p -c:v ffv1 "$work/ref.mkv"
lscpu > "$out/cpu.txt"
free -b > "$out/host-memory.txt"
uname -a > "$out/kernel.txt"
ffmpeg -version > "$out/ffmpeg.txt"
docker build -t t3box-bench -f ci/Dockerfile.benchmark .
for preset in ultrafast veryfast; do
  for bitrate in 6000 8000; do
    name="$preset-$bitrate"
    docker run --name "t3box-bench-$name" --cpus=4 --memory=4g --memory-swap=4g \
      --network=none -v "$work:/fixture:ro" -v "$out:/results" t3box-bench \
      /usr/bin/time -v -o "/results/$name.resource.txt" /app/t3box-ci-encode \
      /fixture/input.yuv "/results/$name.h264" 120 "$bitrate" "$preset" "/results/$name.aac" \
      > "$out/$name.performance.json"
    docker inspect "t3box-bench-$name" > "$out/$name.container.json"
    docker rm "t3box-bench-$name" >/dev/null
    ffprobe -v error -count_frames -show_streams -of json "$out/$name.h264" > "$out/$name.video.json"
    ffprobe -v error -show_streams -of json "$out/$name.aac" > "$out/$name.audio.json"
    ffmpeg -v error -xerror -i "$out/$name.aac" -f null -
    ffmpeg -v info -xerror -r 30 -i "$out/$name.h264" -i "$work/ref.mkv" \
      -lavfi '[0:v]settb=AVTB,setpts=N/30/TB[a];[1:v]settb=AVTB,setpts=N/30/TB[b];[a][b]psnr' -f null - 2>"$out/$name.psnr.log"
    ffmpeg -v info -xerror -r 30 -i "$out/$name.h264" -i "$work/ref.mkv" \
      -lavfi '[0:v]settb=AVTB,setpts=N/30/TB[a];[1:v]settb=AVTB,setpts=N/30/TB[b];[a][b]ssim' -f null - 2>"$out/$name.ssim.log"
    ffmpeg -v error -y -i "$out/$name.h264" -frames:v 1 "$out/$name.preview.png"
  done
done
python3 ci/report-benchmark.py "$out"
