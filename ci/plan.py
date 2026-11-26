"""Conservative test selection. Unknown diffs always run full coverage."""
import json
import os
import subprocess

def plan(paths, event):
    docs_only = bool(paths) and all(
        p in {"README.md", "ci/README.md"} or
        (p.startswith("docs/") and p.endswith(".md"))
        for p in paths
    )
    full = event != "pull_request" or not docs_only
    # Only narrow an explicit controller-only change. All other code,
    # configuration, test, and unknown paths receive the full quality matrix.
    controller = event == "pull_request" and bool(paths) and all(
        p in {"src/main.rs", "src/wifi.rs"} for p in paths
    )
    return {
        "full": full,
        "transport": ["combined"] if controller else ["controller", "rtmps"],
        "quality": [{"preset": "ultrafast", "bitrates": "6000"}] if controller else [
            {"preset": "ultrafast", "bitrates": "6000 8000"},
            {"preset": "veryfast", "bitrates": "6000 8000"},
        ],
    }

def self_test():
    assert not plan(["README.md"], "pull_request")["full"]
    assert plan(["README.md", "build.rs"], "pull_request")["full"]
    assert plan([], "pull_request")["full"]
    assert plan(["docs/foo.md"], "push")["full"]
    assert len(plan(["src/main.rs"], "pull_request")["quality"]) == 1
    assert len(plan(["src/stream.rs"], "pull_request")["quality"]) == 2
    assert len(plan([".github/workflows/ci.yml"], "pull_request")["quality"]) == 2

if __name__ == "__main__":
    self_test()
    event = os.environ.get("EVENT", "")
    base = os.environ.get("BASE", "")
    paths = []
    if base:
        try:
            data = subprocess.check_output(["git", "diff", "--no-renames", "--name-only", "-z", base, "HEAD"])
            paths = [x.decode("utf-8", "surrogateescape") for x in data.split(b"\0") if x]
        except subprocess.CalledProcessError:
            pass
    result = plan(paths, event)
    print(json.dumps(result))
    with open(os.environ["GITHUB_OUTPUT"], "a") as out:
        for key, value in result.items():
            out.write(f"{key}={json.dumps(value,separators=(',',':'))}\n")
