#!/usr/bin/env python3
"""What the shots' shell scripts must not do on macOS's /bin/bash 3.2, the
bash a runner's `#!/usr/bin/env bash` finds. Checked before the build, so a
mistake shows in seconds rather than after it.

    python3 -I scripts/shots/lint.py [script…]   (default: scripts/shots/**/*.sh)

- bash 4 and later only: associative arrays, mapfile/readarray, case
  changes (${x,,} ${x^^}), |&, &>>, coproc, ;& and ;;& in a case.
- An array set empty (`name=()`) and later expanded bare ("${name[@]}"):
  under set -u bash 3.2 calls an empty array unset and stops the script
  (43812423973's compose). Expand it as ${name[@]+"${name[@]}"}, or mark
  a line where it can't be empty with `# never empty`.

Exit 1 with each finding as file:line, else 0.
"""

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BASH4 = [
    (re.compile(r"\b(declare|local|typeset)\s+-[a-zA-Z]*A"), "an associative array (bash 4)"),
    (re.compile(r"\b(mapfile|readarray)\b"), "mapfile/readarray (bash 4)"),
    (re.compile(r"\$\{[A-Za-z_][A-Za-z0-9_]*(\[[^]]*\])?(,,?|\^\^?)\}"), "a case change ${x,,} ${x^^} (bash 4)"),
    (re.compile(r"\|&"), "|& (bash 4)"),
    (re.compile(r"&>>"), "&>> (bash 4)"),
    (re.compile(r"\bcoproc\b"), "coproc (bash 4)"),
    (re.compile(r";;?&\s*$"), ";& / ;;& in a case (bash 4)"),
]
EMPTIED = re.compile(r"(?:^|[\s;(])(?:local\s+[^=]*?\b)?([A-Za-z_][A-Za-z0-9_]*)=\(\)")
BARE = re.compile(r'(?<!\+)"\$\{([A-Za-z_][A-Za-z0-9_]*)\[@\]\}"')


def code(line):
    """The line without a comment (a crude cut at an unquoted #)."""
    quote = None
    for i, c in enumerate(line):
        if quote:
            if c == quote:
                quote = None
        elif c in "'\"":
            quote = c
        elif c == "#" and (i == 0 or line[i - 1] in " \t;"):
            return line[:i]
    return line


def lint(path):
    findings = []
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    emptied = set()
    for line in lines:
        for m in EMPTIED.finditer(code(line)):
            emptied.add(m.group(1))
    for n, line in enumerate(lines, 1):
        text = code(line)
        for pattern, what in BASH4:
            if pattern.search(text):
                findings.append(f"{path}:{n}: {what}")
        if "never empty" in line:
            continue
        for m in BARE.finditer(text):
            name = m.group(1)
            if name in emptied and f"${{{name}[@]+" not in text:
                findings.append(f"{path}:{n}: \"${{{name}[@]}}\" can be empty: "
                                f"${{{name}[@]+\"${{{name}[@]}}\"}}, or mark it `# never empty`")
    return findings


def main():
    paths = sys.argv[1:] or sorted(glob.glob(os.path.join(ROOT, "scripts", "shots", "**", "*.sh"), recursive=True))
    findings = [f for p in paths for f in lint(p)]
    for f in findings:
        print(os.path.relpath(f, ROOT) if f.startswith(ROOT) else f)
    if findings:
        print(f"lint: {len(findings)} finding(s) bash 3.2 would trip on", file=sys.stderr)
        return 1
    print(f"lint: {len(paths)} scripts, nothing bash 3.2 would trip on")
    return 0


if __name__ == "__main__":
    sys.exit(main())
