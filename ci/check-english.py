#!/usr/bin/env python3
"""Enforce the English-only rule for commit messages, pull requests and code comments.

Japanese (and other CJK) text is refused in:
  * commit messages            --message FILE | --commits REV_RANGE
  * pull request title/body    --text-env VAR [VAR ...]
  * comments in source files   --staged | --diff BASE | --all [PATH ...]
  * Markdown and plain-text documentation (the whole line counts)

CJK inside string literals in code is allowed: tests use it on purpose
(Unicode SSIDs, OCR fixtures, full-width digits).

Used by .githooks/commit-msg, .githooks/pre-commit, the English workflow and
the Claude Code hook in .claude/hooks/english-only.py.
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

# Hiragana, Katakana (+ phonetic extensions), CJK punctuation, CJK ideographs
# (+ extension A and compatibility), half/full-width forms, Hangul syllables.
CJK = re.compile(
    "[　-〿぀-ゟ゠-ヿㇰ-ㇿ㐀-䶿"
    "一-鿿豈-﫿＀-￯가-힯]"
)

HASH = {".py", ".rb", ".sh", ".bash", ".yml", ".yaml", ".toml", ".nft", ".conf",
        ".service", ".timer", ".socket", ".rules", ".env", ".cfg", ".ini", ".ru",
        ".fragment", ".dockerignore", ".gitignore"}
SLASH = {".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".c", ".h", ".cc",
         ".cpp", ".swift", ".kt", ".kts", ".java", ".css", ".scss", ".go", ".gradle"}
XML = {".html", ".xml", ".svg", ".plist"}
PROSE = {".md", ".txt", ".rst"}
SQL = {".sql"}
HASH_NAMES = {"Dockerfile", "Gemfile", "Rakefile", "Makefile", "Brewfile", "config.ru"}
# Generated or third-party content that is not ours to translate.
SKIP = re.compile(r"(^|/)(licences\.json|package-lock\.json|Cargo\.lock|Gemfile\.lock|"
                  r"schema\.d\.ts)$|(^|/)contracts/fixtures/|\.(png|jpe?g|der|ico|woff2?|ttf)$")


def comment_style(path: str) -> str | None:
    p = Path(path)
    name, ext = p.name, p.suffix.lower()
    if SKIP.search(path):
        return None
    if ext in PROSE:
        return "prose"
    if ext in SLASH:
        return "slash"
    if ext in HASH or name in HASH_NAMES or name.startswith("Dockerfile"):
        return "hash"
    if ext in XML:
        return "xml"
    if ext in SQL:
        return "sql"
    if ext == "" and path.startswith((".githooks/", "os/", "ci/", "tools/")):
        return "hash"  # extensionless scripts
    return None


def comment_text(line: str, style: str, in_block: bool) -> tuple[str, bool]:
    """Return (text that is inside a comment on this line, still in block comment)."""
    if style == "prose":
        return line, False
    out = []
    i, n = 0, len(line)
    quote: str | None = None
    while i < n:
        if in_block:
            end = line.find("*/" if style == "slash" else "-->", i)
            if end < 0:
                out.append(line[i:])
                return "".join(out), True
            out.append(line[i:end])
            i = end + (2 if style == "slash" else 3)
            in_block = False
            continue
        c = line[i]
        if quote:
            if c == "\\":
                i += 2
                continue
            if c == quote:
                quote = None
            i += 1
            continue
        if c in "\"'`":
            # A lone apostrophe in Rust lifetimes/chars is harmless for this purpose.
            quote = c
            i += 1
            continue
        if style == "hash" and c == "#":
            out.append(line[i + 1:])
            break
        if style == "sql" and line.startswith("--", i):
            out.append(line[i + 2:])
            break
        if style == "slash":
            if line.startswith("//", i):
                out.append(line[i + 2:])
                break
            if line.startswith("/*", i):
                in_block = True
                i += 2
                continue
        if style == "xml" and line.startswith("<!--", i):
            in_block = True
            i += 4
            continue
        i += 1
    return "".join(out), in_block


def scan_lines(path: str, lines: list[tuple[int, str]], style: str) -> list[str]:
    errors = []
    in_block = False
    for lineno, line in lines:
        text, in_block = comment_text(line, style, in_block)
        if CJK.search(text):
            errors.append(f"{path}:{lineno}: non-English text in a comment: {line.strip()[:120]}")
    return errors


def scan_file(path: str) -> list[str]:
    style = comment_style(path)
    if not style:
        return []
    try:
        content = Path(path).read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        return []
    return scan_lines(path, list(enumerate(content.split("\n"), 1)), style)


def changed_files(diff_args: list[str]) -> list[str]:
    out = subprocess.run(["git", "diff", "--name-only", "--diff-filter=AM", *diff_args],
                         check=True, capture_output=True, text=True).stdout
    return [f for f in out.splitlines() if f]


def added_line_numbers(diff_args: list[str], path: str) -> set[int]:
    out = subprocess.run(["git", "diff", "-U0", *diff_args, "--", path],
                         check=True, capture_output=True, text=True).stdout
    nums: set[int] = set()
    for m in re.finditer(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@", out, re.M):
        start, count = int(m.group(1)), int(m.group(2) or "1")
        nums.update(range(start, start + count))
    return nums


def scan_diff(diff_args: list[str], read_index: bool) -> list[str]:
    """Scan whole files (so block-comment state is right) but report only added lines."""
    errors = []
    for path in changed_files(diff_args):
        style = comment_style(path)
        if not style:
            continue
        if read_index:
            r = subprocess.run(["git", "show", f":{path}"], capture_output=True)
            if r.returncode != 0:
                continue
            raw = r.stdout
        else:
            try:
                raw = Path(path).read_bytes()
            except OSError:
                continue
        try:
            content = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
        added = added_line_numbers(diff_args, path)
        lines = list(enumerate(content.split("\n"), 1))
        errors += [e for e in scan_lines(path, lines, style)
                   if int(e.split(":", 2)[1]) in added]
    return errors


def check_text(label: str, text: str) -> list[str]:
    errors = []
    for i, line in enumerate(text.split("\n"), 1):
        if line.startswith("#") and label == "commit message":
            continue  # git's own comment lines in the editor template
        if CJK.search(line):
            errors.append(f"{label} line {i}: non-English text: {line.strip()[:120]}")
    return errors


def self_test() -> None:
    assert CJK.search("日本語") and CJK.search("カタカナ") and CJK.search("１")
    assert not CJK.search("English — with “quotes” and é")
    assert scan_lines("a.rs", [(1, 'let s = "会場"; // ok')], "slash") == []
    assert scan_lines("a.rs", [(1, "let s = 1; // 日本語")], "slash")
    assert scan_lines("a.rs", [(1, "/* start"), (2, "  日本語"), (3, "*/ x")], "slash")
    assert scan_lines("a.rb", [(1, "x = '# 会場'")], "hash") == []
    assert scan_lines("a.rb", [(1, "x = 1 # 会場")], "hash")
    assert scan_lines("a.md", [(1, "見出し")], "prose")
    assert scan_lines("a.html", [(1, "<!-- 注 -->")], "xml")
    assert comment_style("apps/mobile/src/features/settings/licences.json") is None
    assert check_text("commit message", "Fix bug\n\n修正") and not check_text("commit message", "Fix\n# 日本語")
    print("check-english: self-test ok")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--message", help="commit message file (commit-msg hook)")
    ap.add_argument("--commits", help="check every commit message in a revision range, e.g. origin/main..HEAD")
    ap.add_argument("--text-env", nargs="+", metavar="VAR", help="check text in these environment variables (PR title/body)")
    ap.add_argument("--staged", action="store_true", help="check comments on staged added lines")
    ap.add_argument("--diff", metavar="BASE", help="check comments on lines added since BASE")
    ap.add_argument("--all", nargs="*", metavar="PATH", help="check every tracked file (or the given paths)")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        self_test()
        return 0
    errors: list[str] = []
    if a.message:
        errors += check_text("commit message", Path(a.message).read_text(encoding="utf-8", errors="replace"))
    if a.commits:
        revs = subprocess.run(["git", "rev-list", a.commits], check=True, capture_output=True, text=True).stdout.split()
        for rev in revs:
            msg = subprocess.run(["git", "log", "-1", "--format=%B", rev], check=True, capture_output=True, text=True).stdout
            errors += [f"commit {rev[:10]}: {e}" for e in check_text("message", msg)]
    for var in a.text_env or []:
        errors += check_text(var, os.environ.get(var, ""))
    if a.staged:
        errors += scan_diff(["--cached"], read_index=True)
    if a.diff:
        errors += scan_diff([f"{a.diff}...HEAD"], read_index=False)
    if a.all is not None:
        paths = a.all or subprocess.run(["git", "ls-files"], check=True, capture_output=True, text=True).stdout.split("\n")
        for p in paths:
            if p:
                errors += scan_file(p)
    if errors:
        print("English-only rule: commit messages, pull requests, code comments and docs must be in English.", file=sys.stderr)
        for e in errors:
            print("  " + e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
