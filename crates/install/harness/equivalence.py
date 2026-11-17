#!/usr/bin/env python3
"""Equivalence harness: frozen Swift install code (81c121a1) vs its Rust port.

    equivalence.py [run] [--quick] [--no-build]
    equivalence.py update-goldens [--no-build]

`run` builds both front ends (harness/build-swift.sh over the frozen Swift
sources at 81c121a1 in harness/swift/frozen, and examples/install_harness.rs)
and feeds both the same cases: seeded ones from `install_harness gen` and the
committed tests/fixtures/swift/cases.txt.

Categories split into two tiers:
- Hard gate (must match frozen Swift byte-for-byte across all seeds, plus all
  387 committed cases against cases.swift.txt and all 23 divergences): failures
  exit 1.
- Evolved (informational): categories where Rust intentionally diverged after
  the 81c121a1 freeze (scripts v4, atomic writes, preserved JSON layout, etc.,
  documented in EVOLVED_REASONS and crates/install/harness/README.md). These are
  still checked against cases.swift.txt / divergences.rust.txt for Rust stability.
"""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures", "swift")
TARGET_DIR = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
WORK = os.path.join(TARGET_DIR, "install-harness")
SWIFT = os.path.join(WORK, "install-swift")
RUST = os.path.join(TARGET_DIR, "release", "examples", "install_harness")
NICE = ["nice", "-n", "10"]

SEEDS = list(range(1, 17))
COUNT = 2500
QUICK_SEEDS = [1, 2]

# Categories where Rust intentionally evolved past the frozen Swift sources
# (81c121a1). Each entry states the B-numbers that introduced the divergence.
EVOLVED_REASONS = {
    "probe": "B875 (POSIX sh per-binary command -v)",
    "scripts": "B443, B566, B633, B637, B700, B784 (host scripts v1 -> v4)",
    "cmd preflight": "B374 (agent presence probe)",
    "cmd write": "B351, B379 (same-dir temp replace + symlink follow)",
    "merge claude": "B356, B375 (shared hook groups + preserved JSON layout)",
    "merge codebuddy": "B356, B375 (shared hook groups + preserved JSON layout)",
    "merge factory": "B356, B375 (shared hook groups + preserved JSON layout)",
    "merge gemini": "B356, B375 (shared hook groups + preserved JSON layout)",
    "merge qoder": "B356, B375 (shared hook groups + preserved JSON layout)",
    "merge qwen": "B356, B375 (shared hook groups + preserved JSON layout)",
    "remove claude": "B356, B375 (shared hook groups + preserved JSON layout)",
    "remove codebuddy": "B356, B375 (shared hook groups + preserved JSON layout)",
    "remove factory": "B356, B375 (shared hook groups + preserved JSON layout)",
    "remove gemini": "B356, B375 (shared hook groups + preserved JSON layout)",
    "remove qoder": "B356, B375 (shared hook groups + preserved JSON layout)",
    "remove qwen": "B356, B375 (shared hook groups + preserved JSON layout)",
    "flow hooks": "B351, B356, B375, B443, B566, B633, B637, B700, B784",
    "flow pairing": "B351, B443, B566, B633, B637, B700, B784",
    "flow push": "B351, B353, B379, B566, B633, B637, B700, B784",
    "flow stamp": "B351, B355, B379, B566, B633, B700, B784",
    "flow status": "B374, B443, B566, B633, B700, B784",
    "flow unhooks": "B351, B356, B375, B633, B637, B700, B784",
    "flow uninstall": "B566, B633, B637, B700, B784",
}

# Cases for a command the Swift code never had, so its binary can't answer
# them ("bad cmd …"): each a fix made on this side as one new command, its
# difference documented. Their Swift lines are the records
# divergences.swift.txt keeps, never run again.
SWIFT_LACKS = (
    "cmd writekeep ",  # config-mode (B352): a config written keeping its mode
    "cmd pushtest ",   # self-test-scope (B353): the push self-test for one pairing
    "cmd stamptest ",  # stamp-test-restores (B355): one command where Swift ran three
)


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL,
       stderr=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-install", "--example", "install_harness"],
       check=True, cwd=REPO, env=env)


