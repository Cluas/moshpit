#!/usr/bin/env python3
"""List the README translations that no longer follow README.md.

Each translation (README-zh.md, README.<locale>.md) starts with

    <!-- source: README.md <blob> -->

where <blob> is the git blob id of the README.md it was written from
(`git hash-object README.md`). When README.md changes, its blob changes, and
every translation still naming the old one is listed here: update it and its
marker, or leave it for someone who writes that language.

    python3 scripts/readme-translations.py            # report; always exits 0
    python3 scripts/readme-translations.py --github   # as workflow warnings
    python3 scripts/readme-translations.py --strict   # exit 1 when any is behind
"""
import hashlib
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MARKER = re.compile(r"<!-- source: README\.md ([0-9a-f]{40}) -->")


def blob_id(data: bytes) -> str:
    """What `git hash-object` prints for these bytes."""
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def main() -> int:
    current = blob_id((ROOT / "README.md").read_bytes())
    behind = []
    for path in sorted(ROOT.glob("README*.md")):
        if path.name == "README.md":
            continue
        head = path.read_text(encoding="utf-8").split("\n", 3)[:3]
        found = next((m.group(1) for line in head if (m := MARKER.search(line))), None)
        if found is None:
            behind.append((path.name, "has no source marker"))
        elif found != current:
            behind.append((path.name, f"follows README.md {found[:8]}, which is now {current[:8]}"))
    github = "--github" in sys.argv
    for name, why in behind:
        print(f"::warning file={name}::{name} {why}" if github else f"{name}: {why}")
    if not behind and not github:
        print(f"every translation follows README.md {current[:8]}")
    return 1 if behind and "--strict" in sys.argv else 0


if __name__ == "__main__":
    sys.exit(main())
