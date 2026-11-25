import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
rows = []
for perf in sorted(root.glob("*.performance.json")):
    name = perf.name.removesuffix(".performance.json")
    p = json.loads(perf.read_text())
    v = json.loads((root / f"{name}.video.json").read_text())["streams"][0]
    a = json.loads((root / f"{name}.audio.json").read_text())["streams"][0]
    assert v["codec_name"] == "h264" and (v["width"], v["height"]) == (1920, 1080)
    assert int(v["nb_read_frames"]) == p["frames"] == 120
    assert a["codec_name"] == "aac" and int(a["sample_rate"]) == 48000 and a["channels"] == 2
    psnr = float(re.findall(r"average:([\d.]+|inf)", (root / f"{name}.psnr.log").read_text())[-1])
    ssim = float(re.findall(r"All:([\d.]+)", (root / f"{name}.ssim.log").read_text())[-1])
    rss = int(re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", (root / f"{name}.resource.txt").read_text())[1])
    mbps = (root / f"{name}.h264").stat().st_size * 8 / 4 / 1e6
    rows.append(f"| {name} | {p['fps']:.1f} | {p['p95_ms']:.1f} | {rss/1024:.1f} | {mbps:.2f} | {psnr:.2f} | {ssim:.4f} |")
    # Provisional minimum integrity thresholds, not perceptual acceptance.
    assert psnr >= 25 and ssim >= 0.85, f"{name}: quality floor failed"
assert len(rows) == 4
report = "\n".join(["# ARM64 resource-constrained native codec benchmark",
    "", "| Profile | FPS | p95 ms | Peak RSS MiB | Actual Mbps | PSNR dB | SSIM |",
    "|---|---:|---:|---:|---:|---:|---:|", *rows,
    "", "4 vCPU quota / 4 GiB memory; not Pi 5 CPU or bandwidth calibration.",
    "Throughput is offline, not real-time capture or network latency."]) + "\n"
(root / "summary.md").write_text(report)
print(report)