def generate(seed, count):
    out = sh([RUST, "gen", str(seed), str(count)], check=True, capture_output=True, text=True).stdout
    return [line for line in out.split("\n") if line]


def run_side(binary, cases):
    with tempfile.TemporaryDirectory(prefix="install-eq-") as tmp:
        path = os.path.join(tmp, "cases.txt")
        result = os.path.join(tmp, "out.txt")
        with open(path, "w") as f:
            f.write("\n".join(cases) + "\n")
        sh([binary, "run", path, result], check=True)
        with open(result) as f:
            return [line for line in f.read().split("\n") if line]


def read_lines(path):
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [line for line in f.read().split("\n") if line and not line.startswith("#")]


def divergence_ids(path):
    """The fix id of each case in divergences.txt: the `# <id>: …` line above it."""
    ids, current = [], None
    with open(path) as f:
        for line in f.read().split("\n"):
            if line.startswith("# ") and ":" in line:
                current = line[2:].split(":", 1)[0]
            elif line and not line.startswith("#"):
                ids.append(current)
    return ids


def ported_fixes():
    with open(os.path.join(HERE, "divergences.json")) as f:
        return {fix["id"] for fix in json.load(f)["fixes"] if fix.get("status") == "fixed"}


def kind(case):
    parts = case.split(" ")
    if parts[0] in ("flow", "cmd", "merge", "remove") and len(parts) > 1:
        return parts[0] + " " + parts[1]
    return parts[0]


def swift_lacks(case):
    return case.startswith(SWIFT_LACKS)


def run_swift_divergences(cases, recorded):
    """The Swift side of the divergence cases: run, but for the ones it
    lacks, whose line is the record."""
    runnable = [case for case in cases if not swift_lacks(case)]
    ran = iter(run_side(SWIFT, runnable)) if runnable else iter(())
    return [want if swift_lacks(case) else next(ran) for case, want in zip(cases, recorded)]


