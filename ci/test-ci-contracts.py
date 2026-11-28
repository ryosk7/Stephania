"""Cheap negative tests: incomplete, wrong profile, failed metrics must fail."""
import json
import re
import pathlib
import subprocess
import tempfile
import unittest

CI = pathlib.Path(__file__).resolve().parent

class Contracts(unittest.TestCase):
    def run_report(self, root, *profiles):
        return subprocess.run(["python3", str(CI/"report-benchmark.py"), str(root), *profiles],
                              capture_output=True, text=True)

    def fixture(self, root, name="ultrafast-6000", fps=40):
        def put(suffix, content):
            (root / f"{name}.{suffix}").write_text(content)
        put("complete", "")
        put("performance.json", json.dumps(dict(frames=120, fps=fps, p95_ms=20)))
        put("video.json", json.dumps(dict(streams=[dict(codec_name="h264",width=1920,height=1080,nb_read_frames="120")])))
        put("audio.json", json.dumps(dict(streams=[dict(codec_name="aac",sample_rate="48000",channels=2,nb_read_frames="188")])))
        put("metrics.log", "average:40.0 All:0.99")
        put("psnr-frames.txt", "n:1\n"*120)
        put("ssim-frames.txt", "n:1\n"*120)
        (root/f"{name}.decoded.pcm").write_bytes(b"\x00\x10"*384000)
        put("resource.txt", "Maximum resident set size (kbytes): 10000\nPercent of CPU this job got: 100%")
        put("h264","data")
        for i in range(4):
            put(f"text-{i}.txt","Small code 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ")

    def test_acceptance_and_missing_profile(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d)
            self.fixture(root)
            self.assertEqual(self.run_report(root,"ultrafast-6000").returncode,0)
            self.assertNotEqual(self.run_report(root,"ultrafast-8000").returncode,0)
            self.assertNotEqual(self.run_report(root,"ultrafast-6000","ultrafast-8000").returncode,0)
            self.assertTrue((root/"summary.md").exists())

    def test_failure_and_incomplete(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d)
            self.fixture(root,fps=29)
            self.assertNotEqual(self.run_report(root,"ultrafast-6000").returncode,0)
            self.fixture(root)
            (root/"ultrafast-6000.complete").unlink()
            self.assertNotEqual(self.run_report(root,"ultrafast-6000").returncode,0)

    def test_workflow_plain_scalar(self):
        for p in (CI.parent/".github/workflows").glob("*.yml"):
            for line in p.read_text().splitlines():
                if line.strip().startswith(("- run: ", "run: ")):
                    self.assertFalse(line.rstrip().endswith(":"), f"{p}: {line}")

    def test_missing_audio_and_metric_frames(self):
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d)
            self.fixture(root)
            (root/"ultrafast-6000.decoded.pcm").write_bytes(b"\0"*768000)
            self.assertNotEqual(self.run_report(root,"ultrafast-6000").returncode,0)
            self.fixture(root)
            (root/"ultrafast-6000.psnr-frames.txt").write_text("n:1\n")
            self.assertNotEqual(self.run_report(root,"ultrafast-6000").returncode,0)
            self.assertNotEqual(self.run_report(root,"ultrafast-6000","ultrafast-6000").returncode,0)

    def test_shell_syntax(self):
        for p in list(CI.glob("*.sh"))+list((CI.parent/"tests").glob("*.sh")):
            subprocess.run(["bash","-n",str(p)],check=True)

def jobs(path):
    """Top-level jobs of a workflow as {name: text}, without a YAML parser."""
    lines = path.read_text().splitlines()
    start = lines.index("jobs:") + 1
    out, name = {}, None
    for line in lines[start:]:
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            name = m.group(1); out[name] = []
        elif name:
            out[name].append(line)
    return {k: "\n".join(v) for k, v in out.items()}


def steps(job):
    """Steps of a job's text, each starting at its `- ` line."""
    parts = re.split(r"\n(?=      - )", job)
    return [p for p in parts if p.lstrip().startswith("- ")]


