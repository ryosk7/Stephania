#!/usr/bin/env bash
# Control-plane load measurement (docs/PERFORMANCE.md): real Puma in
# production mode against the real engine, inside the §9 cgroup limits
# (CPUQuota=50%, MemoryHigh=96M, MemoryMax=128M) via systemd-run, with
# YJIT off and on. Writes <out>/control-load.json and a Markdown table.
# Usage (sudo-capable runner): ci/control-load.sh <stephania binary> [out-dir]
set -Eeuo pipefail
bin=$(realpath "${1:?stephania binary}")
out=$(realpath -m "${2:-control-load}")
mkdir -p "$out"
work=$(mktemp -d)
token=load-test-token-0123456789abcdef0123
port=9494
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; sudo systemctl stop stephania-load-puma.scope 2>/dev/null || true; wait 2>/dev/null || true; }
diag() { echo "--- engine log"; tail -30 "$out/engine.log" 2>/dev/null; for f in "$out"/puma-yjit*.log; do echo "--- $f"; tail -30 "$f"; done; ls -la "$work"; }
trap 'rc=$?; [ "$rc" = 0 ] || diag; cleanup' EXIT
STEPHANIA_ENGINE_SOCKET="$work/engine.sock" STEPHANIA_OUTPUT="" "$bin" >"$out/engine.log" 2>&1 & pids+=($!)
for _ in $(seq 100); do [ -S "$work/engine.sock" ] && break; sleep 0.1; done
[ -S "$work/engine.sock" ] || { echo "engine did not create its IPC socket"; exit 1; }

# Enrolled tokens for the event streams (ci/control_load.py STREAM_TOKENS):
# one credential may hold only a few streams.
python3 -c 'import hashlib,json,sys; json.dump({"tokens": [{"id": f"load{i}", "sha256": hashlib.sha256(("t3b_" + f"{i:02x}" * 32).encode()).hexdigest()} for i in range(8)]}, open(sys.argv[1], "w"))' "$work/tokens.json"

results=()
for yjit in 0 1; do
  rm -f "$work/control.db"*
  sudo systemd-run --scope --quiet --unit=stephania-load-puma -p CPUQuota=50% -p MemoryHigh=96M -p MemoryMax=128M \
    --setenv=RUBY_YJIT_ENABLE=$yjit --setenv=RACK_ENV=production --setenv=STEPHANIA_CONTROL_TOKEN=$token \
    --setenv=STEPHANIA_CONTROL_PORT=$port --setenv=STEPHANIA_ENGINE_SOCKET="$work/engine.sock" --setenv=STEPHANIA_CONTROL_DB="$work/control.db" \
    --setenv=STEPHANIA_CONTROL_TOKENS_FILE="$work/tokens.json" \
    --setenv=PATH="$PATH" --setenv=HOME="$HOME" --setenv=GEM_HOME="${GEM_HOME:-}" --setenv=BUNDLE_GEMFILE="$PWD/services/control/Gemfile" \
    --uid="$(id -u)" --gid="$(id -g)" -- bash -c "cd services/control && exec bundle exec puma -C config/puma.rb config.ru" >"$out/puma-yjit$yjit.log" 2>&1 &
  pids+=($!)
  for _ in $(seq 100); do curl -fs -o /dev/null -H "Authorization: Bearer $token" "http://127.0.0.1:$port/api/v1/capabilities" && break; sleep 0.2; done
  echo "status probe: $(curl -s -H "Authorization: Bearer $token" "http://127.0.0.1:$port/api/v1/status")"
  kill -0 "${pids[0]}" || { echo "engine exited"; exit 1; }
  # Warm-up, then the measured run.
  python3 ci/control_load.py "http://127.0.0.1:$port" "$token" 3 4 warmup 0 >/dev/null
  r=$(python3 ci/control_load.py "http://127.0.0.1:$port" "$token" 20 4 "yjit=$yjit")
  mem=$(cat /sys/fs/cgroup/system.slice/stephania-load-puma.scope/memory.peak 2>/dev/null || cat /sys/fs/cgroup/system.slice/stephania-load-puma.scope/memory.current 2>/dev/null || echo 0)
  r=$(echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); d['memory_peak_mib']=round(int(sys.argv[1])/1048576,1); print(json.dumps(d))" "$mem")
  echo "$r"
  results+=("$r")
  sudo systemctl stop stephania-load-puma.scope
  sleep 1
done
printf '%s\n' "${results[@]}" | python3 ci/control_load_report.py "$out" 