def run(quick):
    seeds = QUICK_SEEDS if quick else SEEDS
    tally, evolved, failures, total = {}, {}, [], 0
    for seed in seeds:
        cases = generate(seed, COUNT)
        if not cases:
            continue
        swift = run_side(SWIFT, cases)
        rust = run_side(RUST, cases)
        assert len(swift) == len(rust) == len(cases)
        for case, s, r in zip(cases, swift, rust):
            k = kind(case)
            tally[k] = tally.get(k, 0) + 1
            if s != r:
                if k in EVOLVED_REASONS:
                    evolved[k] = evolved.get(k, 0) + 1
                else:
                    failures.append((k, case, s, r))
        total += len(cases)

    committed = read_lines(os.path.join(FIXTURES, "cases.txt"))
    committed_golden = read_lines(os.path.join(FIXTURES, "cases.swift.txt"))
    golden_diffs = []
    if committed:
        assert len(committed) == len(committed_golden)
        swift_committed = run_side(SWIFT, committed)
        rust_committed = run_side(RUST, committed)
        for case, want, s, r in zip(committed, committed_golden, swift_committed, rust_committed):
            k = kind(case)
            if r != want:
                golden_diffs.append((case, want, r))
            if k not in EVOLVED_REASONS and s != r:
                failures.append((k, case, s, r))

    print(f"seeded cases: {total} across {len(tally)} kinds:")
    for k in sorted(tally):
        n = tally[k]
        if k in EVOLVED_REASONS:
            d = evolved.get(k, 0)
            print(f"  [evolved]   {k:18s} {n - d:5d}/{n:<5d} matched, {d:5d} evolved ({EVOLVED_REASONS[k]})")
        else:
            d = sum(1 for fk, _, _, _ in failures if fk == k)
            status = "PASS" if d == 0 else f"FAIL ({d} diffs)"
            print(f"  [hard-gate] {k:18s} {n:5d}/{n:<5d} {status}")
    for _, case, s, r in failures[:20]:
        print(f"FAIL HARD-GATE {case[:160]}\n  swift {s[:400]}\n  rust  {r[:400]}")
    for case, want, r in golden_diffs[:20]:
        print(f"FAIL GOLDEN {case[:160]}\n  want  {want[:400]}\n  rust  {r[:400]}")
    print(f"committed cases: {len(committed)}, {len(golden_diffs)} golden diffs; {len(failures)} hard-gate diffs")

    bad = 0
    evolved_div = 0
    divergences = read_lines(os.path.join(FIXTURES, "divergences.txt"))
    if divergences:
        recorded = read_lines(os.path.join(FIXTURES, "divergences.swift.txt"))
        recorded_rust = read_lines(os.path.join(FIXTURES, "divergences.rust.txt"))
        swift = run_swift_divergences(divergences, recorded)
        rust = run_side(RUST, divergences)
        ported = ported_fixes()
        ids = divergence_ids(os.path.join(FIXTURES, "divergences.txt"))
        for fix, case, s, r, want, want_r in zip(ids, divergences, swift, rust, recorded, recorded_rust):
            if r != want_r:
                bad += 1
                print(f"DIVERGENCE RUST CHANGED ({fix}) {case[:120]}\n  recorded {want_r[:200]}\n  rust     {r[:200]}")
            if fix in ported and swift_lacks(case):
                # Its port was checked while the Swift code had the command;
                # no Swift binary can answer it now.
                continue
            if fix in ported:
                if kind(case) in EVOLVED_REASONS:
                    evolved_div += 1
                elif s != r:
                    bad += 1
                    print(f"DIVERGENCE NOT PORTED {fix}: fixed in the app, but Swift still differs for {case[:120]}")
                continue
            if s != want:
                bad += 1
                print(f"DIVERGENCE CHANGED {case[:120]}\n  recorded {want[:200]}\n  swift    {s[:200]}")
            if s == r:
                bad += 1
                print(f"DIVERGENCE GONE {case[:120]}")
        print(f"{len(divergences)} divergence cases ({evolved_div} ported+evolved), {bad} not as documented")
    return 1 if failures or golden_diffs or bad else 0


def update_goldens():
    os.makedirs(FIXTURES, exist_ok=True)
    cases = read_lines(os.path.join(FIXTURES, "cases.txt"))
    with open(os.path.join(FIXTURES, "cases.swift.txt"), "w") as f:
        f.write("\n".join(run_side(RUST, cases)) + "\n")
    divergences = read_lines(os.path.join(FIXTURES, "divergences.txt"))
    # A fix ported to the app keeps its pre-fix Swift line: the record of
    # what the case tells the fix apart from.
    ported = ported_fixes()
    ids = divergence_ids(os.path.join(FIXTURES, "divergences.txt"))
    before = read_lines(os.path.join(FIXTURES, "divergences.swift.txt"))
    swift = run_swift_divergences(divergences, before)
    swift = [old if fix in ported else now for fix, now, old in zip(ids, swift, before)]
    with open(os.path.join(FIXTURES, "divergences.swift.txt"), "w") as f:
        f.write("\n".join(swift) + "\n")
    with open(os.path.join(FIXTURES, "divergences.rust.txt"), "w") as f:
        f.write("\n".join(run_side(RUST, divergences)) + "\n")
    print(f"wrote {len(cases)} committed results and {len(divergences)} divergence results")


def main():
    args = sys.argv[1:]
    command = args[0] if args and not args[0].startswith("--") else "run"
    if "--no-build" not in args:
        build()
    if command == "run":
        sys.exit(run("--quick" in args))
    elif command == "update-goldens":
        update_goldens()
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