class WorkflowPolicy(unittest.TestCase):
    WF = CI.parent / ".github/workflows"

    def test_actions_pinned_to_commit(self):
        files = list(self.WF.glob("*.yml")) + list((CI.parent / ".github/actions").glob("*/action.yml"))
        for p in files:
            for line in p.read_text().splitlines():
                m = re.match(r"^\s*(?:- )?uses:\s*(\S+)(.*)$", line)
                if not m or m.group(1).startswith("./"):
                    continue
                self.assertRegex(m.group(1), r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$", f"{p.name}: {line.strip()}")
                self.assertRegex(m.group(2), r"^ # v\d+\.\d+\.\d+\b", f"{p.name}: {line.strip()} needs a # vX.Y.Z comment")

    def test_self_hosted_jobs_only_on_dispatch(self):
        for p in self.WF.glob("*.yml"):
            for name, job in jobs(p).items():
                if "self-hosted" not in job:
                    continue
                cond = re.search(r"^    if: (.*)$", job, re.M)
                self.assertIsNotNone(cond, f"{p.name}:{name} runs on a self-hosted runner without an if:")
                self.assertIn("github.event_name == 'workflow_dispatch'", cond.group(1), f"{p.name}:{name}")
                self.assertIn("github.repository == 'ryosk7/Stephania'", cond.group(1), f"{p.name}:{name}")

    def test_release_signing_isolated(self):
        j = jobs(self.WF / "release.yml")
        self.assertEqual(set(j), {"prepare", "image", "stephctl", "sign", "publish"})
        for name, job in j.items():
            if name != "sign":
                self.assertNotIn("secrets.STEPHANIA_", job, f"{name} must not see signing secrets")
            if name != "publish":
                self.assertNotRegex(job, r"contents: write", f"{name} must not get a write token")
            self.assertNotIn("--clobber", job)
            self.assertNotIn("uses: actions/cache", job, f"{name} must not restore shared caches")
            self.assertNotIn("cache: shared", job)
        sign = j["sign"]
        self.assertIn("environment: ${{ needs.prepare.outputs.signing == 'production' && 'release' || 'release-dry-run' }}", sign)
        for tool in ("npm ", "npx ", "cargo ", "bundle ", "gem ", "pip ", "docker ", "cached-build", "setup-node"):
            self.assertNotIn(tool, sign, f"sign job runs {tool.strip()}")
        self.assertIn("/dev/shm", sign)
        self.assertIn("STEPHANIA_STAGE_DIGEST", sign)
        self.assertIn("cache: none", j["image"])
        self.assertIn("local-image: 'true'", j["image"])
        self.assertIn("merge-base --is-ancestor", j["prepare"])
        # Template expansion of inputs inside run: scripts.
        text = (self.WF / "release.yml").read_text()
        for m in re.finditer(r"\$\{\{\s*(github\.event\.inputs|inputs)\.", text):
            line = text[:m.start()].rsplit("\n", 1)[-1]
            self.assertRegex(line.strip(), r"^(ref|[A-Z_]+):", f"input expanded outside env/with: {line.strip()}")

    def test_testflight_key_scoped(self):
        p = self.WF / "testflight.yml"
        (job,) = jobs(p).values()
        self.assertIn("environment: testflight", job)
        self.assertIn("github.ref == 'refs/heads/main'", job)
        head = job.split("    steps:")[0]
        self.assertNotIn("secrets.", head, "no job-level secrets")
        names = []
        for st in steps(job):
            if "secrets.ASC_KEY_P8 }}" in st:
                names.append(re.search(r"name: (.*)", st).group(1))
            if re.search(r"\bnpm ci\b|\bnpx\b", st):
                self.assertNotIn("secrets.", st)
        self.assertEqual(names, ["API key"])
        text = p.read_text()
        self.assertLess(text.index("npx expo prebuild"), text.index("name: API key"), "dependency code runs before the key exists")


if __name__ == "__main__":
    unittest.main()
