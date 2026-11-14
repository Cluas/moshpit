#!/usr/bin/env python3
"""Controller equivalence harness: the app's Swift TmuxSessionController
against the Rust controller, on the same scripts.

    equivalence.py [run] [--no-build] [--fuzz N] [--seed S]
    equivalence.py update-goldens [--no-build]

`run` builds both front ends (harness/controller/build-swift.sh with SwiftPM,
examples/controller_harness.rs), runs every script in
tests/fixtures/controller/ and N seeded random scripts (fuzz.py) through
both, and compares the logs line by line.

The Swift controller is a bug finder here, not the reference: the reference
is what tmux 3.6a does (live.py, a private `-L` server) and the app's rules.
Every difference is triaged in divergences.json:

  identical       the logs agree
  swift-wrong     Swift misbehaves; the Rust log is the right one, with the
                  evidence (live tmux, or the rule) in divergences.json
  on-purpose      a design change, explained there

A divergence fixture (`div-<id>.scn`) must differ, in a way its entry
names — unless the entry says `"status": "fixed"`: fixed in the app since,
its fixtures must now be identical; every other script must be identical,
except for differences a
listed normalisation removes (a fix whose effect shows up in ordinary
scripts, e.g. a dead pane's terminal freed at the next listing).
`update-goldens` writes tests/fixtures/controller/<name>.log (the Rust log)
for tests/controller_logs.rs, which checks them without swiftc.

Everything runs niced, one process at a time.
"""
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(os.path.dirname(HERE))
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures", "controller")
WORK = os.path.join(REPO, "target", "w8-harness")
RUST = os.path.join(REPO, "target", "release", "examples", "controller_harness")
NICE = ["nice", "-n", "10"]


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    done = sh([os.path.join(HERE, "build-swift.sh")], check=True, capture_output=True, text=True)
    swift = done.stdout.strip().splitlines()[-1]
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-tmux", "--example", "controller_harness"],
       check=True, cwd=REPO, env=env)
    return swift


def run(binary, script):
    """The log; a crash ends it with a `!trap <signal>` line."""
    done = sh([binary, "run", script], capture_output=True, timeout=600)
    lines = done.stdout.decode("utf-8", "replace").split("\n")
    if done.returncode != 0:
        lines.append(f"!trap {done.returncode}")
    return lines


def normalise(lines, rules):
    """Drop the lines a rule's pattern matches; a rule with a `replace`
    rewrites what its pattern matches instead (a difference inside a line,
    the rest of the line still compared)."""
    out = []
    for line in lines:
        for rule in rules:
            if "replace" in rule:
                line = re.sub(rule["pattern"], rule["replace"], line)
        if any("replace" not in rule and re.search(rule["pattern"], line) for rule in rules):
            continue
        out.append(line)
    return out


def dead_pane_normalise(rust, swift):
    """Fix dead-pane-terminals (B87): Rust frees a pane's terminal at the
    first whole-server listing without it; Swift keeps it and goes on
    releasing its scroll hold (and feeding it) for a pane tmux no longer has.
    Drop the closes, and Swift's lines for a pane past the point where Rust
    closed it (in the closing step: past as many as Rust had before it)."""
    def steps(log):
        out, current = [], None
        for line in log:
            if line.startswith("@"):
                current = [line]
                out.append(current)
            elif current is not None:
                current.append(line)
            else:
                out.append([line])
        return out

    def pane_of(line):
        parts = line.split(" ")
        if len(parts) >= 3 and parts[0] == "." and parts[1] in ("view", "pane"):
            return parts[2]
        return None

    closed = set()
    keep_rust, keep_swift = [], []
    rust_steps, swift_steps = steps(rust), steps(swift)
    for i, rust_step in enumerate(rust_steps):
        swift_step = swift_steps[i] if i < len(swift_steps) else []
        before = {}
        closing = set()
        for line in rust_step:
            if line.startswith(". close %"):
                pane = line[len(". close "):]
                if pane not in closed:
                    closing.add(pane)
                continue
            pane = pane_of(line)
            if pane and pane not in closing:
                before[pane] = before.get(pane, 0) + 1
            keep_rust.append(line)
        seen = {}
        for line in swift_step:
            if line.startswith(". close %"):
                continue
            pane = pane_of(line)
            if pane in closed:
                continue
            if pane in closing:
                seen[pane] = seen.get(pane, 0) + 1
                if seen[pane] > before.get(pane, 0):
                    continue
            keep_swift.append(line)
        closed |= closing
    for extra in swift_steps[len(rust_steps):]:
        keep_swift.extend(extra)
    return keep_rust, keep_swift


