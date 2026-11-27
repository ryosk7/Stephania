"""Drive the program audio mix over the engine IPC during a CI broadcast.

Timeline (virtual capture with STEPHANIA_CI_AUDIO_TONES: hdmi = 440 Hz stereo,
mic = 880 Hz mono, both at -12 dBFS peak):
  A  4 s  hdmi only
  B  4 s  mic enabled          -> both tones
  C  4 s  mic muted            -> 440 only
  D  4 s  hdmi gain -12 dB     -> 440 at a quarter of its amplitude
Checks live status and levels at each step; audio_check.py verifies the
decoded broadcast.
"""
import json, socket, struct, sys, time

path = sys.argv[1]
n = 0


def call(method, params=None):
    global n
    n += 1
    req = {"version": 1, "request_id": f"a{n}", "method": method}
    if params is not None:
        req["params"] = params
    body = json.dumps(req).encode()
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(10)
    s.connect(path)
    s.sendall(struct.pack(">I", len(body)) + body)
    size = struct.unpack(">I", s.recv(4))[0]
    data = b""
    while len(data) < size:
        data += s.recv(size - len(data))
    s.close()
    r = json.loads(data)
    return r


def check(cond, msg):
    print(("PASS " if cond else "FAIL ") + msg, flush=True)
    if not cond:
        raise SystemExit(1)


def status():
    return call("audio.status")["result"]


def inp(st, i):
    return next(x for x in st["inputs"] if x["id"] == i)


def configure(*inputs):
    r = call("audio.configure", {"inputs": list(inputs)})
    check(r["ok"], f"audio.configure {inputs}")
    return r["result"]


st = status()
check(not st["running"] and inp(st, "hdmi")["state"] == "idle" and inp(st, "mic")["state"] == "disabled", "idle before streaming")
r = call("audio.configure", {"inputs": [{"id": "mic", "enabled": True, "device": "hw:0;reboot"}]})
check(not r["ok"] and r["error"]["code"] == "invalid_params", "unsafe device name refused")
check(call("stream.start")["ok"], "stream.start")
deadline = time.time() + 10
while time.time() < deadline:
    st = status()
    if st["running"] and inp(st, "hdmi")["state"] == "active" and "levels" in inp(st, "hdmi"):
        break
    time.sleep(0.1)
check(inp(st, "hdmi")["state"] == "active", f"hdmi active {inp(st, 'hdmi')}")
time.sleep(4.0)

configure({"id": "mic", "enabled": True, "device": "virtual-mic", "channels": 1})
time.sleep(1.0)
st = status()
h, m = inp(st, "hdmi"), inp(st, "mic")
check(m["state"] == "active", f"mic active {m}")
check(abs(h["levels"]["peak_dbfs"] + 12) < 1.5, f"hdmi peak ≈ -12 dBFS ({h['levels']['peak_dbfs']})")
check(abs(m["levels"]["peak_dbfs"] + 12) < 1.5, f"mic peak ≈ -12 dBFS ({m['levels']['peak_dbfs']})")
check(-8 < st["master"]["peak_dbfs"] < -4.5, f"program peak is the sum ({st['master']['peak_dbfs']})")
check(not st["master"]["clipped"] and st["resyncs"] == 0, "no clipping, no resync")
time.sleep(3.0)

configure({"id": "mic", "muted": True})
time.sleep(1.0)
st = status()
check(inp(st, "mic")["muted"] and inp(st, "mic")["levels"]["peak_dbfs"] < -60, f"mic muted live ({inp(st, 'mic')['levels']})")
time.sleep(3.0)

configure({"id": "hdmi", "gain_db": -12.0})
time.sleep(1.0)
st = status()
check(abs(inp(st, "hdmi")["levels"]["peak_dbfs"] + 24) < 1.5, f"hdmi gain applied live ({inp(st, 'hdmi')['levels']['peak_dbfs']})")
time.sleep(3.0)
call("stream.stop")
time.sleep(0.5)
check(not status()["running"], "not running after stop")
