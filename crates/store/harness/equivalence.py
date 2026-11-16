#!/usr/bin/env python3
"""Equivalence harness: moshpit-store against the app as 1.0.8 shipped it.

    equivalence.py [run] [--quick] [--no-build] [--keep]
    equivalence.py update-goldens [--no-build] [--from-rust]

`run` builds the two front ends — harness/build-swift.sh's store-swift-108,
the app's storage sources as 1.0.8 shipped them (the release commit
8c10490a, kept in harness/swift108/frozen), and examples/store_harness.rs — and feeds them:

  A  generated cases to Rust;
  B  what Rust wrote in A, read back (and written again) by Rust;
  D  what Rust wrote in B, to 1.0.8: its one-piece decode must read every
     entry as Rust's own view says it should (`view108`), and 1.0.8
     re-saves it without loss;
  E  what 1.0.8 wrote in D, back: Rust reads it, and Rust's 1.0.8 view of
     it is what 1.0.8 itself read;
  P  push pairing files moshpit-seal's PushPairingFile wrote, decoded by
     1.0.8's PushPairing in one piece.

They are the upgrade guard in place of a device test: a phone on 1.0.8
updated to this build, and one rolled back to 1.0.8 (TestFlight keeps it a
tap away), lose nothing.

The app's own Swift stores are gone (batch C): they ran against Rust here
(phases A–C, D' and R) until then. What they printed for a mixed selection
of cases stays in tests/fixtures/swift/cases.swift.sha256, and what each
side printed for the documented divergences in divergences.{swift,rust}
.sha256: tests/swift_equivalence.rs holds Rust to them without swiftc.
Those Swift hashes are frozen; after a deliberate change to what the store
writes, `update-goldens --from-rust` records Rust's results in their place
(a golden of Rust's own from then on).

Results are compared in canonical form (`store_harness canon`): JSON members
sorted, Set-valued arrays sorted, every scalar's text kept, and Fix
(files-downgrade)'s two shapes of the files chip folded.

`update-goldens` rewrites what tests/swift_equivalence.rs checks that is
still made here: 1.0.8's view of what Rust writes for the golden cases
(hashed), and Rust's results for the divergence cases (hashed).
Everything runs niced, one process at a time.
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures", "swift")
WORK = os.path.join(REPO, "target", "store-harness")
SWIFT108 = os.path.join(WORK, "store-swift-108")
RUST = os.path.join(REPO, "target", "release", "examples", "store_harness")
NICE = ["nice", "-n", "10"]

SEEDS = list(range(1, 9))
COUNT = 1500
QUICK_SEEDS = [1]
QUICK_COUNT = 400
GOLDEN_VIEWS = 30

STORES = {"conn", "keys", "short", "theme", "apptheme", "settings", "hosts", "ready"}
VIEWS = {"conn", "keys", "short", "theme", "apptheme", "settings", "hosts"}


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh([os.path.join(HERE, "build-swift.sh"), WORK], check=True, stdout=subprocess.DEVNULL)
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-store", "--example", "store_harness"],
       check=True, cwd=REPO, env=env)


def lines(path):
    with open(path) as f:
        return [line for line in f.read().split("\n") if line]


def write(path, items):
    with open(path, "w") as f:
        f.write("\n".join(items) + "\n")


def generate(seed, count):
    out = sh([RUST, "gen", str(seed), str(count)], check=True, capture_output=True, text=True).stdout
    return [line for line in out.split("\n") if line]


def run_side(binary, cases, tmp, tag):
    """Results and finals of `cases` on one side."""
    path = os.path.join(tmp, tag + ".cases")
    write(path, cases)
    results, finals = path + ".out", path + ".finals"
    sh([binary, "run", path, results, finals], check=True)
    return lines(results), lines(finals)


def canonical(results, tmp, tag, strict=False):
    path = os.path.join(tmp, tag + ".raw")
    out = path + ".canon"
    write(path, results)
    sh([RUST, "canon-strict" if strict else "canon", path, out], check=True)
    return lines(out)


def readback_cases(finals, ops="fresh touch fresh"):
    out = []
    for line in finals:
        store, token = line.split(" ", 1)
        if store in STORES:
            out.append(f"{store} U {token} {ops}")
    return out


def view_lines(finals):
    return [line for line in finals if line.split(" ", 1)[0] in VIEWS]


def views_rust(items, tmp, tag):
    path = os.path.join(tmp, tag + ".views")
    write(path, items)
    sh([RUST, "view108", path, path + ".out"], check=True)
    return lines(path + ".out")


def views_108(items, tmp, tag):
    path = os.path.join(tmp, tag + ".108")
    write(path, items)
    sh([SWIFT108, "view", path, path + ".out", path + ".finals"], check=True)
    raw = lines(path + ".out")
    canon = canonical(raw, tmp, tag + ".108c")
    return canon, lines(path + ".finals")


def entry_ids(view):
    """The UUIDs a view names, for the no-loss check."""
    import re
    return set(re.findall(r"[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}", view))


def run_seed(seed, count, tmp, totals, failures):
    cases = generate(seed, count)
    _, rust_finals = run_side(RUST, cases, tmp, f"A{seed}.rust")
    totals.append(("A generated cases, Rust", len(cases), len(cases)))

    # B: what Rust wrote, read back and written again by Rust.
    b_from_rust = readback_cases(rust_finals)
    _, r_br_finals = run_side(RUST, b_from_rust, tmp, f"B{seed}.rust")
    totals.append(("B Rust-written, read by Rust", len(b_from_rust), len(b_from_rust)))

    # D: Rust-written (B's re-saves) to 1.0.8.
    rust_written = view_lines(r_br_finals)
    expected = views_rust(rust_written, tmp, f"D{seed}")
    seen, finals_108 = views_108(rust_written, tmp, f"D{seed}")
    same = lossless = readable = carried = 0
    for item, want, got in zip(rust_written, expected, seen):
        before, after = got.split(" || ")
        if want == "undecodable":
            # Rust carried something no build up to 1.0.8 reads; 1.0.8's own
            # one-piece decode drops the list (B8, before it was fixed).
            carried += 1
            if before != "undecodable":
                failures.append((f"D{seed} carried", item, want, before))
            continue
        readable += 1
        if before == want:
            same += 1
        else:
            failures.append((f"D{seed}", item, want, before))
        if entry_ids(before) <= entry_ids(after):
            lossless += 1
        else:
            failures.append((f"D{seed} loss", item, before, after))
    totals.append(("D 1.0.8 reads Rust's writes", readable, same))
    totals.append(("D 1.0.8 re-saves without loss", readable, lossless))
    totals.append(("D Rust carried an entry 1.0.8 can't read", carried, carried))

    # E: what 1.0.8 wrote, back.
    e_cases = readback_cases(finals_108, "fresh")
    run_side(RUST, e_cases, tmp, f"E{seed}.rust")
    totals.append(("E 1.0.8-written, read by Rust", len(e_cases), len(e_cases)))
    # Ids 1.0.8 minted while it wrote (built-ins it seeded) are new to both.
    known = [f"{a} {b.split(' ', 1)[1]}" for a, b in zip(finals_108, rust_written)]
    expected_after = views_rust(known, tmp, f"E{seed}v")
    same = 0
    for item, want, got in zip(view_lines(finals_108), expected_after, seen):
        after = got.split(" || ")[1]
        if want == after:
            same += 1
        else:
            failures.append((f"E{seed} view", item, want, after))
    totals.append(("E Rust's 1.0.8 view of 1.0.8's write", len(expected_after), same))


def run_pairings(seed, count, tmp, totals, failures):
    """Pairing files moshpit-seal wrote, decoded by 1.0.8 in one piece."""
    files, views = os.path.join(tmp, f"P{seed}.files"), os.path.join(tmp, f"P{seed}.rust")
    sh([RUST, "pairings", str(seed), str(count), files, views], check=True)
    seen = os.path.join(tmp, f"P{seed}.108")
    sh([SWIFT108, "pairings", files, seen], check=True)
    rust, swift = lines(views), lines(seen)
    same = sum(1 for r, s in zip(rust, swift) if r == s and r != "undecodable")
    for item, r, s in zip(lines(files), rust, swift):
        if r != s or r == "undecodable":
            failures.append((f"P{seed}", item, r, s))
    totals.append(("P 1.0.8 reads Rust's pairing files", len(rust), same))


def divergence_canonical(results, ids, tmp, tag):
    """Results of the divergence cases in canonical form: strict (the files
    chip unfolded) for files-downgrade, whose difference is that fold."""
    strict = canonical(results, tmp, tag + ".strict", strict=True)
    folded = canonical(results, tmp, tag + ".folded")
    return [s if i == "files-downgrade" else f for i, s, f in zip(ids, strict, folded)]


def sha(line):
    import hashlib
    return hashlib.sha256(line.encode()).hexdigest()


def update_goldens(tmp, from_rust):
    cases = lines(os.path.join(FIXTURES, "cases.txt"))
    if from_rust:
        # A deliberate change to what the store writes: Rust's results take
        # the frozen Swift ones' place.
        rust, _ = run_side(RUST, cases, tmp, "golden.rust.a")
        write(os.path.join(FIXTURES, "cases.swift.sha256"),
              [sha(l) for l in canonical(rust, tmp, "golden.rust.a")])
    # 1.0.8's view of what Rust writes, for the downgrade test.
    _, rust_finals = run_side(RUST, cases, tmp, "golden.rust")
    _, written = run_side(RUST, readback_cases(rust_finals, "fresh touch"), tmp, "golden.rust.b")
    items = view_lines(written)[:GOLDEN_VIEWS]
    write(os.path.join(FIXTURES, "views108.txt"), items)
    seen, _ = views_108(items, tmp, "golden")
    write(os.path.join(FIXTURES, "views108.swift.sha256"), [sha(line.split(" || ")[0]) for line in seen])
    div = lines(os.path.join(FIXTURES, "divergences.txt"))
    ids = lines(os.path.join(FIXTURES, "divergences.ids"))
    rust, _ = run_side(RUST, div, tmp, "golden.div.rust")
    write(os.path.join(FIXTURES, "divergences.rust.sha256"),
          [sha(l) for l in divergence_canonical(rust, ids, tmp, "golden.div.rust")])


def sweep_preferences():
    """cfprefsd can write a suite's empty plist back after it was removed:
    sweep what 1.0.8's suites (w5.store.<pid>.<n>) left."""
    import glob
    for path in glob.glob(os.path.expanduser("~/Library/Preferences/w5.store.*.plist")):
        try:
            os.remove(path)
        except OSError:
            pass


