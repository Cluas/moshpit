#!/usr/bin/env python3
"""Turns a nextest JUnit report into the job's verdict.

    verdict.py <junit.xml> <quarantine.txt> [<tmux version> <variant>]

Prints passed / failed / skipped counts, and the failures split into those that
fail the job and those quarantined. Exits 1 if a test outside the quarantine
failed, or the report is missing or holds no tests (a build that broke before
running anything must not read as green). Writes a markdown summary to
$GITHUB_STEP_SUMMARY when set.
"""
import os
import sys
import xml.etree.ElementTree as ET


def quarantine(path):
    out = {}
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, _, rest = (p.strip() for p in line.partition("|"))
        out[name] = rest
    return out


def main():
    junit, qpath = sys.argv[1], sys.argv[2]
    label = " ".join(sys.argv[3:5])
    try:
        suites = ET.parse(junit).getroot().iter("testcase")
    except (OSError, ET.ParseError) as e:
        print(f"verdict: no usable report at {junit}: {e}")
        return 1
    quar = quarantine(qpath)
    passed, skipped, hard, soft = [], [], [], []
    for case in suites:
        name = case.get("name", "?")
        short = name.rsplit("::", 1)[-1]
        full = f'{case.get("classname", "")}::{name}'
        if case.find("failure") is not None or case.find("error") is not None:
            (soft if short in quar or name in quar else hard).append(full)
        elif case.find("skipped") is not None:
            skipped.append(full)
        else:
            passed.append(full)
    total = len(passed) + len(skipped) + len(hard) + len(soft)
    lines = [
        f"E2E-1 {label}: {total} run, {len(passed)} passed, "
        f"{len(hard) + len(soft)} failed ({len(soft)} quarantined), {len(skipped)} skipped"
    ]
    for t in hard:
        lines.append(f"  FAIL        {t}")
    for t in soft:
        short = t.rsplit("::", 1)[-1]
        lines.append(f"  quarantined {t}  [{quar.get(short, '')}]")
    for name in quar:
        if not any(p.endswith("::" + name) for p in passed + soft + hard + skipped):
            lines.append(f"  note: quarantined {name} was not in this run")
    print("\n".join(lines))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("```\n" + "\n".join(lines) + "\n```\n")
    return 1 if hard or total == 0 else 0


sys.exit(main())
