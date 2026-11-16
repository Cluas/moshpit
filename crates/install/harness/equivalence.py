#!/usr/bin/env python3
"""Equivalence harness: the app's Swift install code against its Rust port.

    equivalence.py [run] [--quick] [--no-build]
    equivalence.py update-goldens [--no-build]

`run` builds both front ends (harness/build-swift.sh over the app's
unchanged Moshpit/Services/Install sources and HostCapabilities, kept in
harness/swift/frozen since the app's went to the core, and
examples/install_harness.rs) and feeds both the same cases: seeded ones from
`install_harness gen` and the committed tests/fixtures/swift/cases.txt.

What the cases cover:

  scripts   both host scripts and their digests (the bytes that decide a
            reinstall);
  conf      the push conf of generated pairings: every relay, token, device
            token and environment shape, mint dates absent, fractional,
            negative, far away;
  merge/remove  all seven agents over generated configs: JSON objects with
            keys built to probe JSONSerialization's collation, numbers of
            every class it tells apart, strings with escapes, BOMs and emoji,
            hooks that are ours (any agent), the user's, near misses and
            junk, repeated keys, trailing commas, configs merged once already,
            empty and broken ones; Codex TOML with fences indented every way,
            blank tails, our own block;
  manifest/mread  manifests encoded from generated components and read back
            from their text and from bent copies of it;
  facts/caps/probe/install  the probes' parsers and the install commands;
  cmd       every HostCommands string over generated paths and values;
  flow      the installer against a simulated host (healthy, odd, wrong or
            missing checksums, manifests of every age, configs present or
            not): install a pairing, install and uninstall hooks, uninstall,
            the stamp and push self-tests, the status queries — every command
            the host saw and the report.

Every result must be byte-identical (manifest dates masked: Swift stamps
them with Date()). The accepted differences are the documented fixes in
harness/divergences.json; their minimal cases (tests/fixtures/swift/
divergences.txt, each under a `# <id>: …` line) must keep printing what
divergences.swift.txt recorded on the Swift side and something else on the
Rust side — except the cases of a fix since ported to the app (`"status":
"fixed"`), which must now print the same on both sides; divergences.swift.txt
keeps what the Swift code printed before the fix.
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
WORK = os.path.join(REPO, "target", "install-harness")
SWIFT = os.path.join(WORK, "install-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "install_harness")
NICE = ["nice", "-n", "10"]

SEEDS = list(range(1, 17))
COUNT = 2500
QUICK_SEEDS = [1, 2]


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
    return parts[0] + (" " + parts[1] if parts[0] in ("flow", "cmd") else "")


# Cases for a command the Swift code never had, so its binary can't answer
# them ("bad cmd …"): each a fix made on this side as one new command, its
# difference documented. Their Swift lines are the records
# divergences.swift.txt keeps, never run again.
SWIFT_LACKS = (
    "cmd writekeep ",  # config-mode (B352): a config written keeping its mode
    "cmd pushtest ",   # self-test-scope (B353): the push self-test for one pairing
    "cmd stamptest ",  # stamp-test-restores (B355): one command where Swift ran three
)


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
    tally, failures, total = {}, [], 0
    batches = [generate(seed, COUNT) for seed in seeds]
    batches.append(read_lines(os.path.join(FIXTURES, "cases.txt")))
    for cases in batches:
        if not cases:
            continue
        swift = run_side(SWIFT, cases)
        rust = run_side(RUST, cases)
        assert len(swift) == len(rust) == len(cases)
        for case, s, r in zip(cases, swift, rust):
            tally[kind(case)] = tally.get(kind(case), 0) + 1
            if s != r:
                failures.append((case, s, r))
        total += len(cases)
    print(f"{total} cases: " + ", ".join(f"{k} {v}" for k, v in sorted(tally.items())))
    for case, s, r in failures[:30]:
        print(f"DIFF {case[:300]}\n  swift {s[:500]}\n  rust  {r[:500]}")
    print(f"{len(failures)} unexpected differences")

    bad = 0
    divergences = read_lines(os.path.join(FIXTURES, "divergences.txt"))
    if divergences:
        recorded = read_lines(os.path.join(FIXTURES, "divergences.swift.txt"))
        swift = run_swift_divergences(divergences, recorded)
        rust = run_side(RUST, divergences)
        ported = ported_fixes()
        ids = divergence_ids(os.path.join(FIXTURES, "divergences.txt"))
        for fix, case, s, r, want in zip(ids, divergences, swift, rust, recorded):
            if fix in ported and swift_lacks(case):
                # Its port was checked while the Swift code had the command;
                # no Swift binary can answer it now.
                continue
            if fix in ported:
                if s != r:
                    bad += 1
                    print(f"DIVERGENCE NOT PORTED {fix}: fixed in the app, but Swift still differs for {case[:120]}")
                continue
            if s != want:
                bad += 1
                print(f"DIVERGENCE CHANGED {case[:120]}\n  recorded {want[:200]}\n  swift    {s[:200]}")
            if s == r:
                bad += 1
                print(f"DIVERGENCE GONE {case[:120]}")
        print(f"{len(divergences)} divergence cases, {bad} not as documented")
    return 1 if failures or bad else 0


def update_goldens():
    os.makedirs(FIXTURES, exist_ok=True)
    cases = read_lines(os.path.join(FIXTURES, "cases.txt"))
    with open(os.path.join(FIXTURES, "cases.swift.txt"), "w") as f:
        f.write("\n".join(run_side(SWIFT, cases)) + "\n")
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
