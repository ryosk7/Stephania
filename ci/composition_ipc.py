"""Drive the engine's control socket during a composed CI broadcast.

Timeline: apply before streaming (must fail: program_inactive) -> start ->
2 s HDMI only -> apply PIP (phone:rear top-right) -> 2 s -> apply HDMI full
screen (fast path) -> 1 s -> fade (500 ms) to a scene v2 with an image asset
in the top-left quadrant -> 1.5 s -> stop. Writes the acknowledged frame
indexes for composition_check.py.
Usage: composition_ipc.py <engine.sock> <acks.json> <assets dir>
"""
import hashlib, json, os, socket, struct, sys, time, zlib

path, out, assets = sys.argv[1], sys.argv[2], sys.argv[3]
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


def png(width, height, rgba):
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    raw = (b"\0" + bytes(rgba) * width) * height
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


# Opaque blue image asset, stored the way the control plane stores uploads.
BLUE = png(16, 16, (0, 0, 255, 255))
BLUE_ID = hashlib.sha256(BLUE).hexdigest()[:16]
os.makedirs(assets, exist_ok=True)
open(os.path.join(assets, BLUE_ID + ".png"), "wb").write(BLUE)
BRAND = {"version": 2, "name": "brand", "layers": [
    {"id": "slides", "type": "video", "source": "hdmi", "x": 0, "y": 0, "width": 1920, "height": 1080, "visible": True},
    {"id": "logo", "type": "image", "asset": BLUE_ID, "x": 0, "y": 0, "width": 960, "height": 540, "visible": True}]}

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
time.sleep(1.0)
missing = dict(BRAND, layers=[BRAND["layers"][0], dict(BRAND["layers"][1], asset="0" * 16)])
r = call("scene.apply", {"scene": missing, "scene_id": "brand", "revision": 9})
check(r["ok"] is False and r["error"]["code"] == "invalid_scene" and "asset_unavailable" in r["error"]["message"], f"missing asset fails closed {r.get('error')}")
r = call("scene.apply", {"scene": BRAND, "scene_id": "brand", "revision": 9, "transition": {"type": "fade", "duration_ms": 500}})
check(r["ok"] and r["result"]["revision"] == 9 and r["result"]["transition"] == {"type": "fade", "duration_ms": 500}, f"v2 scene faded in {r}")
brand_frame = r["result"]["first_frame_index"]
time.sleep(1.5)
call("stream.stop")
json.dump({"pip_frame": pip_frame, "full_frame": full_frame, "brand_frame": brand_frame, "fade_frames": 15}, open(out, "w"))
print(f"acks: pip at frame {pip_frame}, full at frame {full_frame}, brand at frame {brand_frame}", flush=True)
