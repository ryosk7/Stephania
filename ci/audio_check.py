"""Verify the program audio of a recorded CI broadcast (see audio_ipc.py).

Decodes the AAC track, measures the 440 Hz (hdmi) and 880 Hz (mic) tones
in 250 ms windows (Goertzel) and requires the four phases in order, each
lasting at least 2.5 s, with the expected relative levels, and continuous
audio for the whole session.
Usage: audio_check.py <flv>
"""
import math, struct, subprocess, sys

flv = sys.argv[1]
pcm = subprocess.run(["ffmpeg", "-v", "error", "-i", flv, "-map", "0:a", "-ac", "1", "-ar", "48000", "-f", "s16le", "-"],
                     check=True, capture_output=True).stdout
samples = struct.unpack(f"<{len(pcm) // 2}h", pcm[: len(pcm) // 2 * 2])
WIN = 12000
dur = len(samples) / 48000
print(f"decoded audio: {dur:.1f} s")


def goertzel(x, f):
    k = 2 * math.cos(2 * math.pi * f / 48000)
    s1 = s2 = 0.0
    for v in x:
        s1, s2 = v + k * s1 - s2, s1
    p = s1 * s1 + s2 * s2 - k * s1 * s2
    return 2 * math.sqrt(max(p, 0)) / len(x)  # amplitude


wins = []
for i in range(0, len(samples) - WIN, WIN):
    x = samples[i:i + WIN]
    wins.append((goertzel(x, 440), goertzel(x, 880)))
# Classify each window: hdmi level (full / quarter / none) and mic on/off.
FULL = 8192.0


def cls(a440, a880):
    h = "full" if a440 > FULL * 0.6 else "quarter" if FULL * 0.15 < a440 < FULL * 0.4 else "none" if a440 < FULL * 0.05 else "?"
    m = "on" if a880 > FULL * 0.6 else "off" if a880 < FULL * 0.05 else "?"
    return h, m


labels = [cls(*w) for w in wins]
# Collapse into runs, ignoring single transitional windows.
runs = []
for lab in labels:
    if runs and runs[-1][0] == lab:
        runs[-1][1] += 1
    else:
        runs.append([lab, 1])
runs = [r for r in runs if r[1] >= 2 and "?" not in r[0]]
merged = []
for lab, cnt in runs:
    if merged and merged[-1][0] == lab:
        merged[-1][1] += cnt
    else:
        merged.append([lab, cnt])
print("phases:", [(f"{l[0]}/{l[1]}", round(c * WIN / 48000, 2)) for l, c in merged])
want = [("full", "off"), ("full", "on"), ("full", "off"), ("quarter", "off")]
seq = [tuple(l) for l, _ in merged]
if seq != want:
    sys.exit(f"FAIL phase sequence {seq} != {want}")
for (lab, cnt), name in zip(merged, "ABCD"):
    if cnt * WIN / 48000 < 2.5:
        sys.exit(f"FAIL phase {name} {lab} lasted only {cnt * WIN / 48000:.2f} s")
# Level relations (decoded AAC, ±1.5 dB): D is hdmi at -12 dB.
a = sum(w[0] for w, l in zip(wins, labels) if l == ("full", "off")) / max(1, sum(1 for l in labels if l == ("full", "off")))
d = sum(w[0] for w, l in zip(wins, labels) if l == ("quarter", "off")) / max(1, sum(1 for l in labels if l == ("quarter", "off")))
rel = 20 * math.log10(d / a)
print(f"hdmi gain step measured {rel:.2f} dB (expected -12)")
if abs(rel + 12) > 1.5:
    sys.exit("FAIL hdmi gain step not -12 dB")
silent = sum(1 for l in labels if l == ("none", "off"))
if silent:
    sys.exit(f"FAIL {silent} silent windows: program audio was not continuous")
print("PASS program audio: hdmi only → hdmi+mic → mic muted → hdmi -12 dB, continuous")
