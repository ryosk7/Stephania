#!/usr/bin/env bash
set -Eeuo pipefail
mkdir -p "${1:-ci-results}"
# Refuse stale evidence instead of accepting old completion markers.
if [[ -n "$(ls -A "${1:-ci-results}")" ]]; then echo "Output directory must be empty" >&2; exit 2; fi
out="$(realpath "${1:-ci-results}")"
work="$(mktemp -d)"
chmod 755 "$work"
cleanup() {
  for name in "${expected[@]:-}"; do docker rm -f "stephania-bench-$name" >/dev/null 2>&1 || true; done
  rm -rf "$work"
}
containers=()
trap cleanup EXIT
trap 'exit 124' TERM INT
export LC_ALL=C
read -ra presets <<< "${STEPHANIA_BENCH_PRESET:-ultrafast veryfast}"
read -ra rates <<< "${STEPHANIA_BENCH_BITRATE:-6000 8000}"
expected=()
for preset in "${presets[@]}"; do
  for bitrate in "${rates[@]}"; do expected+=("$preset-$bitrate"); done
done
printf '%s\n' "${expected[@]}" > "$out/expected-profiles.txt"
# Generate 4 seconds of moving content and small text. Fixture preparation
# and objective decoding are outside the measured encoder resource budget.
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=1920x1080:rate=30' \
  -vf "drawbox=x=50:y=50:w=1820:h=450:color=white:t=fill,drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='Stephania LT  fn main  1234567890':fontsize=22:fontcolor=black:x=75:y=90,drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf:text='Small code  0123456789  ABCDEFGHIJKLMNOPQRSTUVWXYZ':fontsize=14:fontcolor=black:x=75:y=160" \
  -frames:v 120 -threads 1 -pix_fmt yuyv422 -f rawvideo "$work/input.yuv"
ffmpeg -v error -y -f rawvideo -pixel_format yuyv422 -video_size 1920x1080 -framerate 30 -i "$work/input.yuv" -pix_fmt yuv420p -threads 1 -f rawvideo "$work/ref.yuv"
lscpu > "$out/cpu.txt"
free -b > "$out/host-memory.txt"
uname -a > "$out/kernel.txt"
ffmpeg -version > "$out/ffmpeg.txt"
cpus="$(nproc)"
if (( cpus > 4 )); then cpus=4; fi
printf "%s\n" "$cpus" > "$out/cpu-quota.txt"
if [[ "${STEPHANIA_BENCH_PREBUILT:-0}" != 1 ]]; then bash ci/images/benchmark.sh; fi
run_profile() {
    local preset="$1" bitrate="$2" name="$1-$2"
    containers+=("stephania-bench-$name")
    docker run --name "stephania-bench-$name" --cpus="$cpus" --memory=4g --memory-swap=4g \
      --network=none --user "$(id -u):$(id -g)" -v "$work:/fixture:ro" -v "$out:/results" stephania-bench \
      /usr/bin/time -v -o "/results/$name.resource.txt" /app/stephania-ci-encode \
      /fixture/input.yuv "/results/$name.h264" 120 "$bitrate" "$preset" "/results/$name.aac" \
      > "$out/$name.performance.json" 2>"$out/$name.encoder.log"
    docker inspect "stephania-bench-$name" > "$out/$name.container.json"
    docker rm "stephania-bench-$name" >/dev/null
    ffprobe -v error -count_frames -show_streams -of json "$out/$name.h264" > "$out/$name.video.json"
    ffprobe -v error -count_frames -show_streams -of json "$out/$name.aac" > "$out/$name.audio.json"
    ffmpeg -v error -xerror -i "$out/$name.aac" -f null -
    ffmpeg -v error -xerror -i "$out/$name.aac" -f s16le -acodec pcm_s16le -y "$out/$name.decoded.pcm"
    # Decode once for both full-reference metrics and all preview/OCR samples.
    ffmpeg -v info -xerror -r 30 -i "$out/$name.h264" -f rawvideo -pixel_format yuv420p -video_size 1920x1080 -framerate 30 -i "$work/ref.yuv" \
      -filter_complex '[0:v]settb=AVTB,setpts=N/30/TB,split=3[a][c][p];[1:v]settb=AVTB,setpts=N/30/TB,split=2[b][d];[a][b]psnr=stats_file='"$out/$name.psnr-frames.txt"'[ps];[c][d]ssim=stats_file='"$out/$name.ssim-frames.txt"'[ss];[p]select=not(mod(n\,30)),setpts=N/30/TB[preview]' \
      -map '[ps]' -map '[ss]' -f null - -map '[preview]' -fps_mode passthrough -threads 1 -y "$out/$name.preview-%d.png" 2>"$out/$name.metrics.log"
    for ts in 0 1 2 3; do
      ffmpeg -v error -y -i "$out/$name.preview-$((ts+1)).png" -frames:v 1 -vf "crop=1500:65:60:145,scale=3000:130" "$out/$name.text-$ts.png"
      tesseract "$out/$name.text-$ts.png" stdout --psm 7 2>/dev/null > "$out/$name.text-$ts.txt"
    done
    if [[ "${STEPHANIA_BENCH_VMAF:-0}" == 1 ]] && ffmpeg -hide_banner -filters 2>/dev/null | grep -w libvmaf >/dev/null; then
      ffmpeg -v error -xerror -r 30 -i "$out/$name.h264" -f rawvideo -pixel_format yuv420p -video_size 1920x1080 -framerate 30 -i "$work/ref.yuv" -lavfi "[0:v]settb=AVTB,setpts=N/30/TB[a];[1:v]settb=AVTB,setpts=N/30/TB[b];[a][b]libvmaf=log_fmt=json:log_path=$out/$name.vmaf.json" -f null -
    else
      echo "VMAF not requested or unavailable in runner FFmpeg" > "$out/$name.vmaf-unavailable.txt"
    fi
    touch "$out/$name.complete"
}
# Run each profile in its own strict shell. The parent collects every exit
# code, including failures, without Bash's conditional-errexit suppression.
export out work cpus
export -f run_profile
rc=0
for preset in "${presets[@]}"; do
  for bitrate in "${rates[@]}"; do
    name="$preset-$bitrate"
    if bash -Eeuo pipefail -c 'containers=(); run_profile "$@"' _ "$preset" "$bitrate"; then
      :
    else
      status=$?
      printf '%s\n' "$status" > "$out/$name.failed"
      rc=1
    fi
    docker rm -f "stephania-bench-$name" >/dev/null 2>&1 || true
  done
done
python3 ci/report-benchmark.py "$out" "${expected[@]}" || rc=1
exit "$rc"