def first_difference(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b)) if len(a) != len(b) else None


def stats(lines):
    return (sum(1 for l in lines if l.startswith("> ")),
            sum(1 for l in lines if l.startswith(". ") or l.startswith("=")),
            sum(1 for l in lines if l.startswith("@")))


def main():
    args = sys.argv[1:]
    mode = args[0] if args and not args[0].startswith("--") else "run"
    fuzz = int(args[args.index("--fuzz") + 1]) if "--fuzz" in args else 0
    seed = int(args[args.index("--seed") + 1]) if "--seed" in args else 1
    swift = os.path.join(WORK, "swift-controller", ".build", "release", "controller-swift")
    if "--no-build" not in args:
        swift = build()
    divergences = json.load(open(os.path.join(HERE, "divergences.json")))
    normalisations = divergences["normalisations"]
    expected = {}
    for entry in divergences["divergences"]:
        for fixture in entry.get("fixtures", [entry["fixture"]]):
            expected[fixture] = entry

    scripts = sorted(os.path.join(FIXTURES, f) for f in os.listdir(FIXTURES) if f.endswith(".scn"))
    if fuzz:
        fuzz_dir = os.path.join(WORK, "fuzz")
        sh([sys.executable, os.path.join(HERE, "fuzz.py"), fuzz_dir, str(seed), str(fuzz)], check=True)
        scripts += sorted(os.path.join(fuzz_dir, f) for f in os.listdir(fuzz_dir) if f.endswith(".scn"))

    totals = {"identical": 0, "normalised": 0, "swift-wrong": 0, "on-purpose": 0}
    compared = [0, 0, 0]
    failures = []
    for script in scripts:
        name = os.path.basename(script)[:-4]
        rust_log = run(RUST, script)
        if mode == "update-goldens" and os.path.dirname(script) == FIXTURES:
            with open(os.path.join(FIXTURES, name + ".log"), "w") as f:
                f.write("\n".join(rust_log))
        swift_log = run(swift, script)
        for i, n in enumerate(stats(rust_log)):
            compared[i] += n
        # Every comparison is of the logs less what the listed
        # normalisations take out (a rewritten field is in every listing);
        # the dead-pane one is the divergence fixtures' own difference, so
        # only ordinary scripts get it.
        a, b = normalise(rust_log, normalisations), normalise(swift_log, normalisations)
        if name in expected and expected[name].get("status") == "fixed":
            # Fixed in the app since: the logs must agree now.
            if a == b:
                totals["identical"] += 1
            else:
                at = first_difference(a, b)
                failures.append(f"{name}: fixed in the app ({expected[name]['id']}) but still differs at line {at}")
            continue
        if name in expected:
            entry = expected[name]
            if a == b:
                failures.append(f"{name}: expected to differ ({entry['id']}) but the logs agree")
                continue
            own = entry["fixture"] == name
            missing = [n for n in entry.get("rust_has", []) if own and not any(n in l for l in rust_log)]
            present = [n for n in entry.get("swift_has", []) if own and not any(n in l for l in swift_log)]
            if missing or present:
                failures.append(f"{name}: rust lacks {missing}, swift lacks {present}")
                continue
            totals[entry["kind"]] += 1
            continue
        if rust_log == swift_log:
            totals["identical"] += 1
            continue
        a, b = dead_pane_normalise(a, b)
        if a == b:
            totals["normalised"] += 1
            continue
        at = first_difference(a, b)
        context = "\n".join(f"   rust {i}: {a[i] if i < len(a) else '<end>'}\n  swift {i}: {b[i] if i < len(b) else '<end>'}"
                            for i in range(max(0, at - 3), min(max(len(a), len(b)), at + 4)))
        failures.append(f"{name}: differs at line {at}\n{context}")
    print(json.dumps({"scripts": len(scripts), **totals, "commands": compared[0],
                      "other_lines": compared[1], "steps": compared[2]}))
    for failure in failures:
        print("FAIL", failure)
    sys.exit(1 if failures else 0)


main()
