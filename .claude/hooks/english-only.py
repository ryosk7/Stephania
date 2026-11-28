#!/usr/bin/env python3
"""Claude Code PreToolUse hook: keep commit messages, PRs and code comments in English.

Blocks (exit 2) when:
  * a Bash command that commits (git commit) or writes a pull request or
    review (gh pr create/edit/comment/review, gh api .../pulls..., gh api
    .../issues/.../comments) contains Japanese or other CJK text, or
  * Write, Edit or MultiEdit would add CJK text inside a code comment or a
    Markdown/plain-text doc (string literals are allowed).

The rules themselves live in ci/check-english.py, shared with the git hooks
and the English workflow.
"""
import importlib.util
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("check_english", ROOT / "ci" / "check-english.py")
ce = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ce)

WRITES_TEXT = re.compile(
    r"\bgit\s+(-\S+\s+)*commit\b|\bgh\s+pr\s+(create|edit|comment|review)\b|"
    r"\bgh\s+api\b.*(/pulls\b|/issues/\d+/comments\b)"
)


def block(reasons):
    print("English-only rule: commit messages, pull requests and code comments must be in English.", file=sys.stderr)
    for r in reasons[:10]:
        print("  " + r, file=sys.stderr)
    print("Rewrite the text in English and try again.", file=sys.stderr)
    sys.exit(2)


def main():
    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        return
    tool = event.get("tool_name", "")
    inp = event.get("tool_input") or {}
    if tool == "Bash":
        cmd = inp.get("command", "")
        if WRITES_TEXT.search(cmd) and ce.CJK.search(cmd):
            block([f"command: {line.strip()[:120]}" for line in cmd.splitlines() if ce.CJK.search(line)])
        return
    if tool in ("Write", "Edit", "MultiEdit"):
        path = inp.get("file_path", "")
        try:
            rel = str(Path(path).resolve().relative_to(ROOT))
        except ValueError:
            return  # outside this repository
        style = ce.comment_style(rel)
        if not style:
            return
        if tool == "Write":
            texts = [inp.get("content", "")]
        elif tool == "Edit":
            texts = [inp.get("new_string", "")]
        else:
            texts = [e.get("new_string", "") for e in inp.get("edits", [])]
        errors = []
        for t in texts:
            errors += ce.scan_lines(rel, list(enumerate(t.split("\n"), 1)), style)
        if errors:
            block(errors)


if __name__ == "__main__":
    main()
