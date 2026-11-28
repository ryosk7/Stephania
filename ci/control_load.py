#!/usr/bin/env python3
"""Closed-loop load against the control plane (docs/PERFORMANCE.md).

N keep-alive clients issue a Studio-like mix for a fixed time while the
maximum number of SSE streams is held open. Prints one JSON object with
throughput, latency percentiles per route, errors and stream health.
Usage: control_load.py <base-url> <token> <seconds> <clients> <label> [streams]
Streams default to the maximum (8); the warm-up passes 0 because an
abandoned stream keeps its slot until the next 15 s keepalive write fails.
One credential may hold only a few streams, so each stream uses its own
enrolled token (STREAM_TOKENS; ci/control-load.sh writes their hashes).
"""
import http.client, json, random, socket, statistics, sys, threading, time
from urllib.parse import urlparse

base, token, seconds, clients, label = sys.argv[1], sys.argv[2], float(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
u = urlparse(base)
H = {"Authorization": "Bearer " + token, "Content-Type": "application/json", "Host": "localhost"}
MIX = [("GET", "/api/v1/status", 40), ("GET", "/api/v1/scenes", 25), ("GET", "/api/v1/audio", 10),
       ("GET", "/api/v1/capabilities", 10), ("PUT", "/api/v1/scenes/load-{n}", 15)]
WEIGHTS = [w for *_, w in MIX]
SCENE = {"version": 2, "name": "load", "layers": [{"id": "h", "type": "video", "source": "hdmi", "x": 0, "y": 0, "width": 1920, "height": 1080, "visible": True}]}


def conn():
    return http.client.HTTPConnection(u.hostname, u.port, timeout=10)


def call(c, method, path, body=None):
    c.request(method, path, body=None if body is None else json.dumps(body), headers=H)
    r = c.getresponse()
    data = r.read()
    return r.status, data


# Seed one scene per client so PUTs exercise optimistic revisions without
# conflicts between clients.
c = conn()
revisions = {}
for n in range(clients):
    s, d = call(c, "PUT", f"/api/v1/scenes/load-{n}", {"revision": 0, "document": SCENE})
    if s == 409:
        s, d = call(c, "GET", f"/api/v1/scenes/load-{n}")
    revisions[n] = json.loads(d)["revision"]
c.close()

# Hold the maximum number of event streams; one more must be refused.
STREAM_TOKENS = ["t3b_" + f"{i:02x}" * 32 for i in range(8)]
streams, stream_alive = [], []
def stream(i):
    s = socket.create_connection((u.hostname, u.port))
    s.sendall(f"GET /api/v1/events HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {STREAM_TOKENS[i]}\r\nAccept: text/event-stream\r\n\r\n".encode())
    s.settimeout(None)
    first = s.recv(4096)
    stream_heads[i] = first[:60]
    stream_alive[i] = b" 200 " in first.split(b"\r\n", 1)[0]
    try:
        while s.recv(4096):
            pass
        stream_end[i] = "closed by server"
    except OSError as e:
        stream_end[i] = repr(e)
    if not stop.is_set():
        stream_alive[i] = False

stop = threading.Event()
MAX_STREAMS = int(sys.argv[6]) if len(sys.argv) > 6 else 8
stream_alive = [False] * MAX_STREAMS
stream_heads = [b""] * MAX_STREAMS
stream_end = [None] * MAX_STREAMS
threads = [threading.Thread(target=stream, args=(i,), daemon=True) for i in range(MAX_STREAMS)]
for t in threads:
    t.start()
time.sleep(1.0)
extra = None
if MAX_STREAMS == 8:
    c = conn()
    extra, _ = call(c, "GET", "/api/v1/events")
    c.close()

lat = {}
errors = []
lock = threading.Lock()
def worker(n):
    c = conn()
    rev = revisions[n]
    rnd = random.Random(n)
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        method, path, _ = rnd.choices(MIX, WEIGHTS)[0]
        key = path.replace("{n}", "n")
        body = None
        if method == "PUT":
            path = path.format(n=n)
            body = {"revision": rev, "document": SCENE}
        t0 = time.perf_counter()
        try:
            s, d = call(c, method, path, body)
        except Exception as e:  # connection reset etc.
            with lock:
                errors.append(f"{method} {key}: {e}")
            c.close(); c = conn()
            continue
        dt = (time.perf_counter() - t0) * 1000
        if method == "PUT" and s == 200:
            rev = json.loads(d)["revision"]
        with lock:
            lat.setdefault(f"{method} {key}", []).append(dt)
            if s >= 400:
                errors.append(f"{method} {key}: HTTP {s} {d[:120]!r}")
    c.close()

t0 = time.monotonic()
ws = [threading.Thread(target=worker, args=(n,)) for n in range(clients)]
for w in ws:
    w.start()
for w in ws:
    w.join()
elapsed = time.monotonic() - t0
alive = sum(stream_alive)
stop.set()

def pct(xs, p):
    xs = sorted(xs)
    return round(xs[min(len(xs) - 1, int(len(xs) * p / 100))], 2) if xs else None

allv = [x for v in lat.values() for x in v]
print(json.dumps({
    "label": label, "clients": clients, "seconds": round(elapsed, 1), "requests": len(allv),
    "rps": round(len(allv) / elapsed, 1),
    "p50_ms": pct(allv, 50), "p95_ms": pct(allv, 95), "p99_ms": pct(allv, 99), "max_ms": round(max(allv), 2) if allv else None,
    "routes": {k: {"n": len(v), "p50_ms": pct(v, 50), "p99_ms": pct(v, 99)} for k, v in sorted(lat.items())},
    "errors": len(errors), "error_samples": errors[:5],
    "streams_open": alive, "extra_stream_status": extra,
    "stream_heads": sorted({h.decode("latin-1") for h in stream_heads}), "stream_ends": sorted({str(e) for e in stream_end}),
}))
