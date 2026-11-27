"""Verify a composed broadcast pixel-wise after the H.264 round trip.

The virtual HDMI source is neutral grey (U=V=128); the virtual phone track is
strongly coloured (U=212, V=234). In the phone layer's region the chroma must
match the phone only between the PIP and full-screen acknowledgements, and the
HDMI region must never show the phone.
Usage: composition_check.py <flv> <acks.json> <width> <height>
"""
import json, subprocess, sys

flv, acks_path, w, h = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
acks = json.load(open(acks_path))
raw = subprocess.run(["ffmpeg", "-v", "error", "-i", flv, "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"],
                     check=True, capture_output=True).stdout
fs = w * h * 3 // 2
frames = [raw[i:i + fs] for i in range(0, len(raw) - fs + 1, fs)]
cw, ch = w // 2, h // 2


def chroma_mean(f, x0, y0, x1, y1):
    u_off, v_off = w * h, w * h + cw * ch
    us = vs = n = 0
    for y in range(y0 // 2, y1 // 2):
        for x in range(x0 // 2, x1 // 2):
            us += f[u_off + y * cw + x]; vs += f[v_off + y * cw + x]; n += 1
    return us / n, vs / n


# PIP layer in output pixels (scene 1440,0 480x270 on 1920x1080), inset by 8 px
# to stay clear of codec ringing at the edges.
px0, py0, px1, py1 = w * 1440 // 1920 + 8, 8, w - 8, h * 270 // 1080 - 8
hx0, hy0, hx1, hy1 = 8, h // 2, w // 2, h - 8  # always HDMI
phone = []
for i, f in enumerate(frames):
    u, v = chroma_mean(f, px0, py0, px1, py1)
    hu, hv = chroma_mean(f, hx0, hy0, hx1, hy1)
    is_phone = abs(u - 212) < 16 and abs(v - 234) < 16
    is_grey = abs(u - 128) < 8 and abs(v - 128) < 8
    phone.append(is_phone)
    if not (abs(hu - 128) < 8 and abs(hv - 128) < 8):
        sys.exit(f"FAIL frame {i}: HDMI region chroma {hu:.0f},{hv:.0f} is not the HDMI source")
    if not (is_phone or is_grey):
        sys.exit(f"FAIL frame {i}: PIP region chroma {u:.0f},{v:.0f} is neither phone nor HDMI")

on = [i for i, p in enumerate(phone) if p]
print(f"frames={len(frames)} phone_frames={len(on)} first={on[:1]} last={on[-1:]} acks={acks}")
if not on:
    sys.exit("FAIL phone layer never appeared in Program")
if on != list(range(on[0], on[-1] + 1)):
    sys.exit("FAIL phone layer not contiguous")
# Every encoded frame passes the composition stage exactly once, so decoded
# frame i is stage frame i: the acknowledgement must be frame-exact.
if on[0] != acks["pip_frame"] or on[-1] != acks["full_frame"] - 1:
    sys.exit(f"FAIL phone frames {on[0]}..{on[-1]} do not match acknowledged {acks['pip_frame']}..{acks['full_frame']}")
if on[0] == 0 or on[-1] == len(frames) - 1:
    sys.exit("FAIL expected HDMI-only frames before and after the PIP section")
print("PASS composed Program matches acknowledgements frame-accurately; HDMI before/after (fast path)")

# Scene v2 image layer (opaque blue, BT.709 limited U=240 V=118) faded in over
# 500 ms in the top-left quadrant: grey before the ack, rising during the
# fade, blue after it.
tx0, ty0, tx1, ty1 = 8, 8, w // 2 - 8, h // 2 - 8
b, fade = acks["brand_frame"], acks["fade_frames"]
tl = [chroma_mean(f, tx0, ty0, tx1, ty1)[0] for f in frames]
if not all(abs(u - 128) < 8 for u in tl[:b]):
    sys.exit("FAIL top-left changed before the v2 scene was acknowledged")
after = tl[b + fade + 1:]
if not after or not all(abs(u - 240) < 10 for u in after):
    sys.exit(f"FAIL image layer not shown after the fade: {[round(u) for u in after[:5]]}")
ramp = tl[b:b + fade]
# The acknowledged frame carries the smallest non-zero weight (1/255), so the
# rise is visible from the next frame on.
if not (tl[b + 1] > tl[b] + 3 and all(y >= x - 3 for x, y in zip(ramp, ramp[1:])) and 150 < ramp[len(ramp) // 2] < 220):
    sys.exit(f"FAIL fade is not a gradual rise from the acknowledged frame: {[round(u) for u in ramp]}")
print(f"PASS v2 image asset faded in from frame {b} over {fade} frames: {[round(u) for u in ramp]}")
