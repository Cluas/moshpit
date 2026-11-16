#!/usr/bin/env python3
"""Equivalence harness: the app's Swift push code against its Rust port.

    equivalence.py [run] [--quick] [--no-build]
    equivalence.py update-goldens [--no-build]

`run` builds both front ends (harness/build-swift.sh over the app's
unchanged MoshpitKit push sources, kept in harness/swift/frozen since E7b
deleted them, and examples/seal_harness.rs) and feeds
both the same cases: seeded ones from `seal_harness gen` (envelopes sealed and
then bent every way, status plaintexts Foundation forgives or refuses, card
text with every kind of whitespace, numeral and emoji, durations in the nine
languages, payloads, pairing and standing files with entries no build reads,
JSONSerialization round trips, base64, UUIDs, ISO 8601 dates, doubles) and the
committed ones in tests/fixtures/swift/cases.txt.

Every result must be byte-identical, with one normalisation: the JSON a store
file holds is compared as JSON (Swift's key order is a per-process hash
order). The only accepted differences are the documented fixes in
harness/divergences.json, whose minimal cases live in
tests/fixtures/swift/divergences.txt: their Swift results are recorded in
divergences.swift.txt and must reappear, and the Rust results must differ.

`update-goldens` rewrites the committed Swift outputs tests/swift_equivalence.rs
checks without swiftc. Everything runs niced, one process at a time.
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
WORK = os.path.join(REPO, "target", "seal-harness")
SWIFT = os.path.join(WORK, "seal-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "seal_harness")
NICE = ["nice", "-n", "10"]

SEEDS = list(range(1, 17))
COUNT = 2500
QUICK_SEEDS = [1, 2]


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-seal", "--example", "seal_harness"],
       check=True, cwd=REPO, env=env)


def generate(seed, count):
    out = sh([RUST, "gen", str(seed), str(count)], check=True, capture_output=True, text=True).stdout
    return [line for line in out.split("\n") if line]


def run_side(binary, cases):
    with tempfile.TemporaryDirectory(prefix="seal-eq-") as tmp:
        path = os.path.join(tmp, "cases.txt")
        result = os.path.join(tmp, "out.txt")
        scratch = os.path.join(tmp, "scratch")
        os.mkdir(scratch)
        with open(path, "w") as f:
            f.write("\n".join(cases) + "\n")
        sh([binary, "run", path, result, scratch], check=True)
        with open(result) as f:
            return [line for line in f.read().split("\n") if line]


def canonical_number(value):
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, (int, float)):
        return repr(float(value))
    if isinstance(value, list):
        return [canonical_number(v) for v in value]
    if isinstance(value, dict):
        return {k: canonical_number(v) for k, v in value.items()}
    return value


def canonical_file(token):
    """A store file's hex, as comparable JSON when it is JSON."""
    if token in ("-", "~"):
        return token
    raw = bytes.fromhex(token)
    try:
        return "json:" + json.dumps(canonical_number(json.loads(raw)), sort_keys=True, ensure_ascii=False)
    except ValueError:
        return "raw:" + token


def normalise(case, result):
    command = case.split(" ", 1)[0]
    if command in ("pfile", "sfile") and " || " in result:
        results, files = result.split(" || ", 1)
        parts = dict(part.split("=", 1) for part in files.split(" "))
        return results + " || file=" + canonical_file(parts["file"]) + " backup=" + parts["backup"] + \
            " stamped=" + parts["stamped"]
    return result


def compare(cases, swift, rust, label):
    assert len(cases) == len(swift) == len(rust), (label, len(cases), len(swift), len(rust))
    differences = []
    tally = {}
    for case, s, r in zip(cases, swift, rust):
        command = case.split(" ", 1)[0]
        tally[command] = tally.get(command, 0) + 1
        if normalise(case, s) != normalise(case, r):
            differences.append((case, s, r))
    return tally, differences


def read_lines(path):
    with open(path) as f:
        return [line for line in f.read().split("\n") if line and not line.startswith("#")]


def divergence_cases():
    path = os.path.join(FIXTURES, "divergences.txt")
    return read_lines(path) if os.path.exists(path) else []


def run(quick):
    seeds = QUICK_SEEDS if quick else SEEDS
    total = 0
    failures = []
    tallies = {}
    for seed in seeds:
        cases = generate(seed, COUNT)
        swift = run_side(SWIFT, cases)
        rust = run_side(RUST, cases)
        tally, differences = compare(cases, swift, rust, f"seed {seed}")
        for k, v in tally.items():
            tallies[k] = tallies.get(k, 0) + v
        total += len(cases)
        failures += differences
    committed_path = os.path.join(FIXTURES, "cases.txt")
    committed = read_lines(committed_path) if os.path.exists(committed_path) else []
    if committed:
        swift = run_side(SWIFT, committed)
        rust = run_side(RUST, committed)
        tally, differences = compare(committed, swift, rust, "committed")
        total += len(committed)
        failures += differences
    print(f"{total} cases: " + ", ".join(f"{k} {v}" for k, v in sorted(tallies.items())))
    for case, s, r in failures[:40]:
        print(f"DIFF {case[:300]}\n  swift {s[:400]}\n  rust  {r[:400]}")
    print(f"{len(failures)} unexpected differences")

    divergences = divergence_cases()
    bad = 0
    if divergences:
        swift = run_side(SWIFT, divergences)
        rust = run_side(RUST, divergences)
        recorded = read_lines(os.path.join(FIXTURES, "divergences.swift.txt"))
        for case, s, r, want in zip(divergences, swift, rust, recorded):
            # Compared as the screen-free comparisons are: a store file Swift
            # wrote holds its keys in a per-process hash order.
            if normalise(case, s) != normalise(case, want):
                bad += 1
                print(f"DIVERGENCE CHANGED {case}\n  recorded {want}\n  swift    {s}")
            if normalise(case, s) == normalise(case, r):
                bad += 1
                print(f"DIVERGENCE GONE {case}: both print {s}")
        print(f"{len(divergences)} divergence cases, {bad} not as documented")
    tables = sh([SWIFT, "tables"], check=True, capture_output=True, text=True).stdout
    with open(os.path.join(FIXTURES, "numeric.txt")) as f:
        if f.read() != tables:
            bad += 1
            print("the Swift runtime's numericType table changed: update-goldens, then regenerate numeric_table.rs")
    return 1 if failures or bad else 0


def update_goldens():
    os.makedirs(FIXTURES, exist_ok=True)
    cases = read_lines(os.path.join(FIXTURES, "cases.txt"))
    swift = run_side(SWIFT, cases)
    with open(os.path.join(FIXTURES, "cases.swift.txt"), "w") as f:
        f.write("\n".join(swift) + "\n")
    divergences = divergence_cases()
    if divergences:
        with open(os.path.join(FIXTURES, "divergences.swift.txt"), "w") as f:
            f.write("\n".join(run_side(SWIFT, divergences)) + "\n")
    tables = sh([SWIFT, "tables"], check=True, capture_output=True, text=True).stdout
    with open(os.path.join(FIXTURES, "numeric.txt"), "w") as f:
        f.write(tables)
    print(f"wrote {len(swift)} committed results, {len(divergences)} divergence results, the numeric table")


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
