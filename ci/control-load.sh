#!/usr/bin/env bash
# Control-plane load measurement (docs/PERFORMANCE.md): real Puma in
# production mode against the real engine, inside the §9 cgroup limits
# (CPUQuota=50%, MemoryHigh=96M, MemoryMax=128M) via systemd-run, with
# YJIT off and on. Writes <out>/control-load.json and a Markdown table.
# Usage (sudo-capable runner): ci/control-load.sh <t3box binary> [out-dir]
set -Eeuo pipefail
bin=$(realpath "${1:?t3box binary}")
out=$(realpath -m "${2:-control-load}")
mkdir -p "$out"
work=$(mktemp -d)
token=load-test-token-0123456789abcdef0123
port=9494
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; sudo systemctl stop t3box-load-puma.scope 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT
T3BOX_ENGINE_SOCKET="$work/engine.sock" T3BOX_OUTPUT="" "$bin" >"$out/engine.log" 2>&1 & pids+=($!)
for _ in $(seq 50); do [ -S "$work/engine.sock" ] && break; sleep 0.1; done

results=()
for yjit in 0 1; do
  rm -f "$work/control.db"*
  sudo systemd-run --scope --quiet --unit=t3box-load-puma -p CPUQuota=50% -p MemoryHigh=96M -p MemoryMax=128M \
    --setenv=RUBY_YJIT_ENABLE=$yjit --setenv=RACK_ENV=production --setenv=T3BOX_CONTROL_TOKEN=$token \
    --setenv=T3BOX_CONTROL_PORT=$port --setenv=T3BOX_ENGINE_SOCKET="$work/engine.sock" --setenv=T3BOX_CONTROL_DB="$work/control.db" \
    --setenv=PATH="$PATH" --setenv=HOME="$HOME" --setenv=GEM_HOME="${GEM_HOME:-}" --setenv=BUNDLE_GEMFILE="$PWD/services/control/Gemfile" \
    --uid="$(id -u)" --gid="$(id -g)" -- bash -c "cd services/control && exec bundle exec puma -C config/puma.rb config.ru" >"$out/puma-yjit$yjit.log" 2>&1 &
  pids+=($!)
  for _ in $(seq 100); do curl -fs -o /dev/null -H "Authorization: Bearer $token" "http://127.0.0.1:$port/api/v1/capabilities" && break; sleep 0.2; done
  # Warm-up, then the measured run.
  python3 ci/control_load.py "http://127.0.0.1:$port" "$token" 3 4 warmup >/dev/null
  r=$(python3 ci/control_load.py "http://127.0.0.1:$port" "$token" 20 4 "yjit=$yjit")
  mem=$(cat /sys/fs/cgroup/system.slice/t3box-load-puma.scope/memory.peak 2>/dev/null || cat /sys/fs/cgroup/system.slice/t3box-load-puma.scope/memory.current 2>/dev/null || echo 0)
  r=$(echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); d['memory_peak_mib']=round(int(sys.argv[1])/1048576,1); print(json.dumps(d))" "$mem")
  echo "$r"
  results+=("$r")
  sudo systemctl stop t3box-load-puma.scope
  sleep 1
done
printf '%s\n' "${results[@]}" | python3 ci/control_load_report.py "$out"
