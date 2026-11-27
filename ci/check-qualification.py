#!/usr/bin/env python3
"""Guards docs/QUALIFICATION.md against unsupported claims.

Every results row must be `Not run`, `Blocked`, or `Pass`/`Fail` with a
date (YYYY-MM-DD), hardware, software revision and an evidence link.
Run with --self-test to check the checker.
"""
import re
import sys

RESULTS = {"Not run", "Blocked", "Pass", "Fail"}
LINK = re.compile(r"\[[^\]]+\]\([^)]+\)|https?://\S+")


def rows(text):
    section = text.split("## Results", 1)
    if len(section) != 2:
        raise SystemExit("QUALIFICATION.md: missing '## Results' section")
    for line in section[1].splitlines():
        if not line.startswith("|") or set(line) <= set("|- "):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if cells[0] == "ID":
            continue
        yield cells


def check(text):
    errors = []
    for cells in rows(text):
        if len(cells) != 7:
            errors.append(f"row needs 7 columns: {cells}")
            continue
        rid, date, hw, sw, result, evidence, _ = cells
        if result not in RESULTS:
            errors.append(f"{rid}: result {result!r} not in {sorted(RESULTS)}")
        elif result in ("Pass", "Fail"):
            if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
                errors.append(f"{rid}: {result} needs a date")
            if not hw or not sw:
                errors.append(f"{rid}: {result} needs hardware and software revision")
            if not LINK.search(evidence):
                errors.append(f"{rid}: {result} needs an evidence link")
    return errors


def self_test():
    head = "## Results\n| ID | Date | Hardware | Software | Result | Evidence | Notes |\n|---|---|---|---|---|---|---|\n"
    assert check(head + "| Q-A | | | | Not run | | |\n") == []
    assert check(head + "| Q-A | | | | Pass | | |\n")
    assert check(head + "| Q-A | 2026-12-01 | pi5-1 | abc123 | Pass | | |\n")
    assert check(head + "| Q-A | 2026-12-01 | pi5-1 | abc123 | Pass | [log](evidence/q-a.tar.zst) | |\n") == []
    assert check(head + "| Q-A | | | | Passed | | |\n")
    print("check-qualification self-test ok")


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        self_test()
        sys.exit(0)
    path = sys.argv[1] if len(sys.argv) > 1 else "docs/QUALIFICATION.md"
    errs = check(open(path, encoding="utf-8").read())
    for e in errs:
        print("QUALIFICATION:", e)
    sys.exit(1 if errs else 0)
