"""Report every available result before failing provisional acceptance gates."""
import difflib
import json
import pathlib
import re
import sys
import math

root = pathlib.Path(sys.argv[1])
expected_profiles = sys.argv[2:] or (root / "expected-profiles.txt").read_text().splitlines()
rows = []
results = []
failures = []
for name in expected_profiles:
    perf = root / f"{name}.performance.json"
    try:
        if not (root / f"{name}.complete").is_file():
            raise ValueError("profile execution incomplete")
        p = json.loads(perf.read_text())
        v = json.loads((root / f"{name}.video.json").read_text())["streams"][0]
        a = json.loads((root / f"{name}.audio.json").read_text())["streams"][0]
        log = (root / f"{name}.metrics.log").read_text()
        psnr = float(re.findall(r"average:([\d.]+|inf)", log)[-1])
        ssim = float(re.findall(r"All:([\d.]+)", log)[-1])
        resource = (root / f"{name}.resource.txt").read_text()
        rss = int(re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", resource)[1])
        cpu = int(re.search(r"Percent of CPU this job got:\s*(\d+)%", resource)[1])
        mbps = (root / f"{name}.h264").stat().st_size * 8 / 4 / 1e6
        expected = 'smallcode0123456789abcdefghijklmnopqrstuvwxyz'
        scores = []
        for ts in range(4):
            actual = re.sub(r'[^a-z0-9]', '', (root / f'{name}.text-{ts}.txt').read_text().lower())
            scores.append(difflib.SequenceMatcher(None, expected, actual, autojunk=False).ratio())
        ocr = min(scores)
        vmaf_path = root / f'{name}.vmaf.json'
        vmaf = json.loads(vmaf_path.read_text())['pooled_metrics']['vmaf']['mean'] if vmaf_path.exists() else None
        if not all(math.isfinite(x) for x in (p["fps"], p["p95_ms"], ssim, ocr, mbps)) or math.isnan(psnr) or (vmaf is not None and not math.isfinite(vmaf)):
            raise ValueError("non-finite metrics")
        checks = {
            "video_format": v["codec_name"] == "h264" and (v["width"], v["height"]) == (1920,1080),
            "frame_count": int(v["nb_read_frames"]) == p["frames"] == 120,
            "audio_format": a["codec_name"] == "aac" and int(a["sample_rate"]) == 48000 and a["channels"] == 2,
            "psnr": psnr >= 30,
            "ssim": ssim >= 0.90,
            "ocr": ocr >= 0.90,
            "offline_fps": p["fps"] >= 30,
            "memory": rss <= 1024*1024,
        }
        # Alternative veryfast profiles are comparative, not production speed gates.
        required = set(checks) if name.startswith("ultrafast-") else set(checks)-{"offline_fps"}
        bad = sorted(k for k in required if not checks[k])
        failures.extend(f"{name}: {k}" for k in bad)
        result = dict(profile=name, fps=p["fps"], p95_ms=p["p95_ms"], peak_rss_mib=rss/1024,
                      cpu_percent=cpu, video_mbps=mbps, psnr_db=psnr if math.isfinite(psnr) else None,
                      ssim=ssim, ocr_similarity=ocr, vmaf=vmaf, checks=checks, required_checks=sorted(required), passed=not bad)
        results.append(result)
        rows.append(f"| {name} | {p['fps']:.1f} | {p['p95_ms']:.1f} | {cpu}% | {rss/1024:.1f} | {mbps:.2f} | {psnr:.2f} | {ssim:.4f} | {ocr:.1%} | {vmaf if vmaf is not None else 'N/A'} | {'PASS' if not bad else 'FAIL'} |")
    except (ValueError, KeyError, IndexError, TypeError, OSError, AttributeError) as e:
        failures.append(f"{name}: missing/invalid evidence ({e})")
if len(results) != len(expected_profiles):
    failures.append(f"expected {len(expected_profiles)} complete profiles; received {len(results)}")
report = "\n".join(["# ARM64 native codec quality / performance", "",
    "| Profile | FPS | p95 ms | CPU | RSS MiB | Mbps | PSNR dB | SSIM | OCR | VMAF | Gate |",
    "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|", *rows, "",
    "Provisional gates: PSNR >=30 dB, SSIM >=0.90, OCR similarity >=90%, RSS <=1 GiB, production ultrafast >=30 offline FPS.",
    "Veryfast FPS is comparative. VMAF is optional; N/A is not a score.",
    "Hosted ARM64 results are not Pi 5 calibrated. Offline throughput is not live latency.",
    "", *[f"- FAIL: {x}" for x in failures]])+"\n"
(root / "summary.md").write_text(report)
(root / "quality-report.json").write_text(json.dumps({"profiles":results,"failures":failures}, indent=2, allow_nan=False))
print(report)
sys.exit(bool(failures))