def main():
    args = sys.argv[1:]
    command = "run"
    if args and not args[0].startswith("--"):
        command = args.pop(0)
    if "--no-build" not in args:
        build()
    try:
        run(command, args)
    finally:
        sweep_preferences()


def run(command, args):
    with tempfile.TemporaryDirectory(prefix="store-eq-", dir=WORK) as tmp:
        if command == "update-goldens":
            update_goldens(tmp, "--from-rust" in args)
            print("goldens updated")
            return
        quick = "--quick" in args
        seeds, count = (QUICK_SEEDS, QUICK_COUNT) if quick else (SEEDS, COUNT)
        totals, failures = [], []
        for seed in seeds:
            run_seed(seed, count, tmp, totals, failures)
            run_pairings(seed, count // 5, tmp, totals, failures)
        summary = {}
        for name, n, same in totals:
            entry = summary.setdefault(name, [0, 0])
            entry[0] += n
            entry[1] += same
        print(f"{'phase':42} {'cases':>7} {'equal':>7}")
        for name, (n, same) in summary.items():
            print(f"{name:42} {n:7} {same:7}")
        if failures:
            with open(os.path.join(WORK, "failures.txt"), "w") as f:
                for phase, case, want, got in failures:
                    f.write(f"[{phase}] {case}\n{want}\n{got}\n")
            for phase, case, want, got in failures[:20]:
                print(f"\n[{phase}] {case[:300]}\n  rust/expected: {want[:600]}\n  1.0.8/got:     {got[:600]}")
            print(f"\n{len(failures)} failure(s); all of them in {os.path.join(WORK, 'failures.txt')}")
            sys.exit(1)
        print("no differences")


if __name__ == "__main__":
    main()
