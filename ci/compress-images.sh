#!/usr/bin/env bash
# Compress PNG and JPEG files in place, or check that they are compressed.
#   ci/compress-images.sh FILE...          compress (used by .githooks/pre-commit)
#   ci/compress-images.sh --check FILE...  fail if a file is not compressed
#                                          (CI guard; saves > 3% and 2 KiB)
# Already-palette PNGs are only re-optimised losslessly, so running this again
# on a file it already compressed never adds generation loss.
# PNG: pngquant (lossy palette, quality floor 80) then optipng (lossless).
# JPEG: jpegoptim, quality cap 88, metadata stripped.
set -euo pipefail
check=0
if [ "${1:-}" = --check ]; then check=1; shift; fi
need() { command -v "$1" >/dev/null || { echo "compress-images: $1 is not installed (apt install pngquant optipng jpegoptim / brew install pngquant optipng jpegoptim)" >&2; exit 2; }; }

is_palette() { [ "$(od -An -tu1 -j25 -N1 -- "$1" | tr -d ' ')" = 3 ]; }

compress() { # file
  case "${1,,}" in
    *.png)
      need pngquant; need optipng
      if ! is_palette "$1"; then
        # 98: result would be larger, 99: quality floor not reached; keep the original.
        rc=0; pngquant --quality=80-98 --skip-if-larger --strip --force --ext .png -- "$1" || rc=$?
        case "$rc" in 0|98|99) ;; *) exit "$rc" ;; esac
      fi
      optipng -quiet -fix -o2 -strip all -- "$1" ;;
    *.jpg|*.jpeg)
      need jpegoptim
      jpegoptim --quiet --strip-all --max=88 -- "$1" ;;
  esac
}

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
fail=0
for f in "$@"; do
  [ -f "$f" ] || continue
  case "${f,,}" in *.png|*.jpg|*.jpeg) ;; *) continue ;; esac
  before=$(wc -c <"$f")
  if [ "$check" = 1 ]; then
    t="$tmp/$(basename "$f")"; cp -- "$f" "$t"; compress "$t"; after=$(wc -c <"$t")
    saved=$((before - after))
    if [ "$saved" -gt 2048 ] && [ $((saved * 100)) -gt $((before * 3)) ]; then
      echo "::error file=$f::not compressed ($before -> $after bytes); run ci/compress-images.sh $f"; fail=1
    fi
  else
    compress "$f"; after=$(wc -c <"$f")
    if [ "$after" -lt "$before" ]; then echo "compressed $f: $before -> $after bytes"; fi
  fi
done
exit "$fail"
