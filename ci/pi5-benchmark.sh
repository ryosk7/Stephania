#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p "${1:-ci-results}"
out="$(realpath "${1:-ci-results}")"
work="$(mktemp -d)"
chmod 755 "$work"
trap 'rm -rf "$work"' EXIT
# Generate 4 seconds of moving content and small text. Fixture preparation
# and objective decoding are outside the measured encoder resource budget.
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=1920x1080:rate=30' \
  -vf "drawbox=x=50:y=50:w=1820:h=450:color=white:t=fill,drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='T3Box LT  fn main  1234567890':fontsize=22:fontcolor=black:x=75:y=90,drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='Small code  0123456789  ABCDEFGHIJKLMNOPQRSTUVWXYZ':fontsize=14:fontcolor=black:x=75:y=160" \
  -frames:v 120 -threads 1 -pix_fmt yuyv422 -f rawvideo "$work/input.yuv"
ffmpeg -v error -y -f rawvideo -pixel_format yuyv422 -video_size 1920x1080 -framerate 30 -i "$work/input.yuv" -pix_fmt yuv420p -c:v ffv1 "$work/ref.mkv"
lscpu > "$out/cpu.txt"
free -b > "$out/host-memory.txt"
uname -a > "$out/kernel.txt"
ffmpeg -version > "$out/ffmpeg.txt"
cpus="$(nproc)"
if (( cpus > 4 )); then cpus=4; fi
printf "%s\n" "$cpus" > "$out/cpu-quota.txt"
if [[ "${T3BOX_BENCH_PREBUILT:-0}" != 1 ]]; then docker build -t t3box-bench -f ci/Dockerfile.benchmark .; fi
for preset in ${T3BOX_BENCH_PRESET:-ultrafast veryfast}; do
  for bitrate in ${T3BOX_BENCH_BITRATE:-6000 8000}; do
    name="$preset-$bitrate"
    docker run --name "t3box-bench-$name" --cpus="$cpus" --memory=4g --memory-swap=4g \
      --network=none --user "$(id -u):$(id -g)" -v "$work:/fixture:ro" -v "$out:/results" t3box-bench \
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
    for ts in 0 1 2 3; do
      ffmpeg -v error -y -ss "$ts" -i "$out/$name.h264" -frames:v 1 -vf "crop=1500:65:60:145,scale=3000:130" "$out/$name.text-$ts.png"
      tesseract "$out/$name.text-$ts.png" stdout --psm 7 2>/dev/null > "$out/$name.text-$ts.txt"
    done
    if ffmpeg -hide_banner -filters 2>/dev/null | grep -w libvmaf >/dev/null; then
      ffmpeg -v error -xerror -r 30 -i "$out/$name.h264" -i "$work/ref.mkv" -lavfi "[0:v]settb=AVTB,setpts=N/30/TB[a];[1:v]settb=AVTB,setpts=N/30/TB[b];[a][b]libvmaf=log_fmt=json:log_path=$out/$name.vmaf.json" -f null -
    else
      echo "libvmaf unavailable in runner FFmpeg" > "$out/$name.vmaf-unavailable.txt"
    fi
  done
done
expected=4
if [[ -n "${T3BOX_BENCH_PRESET:-}" && -n "${T3BOX_BENCH_BITRATE:-}" ]]; then expected=1; fi
python3 ci/report-benchmark.py "$out" "$expected"
