"""Phone stand-in: sends two H.264 camera tracks over WebRTC (aiortc) via the
control plane's signaling endpoint, then checks the engine sees both
sources with the expected content (docs/PHONE.md).

Usage: sender.py <control-base-url> <token> [seconds]
"""
import asyncio
import fractions
import json
import sys
import time
import urllib.request

import numpy as np
from aiortc import RTCPeerConnection, RTCRtpSender, RTCSessionDescription
from aiortc.mediastreams import VideoStreamTrack
from av import VideoFrame

base, token = sys.argv[1].rstrip("/"), sys.argv[2]
seconds = float(sys.argv[3]) if len(sys.argv) > 3 else 8.0


def check(cond, message):
    print(("PASS " if cond else "FAIL ") + message, flush=True)
    if not cond:
        raise SystemExit(1)


def api(method, path, body=None):
    req = urllib.request.Request(base + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"Authorization": "Bearer " + token, "Content-Type": "application/json", "Host": "localhost"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            raw = r.read()
            return r.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"null")


class Solid(VideoStreamTrack):
    """640x360 30 fps solid luma (and a moving bar so frames differ)."""

    def __init__(self, luma):
        super().__init__()
        self.luma = luma
        self.n = 0

    async def recv(self):
        pts, time_base = await self.next_timestamp()
        img = np.full((360 * 3 // 2, 640), 128, dtype=np.uint8)
        img[:360, :] = self.luma
        x = (self.n * 8) % 600
        img[:360, x:x + 16] = min(255, self.luma + 30)
        self.n += 1
        frame = VideoFrame.from_ndarray(img, format="yuv420p")
        frame.pts, frame.time_base = pts, time_base
        return frame


async def main():
    pc = RTCPeerConnection()
    rear, front = Solid(200), Solid(60)
    tracks = {}
    for t, role in ((rear, "rear"), (front, "front")):
        tr = pc.addTransceiver(t, direction="sendonly")
        h264 = [c for c in RTCRtpSender.getCapabilities("video").codecs if c.mimeType == "video/H264"]
        tr.setCodecPreferences(h264)
        tracks[t.id] = role
    await pc.setLocalDescription(await pc.createOffer())

    status, body = api("POST", "/api/v1/cameras/session/offer", {"sdp": pc.localDescription.sdp, "tracks": tracks})
    check(status == 200 and "sdp" in body, f"device answered the offer ({status})")
    await pc.setRemoteDescription(RTCSessionDescription(sdp=body["sdp"], type="answer"))

    status, again = api("POST", "/api/v1/cameras/session/offer", {"sdp": pc.localDescription.sdp, "tracks": tracks})
    check(status == 409 and again["error"]["code"] == "busy", f"second phone refused while the first is active ({status})")

    deadline = time.time() + 20
    while pc.connectionState != "connected" and time.time() < deadline:
        await asyncio.sleep(0.2)
    check(pc.connectionState == "connected", f"ICE+DTLS connected ({pc.connectionState})")

    await asyncio.sleep(seconds)
    _, st = api("GET", "/api/v1/status")
    sources = {s["id"]: s for s in st.get("sources", [])}
    print("engine sources:", json.dumps(sources), flush=True)
    for name, luma in (("phone:rear", 200), ("phone:front", 60)):
        s = sources.get(name)
        check(s is not None and s["fresh"], f"{name} decoded and fresh in the engine")
        check((s["width"], s["height"]) == (640, 360), f"{name} geometry {s['width']}x{s['height']}")
        check(abs(s["mean_luma"] - luma) < 12, f"{name} content luma {s['mean_luma']} ≈ {luma} (tracks not swapped)")
    _, ps = api("GET", "/api/v1/cameras/session")
    print("phone session:", json.dumps(ps), flush=True)
    check(ps["state"] == "connected" and ps["access_units"].get("phone:rear", 0) > seconds * 15, "access units forwarded at frame rate")

    status, _ = api("DELETE", "/api/v1/cameras/session")
    check(status == 204, "session closed")
    await pc.close()
    await asyncio.sleep(1.5)
    _, st = api("GET", "/api/v1/status")
    stale = [s for s in st.get("sources", []) if s["id"].startswith("phone:") and s["fresh"]]
    check(not stale, "phone sources go stale after the session ends (layers fall back)")
    print("phone WebRTC e2e: all checks passed", flush=True)


asyncio.run(main())
