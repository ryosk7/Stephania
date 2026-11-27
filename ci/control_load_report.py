#!/usr/bin/env python3
"""Table and gate for ci/control-load.sh. Usage: control_load_report.py <out-dir> < results.jsonl"""
import json, sys

out = sys.argv[1]
rows = [json.loads(l) for l in sys.stdin if l.strip()]
json.dump(rows, open(f"{out}/control-load.json", "w"), indent=1)
md = ["| Run | req/s | p50 ms | p95 ms | p99 ms | max ms | errors | SSE held | 9th stream | memory peak MiB |",
      "|---|---|---|---|---|---|---|---|---|---|"]
for r in rows:
    md.append("| {label} | {rps} | {p50_ms} | {p95_ms} | {p99_ms} | {max_ms} | {errors} | {streams_open}/8 | {extra_stream_status} | {memory_peak_mib} |".format(**r))
open(f"{out}/control-load.md", "w").write("\n".join(md) + "\n")
print("\n".join(md))
bad = [r["label"] for r in rows if r["errors"] or r["streams_open"] != 8 or r["extra_stream_status"] != 429 or r["p99_ms"] > 250]
if bad:
    sys.exit("FAIL load gate: " + ", ".join(bad))
print("PASS no errors, 8 streams held, 9th refused with 429, p99 < 250 ms within the section 9 cgroup")
