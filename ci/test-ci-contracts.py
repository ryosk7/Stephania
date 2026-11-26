"""Cheap negative tests: incomplete, wrong profile, failed metrics must fail."""
import json
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
        put("audio.json", json.dumps(dict(streams=[dict(codec_name="aac",sample_rate="48000",channels=2)])))
        put("metrics.log", "average:40.0 All:0.99")
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

    def test_shell_syntax(self):
        for p in list(CI.glob("*.sh"))+list((CI.parent/"tests").glob("*.sh")):
            subprocess.run(["bash","-n",str(p)],check=True)

if __name__ == "__main__":
    unittest.main()
