#!/usr/bin/env python3
"""Hold app::home to the app's Swift HomeSnapshot it replaced.

Runs every scenario of `scenarios.json` through both — `build-swift.sh`'s
binary (the app's HomeSnapshot at 95f649ab, over session objects set up as
the Rust session path's driver sets them) and `examples/home_crosscheck.rs`
(app::home over the same session views) — and compares the two
`{"op": "home"}` messages as the screen reads them.

Left out of the comparison: what only the Rust record carries, for R5
(`lostAt`, `failure`), which the Swift builder never sent. Differences
are listed against `divergences.json`, which explains each one; an
unexplained difference fails.

    crates/app/harness/home/crosscheck.py [scenario ...]

`$CARGO` is the cargo command line (default `cargo`), `$CARGO_TARGET_DIR`
where it builds.
"""

import json
import os
import shlex
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
SCENARIOS = os.path.join(HERE, "scenarios.json")
R5_ONLY = {"lostAt", "failure"}


def normal(message):
    """The message as the screen reads it: an absent optional and a null
    one are the same (`#[serde(default)]`), and R5's fields are Rust's
    alone."""
    if isinstance(message, dict):
        return {k: normal(v) for k, v in message.items() if v is not None and k not in R5_ONLY}
    if isinstance(message, list):
        return [normal(v) for v in message]
    return message


def runs(binary, names):
    text = subprocess.run([binary, SCENARIOS, *names], check=True, capture_output=True,
                          text=True).stdout
    return {r["scenario"]: normal(r["message"]) for r in map(json.loads, text.splitlines())}


def diff(a, b, path=""):
    """Where `a` (Swift) and `b` (Rust) differ, as (path, swift, rust)."""
    if isinstance(a, dict) and isinstance(b, dict):
        out = []
        for key in sorted(set(a) | set(b)):
            out += diff(a.get(key), b.get(key), f"{path}.{key}")
        return out
    if isinstance(a, list) and isinstance(b, list):
        if len(a) == len(b):
            return [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f"{path}[{i}]")]
        return [(f"{path} (of {len(a)} / {len(b)})", a, b)]
    return [] if a == b else [(path, a, b)]


def main():
    fixture = json.load(open(SCENARIOS))
    names = sys.argv[1:] or [s["name"] for s in fixture["scenarios"]]
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    swift_bin = os.path.join(target, "home-harness", "home-swift")
    if not os.path.exists(swift_bin):
        subprocess.run([os.path.join(HERE, "build-swift.sh"), swift_bin], check=True)
    cargo = shlex.split(os.environ.get("CARGO", "cargo"))
    subprocess.run([*cargo, "build", "--quiet", "-p", "moshpit-app", "--example", "home_crosscheck"],
                   check=True, cwd=ROOT)
    rust_bin = os.path.join(target, "debug", "examples", "home_crosscheck")
    swift = runs(swift_bin, names)
    rust = runs(rust_bin, names)
    known = json.load(open(os.path.join(HERE, "divergences.json")))
    same = explained = 0
    unexplained = []
    for name in names:
        found = diff(swift[name], rust[name])
        if not found:
            same += 1
            print(f"same        {name}")
            continue
        why = known.get(name)
        if why and all(any(where.startswith(p) for p in why["paths"]) for where, _, _ in found):
            explained += 1
            print(f"explained   {name}: {why['id']}")
        else:
            unexplained.append(name)
            print(f"DIFFERENT   {name}")
        for where, a, b in found:
            print(f"    {where}\n      swift: {json.dumps(a, ensure_ascii=False)[:400]}"
                  f"\n      rust:  {json.dumps(b, ensure_ascii=False)[:400]}")
    print(f"\n{len(names)} scenarios: {same} the same, {explained} explained, "
          f"{len(unexplained)} unexplained")
    sys.exit(1 if unexplained else 0)


if __name__ == "__main__":
    main()
