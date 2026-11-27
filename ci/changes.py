#!/usr/bin/env python3
"""Change-based job selection for pull requests (ci/README.md).

Outputs one boolean per area. Rules, in order:
- push, schedule and workflow_dispatch run everything (main is always
  fully verified, nightly catches anything a PR skipped);
- a diff that cannot be computed runs everything;
- changes to workflows, CI actions, the build image or Cargo manifests
  run everything;
- every other path maps to the areas it can affect; a path that matches no
  rule runs everything (unknown changes are never skipped);
- documentation-only changes run nothing.
Usage: changes.py [--self-test]; reads EVENT, BASE; writes GITHUB_OUTPUT.
"""
import fnmatch
import json
import os
import subprocess
import sys

AREAS = ["rust", "media", "control", "studio", "provisioning", "mobile", "mobile_native", "ble", "wifi", "lan", "phone", "os"]

# (glob, areas). A path may match several rules; the union is selected.
RULES = [
    ("docs/*.md", []), ("*.md", []), ("LICENSE*", []), (".gitignore", []),
    (".github/*", AREAS), ("ci/images/*", AREAS), ("Cargo.toml", AREAS), ("Cargo.lock", AREAS),
    ("rust-toolchain*", AREAS), ("build.rs", ["rust", "media", "phone", "os"]),
    ("ci/changes.py", AREAS), ("ci/plan.py", AREAS),
    # Media engine
    ("src/native_*", ["rust", "media"]), ("src/*_bridge.c", ["rust", "media", "phone"]),
    ("src/compositor.rs", ["rust", "media"]), ("src/composition.rs", ["rust", "media", "phone"]),
    ("src/colorspace.rs", ["rust", "media"]), ("src/content_analysis.rs", ["rust", "media"]),
    ("src/rtmp*", ["rust", "media"]), ("src/flv*", ["rust", "media"]), ("src/amf0*", ["rust", "media"]),
    ("src/handshake.rs", ["rust", "media"]), ("src/session.rs", ["rust", "media"]), ("src/media_*", ["rust", "media"]),
    ("src/av_clock.rs", ["rust", "media"]), ("src/pcm_framer.rs", ["rust", "media"]), ("src/aac_adts.rs", ["rust", "media"]),
    ("src/audio_mixer.rs", ["rust", "media"]), ("src/audio.rs", ["rust", "media", "control"]), ("ci/audio*", ["media"]), ("src/rate_control.rs", ["rust", "media"]), ("src/reconnect.rs", ["rust", "media"]),
    ("src/transport_deadline.rs", ["rust", "media"]), ("src/tcp_telemetry.rs", ["rust", "media"]), ("src/stream.rs", ["rust", "media"]),
    ("src/accelerated_ci.rs", ["rust", "media"]), ("src/bin/t3box-native-*", ["rust", "media"]), ("src/bin/t3box-ci-encode.rs", ["rust", "media"]),
    ("ci/composition*", ["media"]), ("ci/rtmps-test-server.py", ["media"]), ("ci/report-benchmark.py", ["media"]),
    ("ci/*quality*", ["media"]), ("ci/*transport*", ["media"]), ("ci/test-ci-contracts.py", ["media"]),
    ("tests/virtual-lab/*", ["media"]), ("tests/*.sh", ["media"]), ("tests/fixtures/*", ["media"]),
    # Engine core shared by everything that talks to it
    ("src/main.rs", ["rust", "media", "control", "phone", "os"]), ("src/ipc.rs", ["rust", "control", "phone", "os"]),
    ("src/scene.rs", ["rust", "control", "media"]), ("src/lib.rs", ["rust"]),
    # Phone media
    ("src/h264_decode.rs", ["rust", "phone"]), ("src/phone_ingest.rs", ["rust", "phone"]),
    ("services/phone/*", ["rust", "phone"]), ("tests/phone/*", ["phone"]),
    # Provisioning, BLE, Wi-Fi, LAN trust
    ("src/provisioning*", ["rust", "provisioning", "mobile", "ble", "wifi", "lan"]), ("src/bin/t3box-provision.rs", ["rust", "provisioning", "mobile", "ble", "wifi", "lan"]),
    ("src/nm*.rs", ["rust", "wifi"]), ("tests/wifi/*", ["wifi"]),
    ("services/ble/*", ["ble"]), ("tests/ble/*", ["ble"]), ("os/bluez/*", ["ble", "os"]),
    ("packages/*", ["provisioning", "mobile"]), ("apps/mobile/*", ["mobile"]),
    # Native code and anything that changes the native projects (prebuild).
    ("apps/mobile/modules/*", ["mobile", "mobile_native"]), ("apps/mobile/app.json", ["mobile", "mobile_native"]),
    ("apps/mobile/package*.json", ["mobile", "mobile_native"]), ("ci/mobile-*", ["mobile", "mobile_native"]), ("tools/setup-page/*", ["provisioning"]),
    ("src/pki.rs", ["rust", "lan", "os"]), ("src/management_tokens.rs", ["rust", "lan", "provisioning"]), ("src/bin/t3box-pki.rs", ["rust", "lan", "os"]),
    ("ci/lan*", ["lan"]), ("ci/control*load*", ["control"]),
    # Control plane and Studio
    ("services/control/*", ["control", "studio", "lan", "phone", "os"]), ("apps/studio/*", ["studio", "os"]),
    ("contracts/openapi.yaml", ["control", "studio"]), ("contracts/ipc.schema.json", ["rust", "control"]),
    ("contracts/scene.schema.json", ["rust", "control", "studio"]), ("contracts/ble-protocol.md", ["provisioning"]),
    ("contracts/fixtures/*", ["rust", "control", "studio", "provisioning"]), ("contracts/*", ["control", "studio", "provisioning"]),
    # OS, updates, image
    ("os/*", ["os"]), ("systemd/*", ["control", "os"]), ("src/bootctl.rs", ["rust", "os"]), ("src/bin/t3box-bootctl.rs", ["rust", "os"]),
    ("ci/update-e2e.sh", ["os"]), ("ci/installer-e2e.sh", ["os"]), ("ci/fake_engine.py", ["os"]), ("ci/qemu-smoke.sh", ["os"]),
    ("ci/check-qualification.py", ["rust"]), ("tools/qualification/*", ["rust"]),
    ("src/*", ["rust"]), ("src/bin/*", ["rust"]),
]

