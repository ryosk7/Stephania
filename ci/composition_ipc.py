"""Drive the engine's control socket during a composed CI broadcast.

Timeline: apply before streaming (must fail: program_inactive) -> start ->
2 s HDMI only -> apply PIP (phone:rear top-right) -> 2 s -> apply HDMI full
screen (fast path) -> 1.5 s -> stop. Writes the acknowledged frame indexes
for composition_check.py.
"""
import json, socket, struct, sys, time

path, out = sys.argv[1], sys.argv[2]
n = 0


def call(method, params=None):
    global n
    n += 1
    req = {"version": 1, "request_id": f"c{n}", "method": method}
    if params is not None:
        req["params"] = params
    body = json.dumps(req).encode()
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(10)
    s.connect(path)
    s.sendall(struct.pack(">I", len(body)) + body)
    hdr = s.recv(4)
    size = struct.unpack(">I", hdr)[0]
    data = b""
    while len(data) < size:
        data += s.recv(size - len(data))
    s.close()
    return json.loads(data)


def scene(*layers):
    return {"version": 1, "name": "ci", "layers": [
        {"id": i, "source": src, "x": x, "y": y, "width": w, "height": h, "visible": True} for i, src, x, y, w, h in layers]}


HDMI = ("slides", "hdmi", 0, 0, 1920, 1080)
PIP = scene(HDMI, ("rear", "phone:rear", 1440, 0, 480, 270))
FULL = scene(HDMI)


def check(cond, msg):
    print(("PASS " if cond else "FAIL ") + msg, flush=True)
    if not cond:
        raise SystemExit(1)


caps = call("engine.capabilities")
check(caps["result"]["program_apply"] is True, "engine reports program_apply (compositor on by default)")
r = call("scene.apply", {"scene": PIP, "scene_id": "pip", "revision": 7})
check(r["ok"] is False and r["error"]["code"] == "program_inactive", f"apply before streaming fails closed {r.get('error')}")
check(call("stream.start")["ok"], "stream.start")
time.sleep(2.0)
r = call("scene.apply", {"scene": PIP, "scene_id": "pip", "revision": 7})
check(r["ok"] and r["result"]["applied"] and r["result"]["revision"] == 7, f"PIP applied with ack {r}")
pip_frame = r["result"]["first_frame_index"]
st = call("engine.status")["result"]
check(st["program"] == {"scene_id": "pip", "revision": 7}, f"status reports Program on air {st['program']}")
time.sleep(2.0)
r = call("scene.apply", {"scene": FULL, "scene_id": "full", "revision": 8})
check(r["ok"] and r["result"]["revision"] == 8, f"HDMI full screen applied {r}")
full_frame = r["result"]["first_frame_index"]
time.sleep(1.5)
call("stream.stop")
json.dump({"pip_frame": pip_frame, "full_frame": full_frame}, open(out, "w"))
print(f"acks: pip at frame {pip_frame}, full at frame {full_frame}", flush=True)
