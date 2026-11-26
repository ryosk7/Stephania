"""Conservative test selection. Unknown diffs always run full coverage."""
import json
import os
import subprocess
import sys

def plan(paths, event):
    docs_only = bool(paths) and all(
        p in {"README.md", "ci/README.md"} or
        (p.startswith("docs/") and p.endswith(".md"))
        for p in paths
    )
    full = event != "pull_request" or not docs_only
    # Never reduce media coverage for executable changes.
    return {
        "full": full,
        "reason": "full" if full else "docs-only",
        "transport": ["controller", "rtmps"],
        "quality": [
            {"preset": "ultrafast", "bitrates": "6000 8000"},
            {"preset": "veryfast", "bitrates": "6000 8000"},
        ],
    }

def self_test():
    assert not plan(["README.md"], "pull_request")["full"]
    assert plan(["README.md", "build.rs"], "pull_request")["full"]
    assert plan([], "pull_request")["full"]
    assert plan(["docs/foo.md"], "push")["full"]
    assert len(plan(["src/main.rs"], "pull_request")["quality"]) == 2
    assert plan(["docs/a.md", "src/lib.rs"], "pull_request")["full"]
    assert plan(["ci/README.md", "ci/plan.py"], "pull_request")["full"]
    assert plan(["docs/a.md"], "workflow_dispatch")["full"]
    assert len(plan(["src/stream.rs"], "pull_request")["quality"]) == 2
    assert len(plan([".github/workflows/ci.yml"], "pull_request")["quality"]) == 2

if __name__ == "__main__":
    self_test()
    if "--self-test" in sys.argv:
        sys.exit(0)
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