def areas_for(path):
    hits = [areas for glob, areas in RULES if fnmatch.fnmatch(path, glob)]
    if not hits:
        return set(AREAS)
    return set().union(*hits)

def select(paths, event):
    if event != "pull_request" or paths is None:
        return {a: True for a in AREAS} | {"reason": "full"}
    chosen = set()
    for p in paths:
        chosen |= areas_for(p)
    result = {a: a in chosen for a in AREAS}
    result["reason"] = "selected: " + (",".join(sorted(chosen)) or "none (docs only)")
    return result

def self_test():
    assert all(select(["docs/a.md"], "push")[a] for a in AREAS)
    assert not any(select(["docs/a.md", "README.md"], "pull_request")[a] for a in AREAS)
    assert all(select(None, "pull_request")[a] for a in AREAS), "unknown diff runs everything"
    assert all(select(["weird/new-thing.bin"], "pull_request")[a] for a in AREAS), "unknown path runs everything"
    assert all(select([".github/workflows/ci.yml"], "pull_request")[a] for a in AREAS)
    r = select(["services/control/app.rb"], "pull_request")
    assert r["control"] and r["studio"] and not r["media"] and not r["ble"] and not r["rust"]
    r = select(["src/native_av.rs"], "pull_request")
    assert r["rust"] and r["media"] and not r["control"]
    r = select(["apps/mobile/src/app/wifi.tsx"], "pull_request")
    assert r["mobile"] and not r["rust"] and not r["ble"] and not r["mobile_native"]
    r = select(["apps/mobile/modules/t3box-pinned-fetch/ios/T3BoxPinnedFetchModule.swift"], "pull_request")
    assert r["mobile"] and r["mobile_native"] and not r["rust"]
    r = select(["src/provisioning_gatt.rs"], "pull_request")
    assert r["ble"] and r["wifi"] and r["mobile"] and not r["media"]
    r = select(["services/phone/src/main.rs"], "pull_request")
    assert r["phone"] and r["rust"] and not r["os"]
    r = select(["os/image/build.sh"], "pull_request")
    assert r["os"] and not r["rust"]
    r = select(["src/brand_new_module.rs"], "pull_request")
    assert r["rust"] and not r["media"]
    assert all(select(["Cargo.lock"], "pull_request")[a] for a in AREAS)
    print("changes.py self-test ok")

if __name__ == "__main__":
    self_test()
    if "--self-test" in sys.argv:
        sys.exit(0)
    event = os.environ.get("EVENT", "")
    base = os.environ.get("BASE", "")
    paths = None
    if base and set(base) != {"0"}:
        try:
            subprocess.run(["git", "fetch", "-q", "--depth=1", "origin", base], check=False)
            out = subprocess.check_output(["git", "diff", "--no-renames", "--name-only", "-z", base, "HEAD"])
            paths = [x.decode("utf-8", "surrogateescape") for x in out.split(b"\0") if x]
        except subprocess.CalledProcessError:
            paths = None
    result = select(paths, event)
    print(json.dumps(result))
    with open(os.environ["GITHUB_OUTPUT"], "a") as fh:
        for k, v in result.items():
            fh.write(f"{k}={json.dumps(v) if isinstance(v, bool) else v}\n")
