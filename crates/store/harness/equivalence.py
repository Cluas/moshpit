#!/usr/bin/env python3
"""Equivalence harness: the app's Swift stores against moshpit-store.

    equivalence.py [run] [--quick] [--no-build] [--keep]
    equivalence.py update-goldens [--no-build]

`run` builds the four front ends — harness/build-swift.sh over the app's
unchanged storage sources as checked out (store-swift), the same store
classes on the app's Rust-store adapters (store-swift-rust: what the app
runs with MOSHPIT_RUST_STORE on), and as 1.0.8 shipped them
(store-swift-108), and examples/store_harness.rs — and feeds them:

  A  the same generated cases to Rust and to the current Swift;
  B  what each side wrote in A, read back (and written again) by both;
  C  what B wrote across sides, read by both: Swift→Rust→Swift and
     Rust→Swift→Rust;
  D  what Rust wrote, to 1.0.8: its one-piece decode must read every entry
     as Rust's own view says it should (`view108`), and 1.0.8 re-saves it;
  D' what the current Swift wrote in B, to 1.0.8: read as 1.0.8 reads
     Rust's write of the same input, and no list written over that 1.0.8
     keeps of Rust's (B153: a set holding the files chip was undecodable
     there, and 1.0.8 wrote its built-ins over every custom chip);
  E  what 1.0.8 wrote in D, read by Rust and the current Swift alike, and
     by Rust's 1.0.8 view as 1.0.8 itself read it;
  P  push pairing files moshpit-seal's PushPairingFile wrote, decoded by
     1.0.8's PushPairing in one piece (seal's own harness holds the file to
     the current Swift).

and, for the app's Rust-store adapters (the switch on):

  R  the same cases through the store classes on the adapters as through
     them on their own Swift code — one API, two sides; what each side
     wrote read by the other; Swift's writes read and touched by the
     adapters and read by Swift again (switching on with data, using the
     app, switching off: nothing lost, every store read as if Swift had
     done it all); what the adapters wrote, to 1.0.8; push pairings through
     the adapter and Swift's file alike, each reading the other's, and the
     adapter's files decoded by 1.0.8.

Results are compared in canonical form (`store_harness canon`): JSON members
sorted (the app's own order is a Swift Dictionary's, seeded per process),
Set-valued arrays sorted, every scalar's text kept, and Fix
(files-downgrade)'s two shapes of the files chip folded. Every other
difference fails, except the documented ones in harness/divergences.json,
which the generator keeps clear of and which have minimal cases of their own
in tests/fixtures/swift/divergences.txt (one id per case in divergences.ids;
the hashes of both sides' results in divergences.{swift,rust}.sha256: Swift's
must reappear, Rust's too, and the two must differ) — except the cases of a
fix since ported to the app (`"status": "fixed"`), which must now print the
same on both sides; divergences.swift.sha256 keeps what the Swift printed
before the fix.

`update-goldens` rewrites the committed fixtures tests/swift_equivalence.rs
checks without swiftc: a mixed selection of generated cases with the SHA-256
of each canonical Swift result line (the results themselves run to
megabytes), 1.0.8's view of what Rust writes for them (hashed too), and the
divergence cases' results, hashed too. Everything runs niced, one process at a time; the
Swift side's UserDefaults suites are removed as it goes.
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
SWIFT = os.path.join(WORK, "store-swift")
SWIFT_RUST = os.path.join(WORK, "store-swift-rust")
SWIFT108 = os.path.join(WORK, "store-swift-108")
RUST = os.path.join(REPO, "target", "release", "examples", "store_harness")
NICE = ["nice", "-n", "10"]

SEEDS = list(range(1, 9))
COUNT = 1500
QUICK_SEEDS = [1]
QUICK_COUNT = 400
GOLDEN_SEED = 1
GOLDEN_COUNT = 150
GOLDEN_SHORTCUTS = 8
GOLDEN_VIEWS = 30

STORES = {"conn", "keys", "short", "theme", "apptheme", "settings", "hosts", "ready"}
VIEWS = {"conn", "keys", "short", "theme", "apptheme", "settings", "hosts"}


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    # The library store-swift-rust links, built for this Mac.
    sh(["cargo", "build", "--quiet", "-p", "moshpit-ffi"], check=True, cwd=REPO, env=env)
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


def compare(phase, cases, rust, swift, tmp, failures):
    rust_c = canonical(rust, tmp, phase + ".rust")
    swift_c = canonical(swift, tmp, phase + ".swift")
    same = sum(1 for r, s in zip(rust_c, swift_c) if r == s)
    for case, r, s in zip(cases, rust_c, swift_c):
        if r != s:
            failures.append((phase, case, r, s))
    raw_same = sum(1 for r, s in zip(rust, swift) if r == s)
    # Without the files-chip fold: since B153 the two sides write and read
    # the chip alike, so every case must be equal unfolded too.
    rust_u = canonical(rust, tmp, phase + ".rust.u", strict=True)
    swift_u = canonical(swift, tmp, phase + ".swift.u", strict=True)
    unfolded = sum(1 for r, s in zip(rust_u, swift_u) if r == s)
    return same, raw_same, unfolded


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


def ordinal_ids(view):
    """`view` with each UUID replaced by the order of its first appearance —
    the ones hex-encoded in a snapshot's stored bytes too."""
    import re
    hex_digit = "(?:3[0-9]|4[1-6])"
    hex_uuid = "2d".join(hex_digit + "{%d}" % n for n in (8, 4, 4, 4, 12))
    view = re.sub(hex_uuid, lambda m: bytes.fromhex(m.group(0)).decode(), view)
    seen = {}
    return re.sub(r"[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}",
                  lambda m: "id%d" % seen.setdefault(m.group(0), len(seen)), view)


def entry_ids(view):
    """The UUIDs a view names, for the no-loss check."""
    import re
    return set(re.findall(r"[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}", view))


def run_seed(seed, count, tmp, totals, failures):
    cases = generate(seed, count)
    rust, rust_finals = run_side(RUST, cases, tmp, f"A{seed}.rust")
    swift, swift_finals = run_side(SWIFT, cases, tmp, f"A{seed}.swift")
    same, raw, unfolded = compare(f"A{seed}", cases, rust, swift, tmp, failures)
    totals.append(("A same input", len(cases), same, raw, unfolded))

    b_from_swift = readback_cases(swift_finals)
    b_from_rust = readback_cases(rust_finals)
    r_bs, r_bs_finals = run_side(RUST, b_from_swift, tmp, f"B{seed}s.rust")
    s_bs, s_bs_finals = run_side(SWIFT, b_from_swift, tmp, f"B{seed}s.swift")
    same, raw, unfolded = compare(f"B{seed}s", b_from_swift, r_bs, s_bs, tmp, failures)
    totals.append(("B Swift-written, read by both", len(b_from_swift), same, raw, unfolded))
    r_br, r_br_finals = run_side(RUST, b_from_rust, tmp, f"B{seed}r.rust")
    s_br, s_br_finals = run_side(SWIFT, b_from_rust, tmp, f"B{seed}r.swift")
    same, raw, unfolded = compare(f"B{seed}r", b_from_rust, r_br, s_br, tmp, failures)
    totals.append(("B Rust-written, read by both", len(b_from_rust), same, raw, unfolded))

    c_srs = readback_cases(r_bs_finals, "fresh")
    r_c1, _ = run_side(RUST, c_srs, tmp, f"C{seed}a.rust")
    s_c1, _ = run_side(SWIFT, c_srs, tmp, f"C{seed}a.swift")
    same, raw, unfolded = compare(f"C{seed}a", c_srs, r_c1, s_c1, tmp, failures)
    totals.append(("C Swift→Rust→Swift", len(c_srs), same, raw, unfolded))
    c_rsr = readback_cases(s_br_finals, "fresh")
    r_c2, _ = run_side(RUST, c_rsr, tmp, f"C{seed}b.rust")
    s_c2, _ = run_side(SWIFT, c_rsr, tmp, f"C{seed}b.swift")
    same, raw, unfolded = compare(f"C{seed}b", c_rsr, r_c2, s_c2, tmp, failures)
    totals.append(("C Rust→Swift→Rust", len(c_rsr), same, raw, unfolded))

    # D: Rust-written (B's re-saves) to 1.0.8.
    rust_written = view_lines(r_bs_finals + r_br_finals)
    expected = views_rust(rust_written, tmp, f"D{seed}")
    seen, finals_108 = views_108(rust_written, tmp, f"D{seed}")
    decoded = same = lossless = 0
    readable = carried = 0
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
        if before != "undecodable":
            decoded += 1
        if entry_ids(before) <= entry_ids(after):
            lossless += 1
        else:
            failures.append((f"D{seed} loss", item, before, after))
    totals.append(("D 1.0.8 reads Rust's writes", readable, same, decoded, same))
    totals.append(("D 1.0.8 re-saves without loss", readable, lossless, lossless, lossless))
    totals.append(("D Rust carried an entry 1.0.8 can't read", carried, carried, carried, carried))

    # D': what the current Swift wrote, to 1.0.8 — the downgrade TestFlight
    # keeps a tap away. B's re-saves of one input on both sides hold the same
    # entries, so 1.0.8 must read the Swift's as it reads Rust's; one entry
    # it can't decode and it reads nothing, and its shortcut store then
    # writes its built-ins over the set (B153, Fix (files-downgrade)).
    swift_written = view_lines(s_bs_finals + s_br_finals)
    if len(swift_written) != len(rust_written):
        failures.append((f"D'{seed} pairing", "-", str(len(rust_written)), str(len(swift_written))))
    seen_swift, _ = views_108(swift_written, tmp, f"Ds{seed}")
    alike = overwritten = 0
    for item, theirs, ours in zip(swift_written, seen, seen_swift):
        rust_before = theirs.split(" || ")[0]
        before, after = ours.split(" || ")
        # Built-ins a side seeded in A carry ids it minted at random: ids
        # are compared by where they first appear.
        if ordinal_ids(before) == ordinal_ids(rust_before):
            alike += 1
        else:
            failures.append((f"D'{seed}", item, rust_before, before))
        # The list 1.0.8 read as nothing and wrote its own over — where it
        # kept the same entries as Rust wrote them.
        if before == "undecodable" and after != "undecodable" and rust_before != "undecodable":
            overwritten += 1
    totals.append(("D' 1.0.8 reads Swift's writes as Rust's", len(swift_written), alike, alike, alike))
    kept = len(swift_written) - overwritten
    totals.append(("D' 1.0.8 keeps what it keeps of Rust's", len(swift_written), kept, kept, kept))

    # E: what 1.0.8 wrote, back.
    e_cases = readback_cases(finals_108, "fresh")
    r_e, _ = run_side(RUST, e_cases, tmp, f"E{seed}.rust")
    s_e, _ = run_side(SWIFT, e_cases, tmp, f"E{seed}.swift")
    same, raw, unfolded = compare(f"E{seed}", e_cases, r_e, s_e, tmp, failures)
    totals.append(("E 1.0.8-written, read by both", len(e_cases), same, raw, unfolded))
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
    totals.append(("E Rust's 1.0.8 view of 1.0.8's write", len(expected_after), same, same, same))

    run_adapters(seed, cases, swift, swift_finals, b_from_swift, s_bs, s_bs_finals, tmp, totals, failures)


def run_adapters(seed, cases, swift, swift_finals, b_from_swift, s_bs, s_bs_finals, tmp, totals, failures):
    """R: the store classes on the app's Rust-store adapters, against the
    same classes on their own Swift code."""
    adapters, adapter_finals = run_side(SWIFT_RUST, cases, tmp, f"R{seed}.adapters")
    same, raw, unfolded = compare(f"R{seed}", cases, adapters, swift, tmp, failures)
    totals.append(("R adapters vs Swift, same input", len(cases), same, raw, unfolded))

    a_bs, a_bs_finals = run_side(SWIFT_RUST, b_from_swift, tmp, f"R{seed}bs.adapters")
    same, raw, unfolded = compare(f"R{seed}bs", b_from_swift, a_bs, s_bs, tmp, failures)
    totals.append(("R Swift-written, read by adapters", len(b_from_swift), same, raw, unfolded))
    b_from_adapters = readback_cases(adapter_finals)
    a_ba, a_ba_finals = run_side(SWIFT_RUST, b_from_adapters, tmp, f"R{seed}ba.adapters")
    s_ba, _ = run_side(SWIFT, b_from_adapters, tmp, f"R{seed}ba.swift")
    same, raw, unfolded = compare(f"R{seed}ba", b_from_adapters, a_ba, s_ba, tmp, failures)
    totals.append(("R adapter-written, read by Swift", len(b_from_adapters), same, raw, unfolded))

    # The switch on with Swift's data, the app used (read, touched), the
    # switch off: Swift reads what the adapters left as it reads what Swift
    # itself left after the same use.
    on_off = readback_cases(a_bs_finals, "fresh")
    swift_only = readback_cases(s_bs_finals, "fresh")
    # A built-in either side injected while it touched the set carries an
    # id it minted at random, and the two inputs differ by those ids:
    # they are compared by where they first appear.
    s_on_off, _ = run_side(SWIFT, on_off, tmp, f"R{seed}c.onoff")
    s_only, _ = run_side(SWIFT, swift_only, tmp, f"R{seed}c.swift")
    on_off_c = canonical(s_on_off, tmp, f"R{seed}c.onoff")
    only_c = canonical(s_only, tmp, f"R{seed}c.swift")
    same = 0
    for case, a, b in zip(on_off, on_off_c, only_c):
        if ordinal_ids(a) == ordinal_ids(b):
            same += 1
        else:
            failures.append((f"R{seed}c", case, b, a))
    raw = sum(1 for a, b in zip(s_on_off, s_only) if a == b)
    totals.append(("R switch on, use, off: Swift reads all", len(on_off), same, raw, same))

    # What the adapters wrote, to 1.0.8.
    written = view_lines(a_bs_finals + a_ba_finals)
    expected = views_rust(written, tmp, f"R{seed}d")
    seen, _ = views_108(written, tmp, f"R{seed}d")
    readable = same = lossless = carried = 0
    for item, want, got in zip(written, expected, seen):
        before, after = got.split(" || ")
        if want == "undecodable":
            carried += 1
            if before != "undecodable":
                failures.append((f"R{seed}d carried", item, want, before))
            continue
        readable += 1
        if before == want:
            same += 1
        else:
            failures.append((f"R{seed}d", item, want, before))
        if entry_ids(before) <= entry_ids(after):
            lossless += 1
        else:
            failures.append((f"R{seed}d loss", item, before, after))
    totals.append(("R 1.0.8 reads the adapters' writes", readable, same, same, same))
    totals.append(("R 1.0.8 re-saves them without loss", readable, lossless, lossless, lossless))
    totals.append(("R adapters carried what 1.0.8 can't read", carried, carried, carried, carried))


def run_adapter_pairings(seed, count, tmp, totals, failures):
    """Pairings through the app's adapter and Swift's file alike (the
    adapter binary checks each reads the other's); 1.0.8 must read the
    adapter's files as it reads the Swift's, every one."""
    files, swift_files = os.path.join(tmp, f"RP{seed}.files"), os.path.join(tmp, f"RP{seed}.swift")
    result = sh([SWIFT_RUST, "pairings", str(seed), str(count), files, swift_files],
                capture_output=True, text=True)
    if result.returncode != 0:
        failures.append((f"RP{seed}", "-", "the two sides agree", result.stderr.strip()[-600:]))
        return
    ours, theirs = os.path.join(tmp, f"RP{seed}.108"), os.path.join(tmp, f"RP{seed}.swift.108")
    sh([SWIFT108, "pairings", files, ours], check=True)
    sh([SWIFT108, "pairings", swift_files, theirs], check=True)
    ours, theirs = lines(ours), lines(theirs)
    same = sum(1 for r, s in zip(ours, theirs) if r == s and r != "undecodable")
    for item, r, s in zip(lines(files), ours, theirs):
        if r != s or r == "undecodable":
            failures.append((f"RP{seed}", item[:200], s, r))
    byte_equal = sum(1 for a, b in zip(lines(files), lines(swift_files)) if a == b)
    totals.append(("R 1.0.8 reads adapter pairing files as Swift's", len(ours), same, byte_equal, same))


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
    totals.append(("P 1.0.8 reads Rust's pairing files", len(rust), same, same, same))


def divergence_canonical(results, ids, tmp, tag):
    """Results of the divergence cases in canonical form: strict (the files
    chip unfolded) for files-downgrade, whose difference is that fold."""
    strict = canonical(results, tmp, tag + ".strict", strict=True)
    folded = canonical(results, tmp, tag + ".folded")
    return [s if i == "files-downgrade" else f for i, s, f in zip(ids, strict, folded)]


def ported_fixes():
    """The fixes since ported to the app (`"status": "fixed"` in
    divergences.json): their cases must now print the same on both sides."""
    import json
    with open(os.path.join(HERE, "divergences.json")) as f:
        return {fix["id"] for fix in json.load(f)["fixes"] if fix.get("status") == "fixed"}


def check_divergences(tmp, failures):
    cases = lines(os.path.join(FIXTURES, "divergences.txt"))
    ids = lines(os.path.join(FIXTURES, "divergences.ids"))
    swift_expected = lines(os.path.join(FIXTURES, "divergences.swift.sha256"))
    rust_expected = lines(os.path.join(FIXTURES, "divergences.rust.sha256"))
    ported = ported_fixes()
    rust, _ = run_side(RUST, cases, tmp, "div.rust")
    swift, _ = run_side(SWIFT, cases, tmp, "div.swift")
    rust_c = divergence_canonical(rust, ids, tmp, "div.rust")
    swift_c = divergence_canonical(swift, ids, tmp, "div.swift")
    if len(swift_expected) != len(cases) or len(rust_expected) != len(cases):
        failures.append(("divergence goldens", "-", f"{len(cases)} cases", "run update-goldens"))
    for case, i, s, want in zip(cases, ids, swift_c, swift_expected):
        # A ported fix's Swift line is the record of the app before it.
        if i not in ported and sha(s) != want:
            failures.append((f"divergence {i} (Swift changed)", case, want, s))
    for case, i, r, s, want in zip(cases, ids, rust_c, swift_c, rust_expected):
        if sha(r) != want:
            failures.append((f"divergence {i} (Rust changed)", case, want, r))
        if i in ported:
            if r != s:
                failures.append((f"divergence {i} (fixed in the app, but Swift still differs)", case, r, s))
        elif r == s:
            failures.append((f"divergence {i} (no longer differs)", case, r, s))
    return len(cases)


def sha(line):
    import hashlib
    return hashlib.sha256(line.encode()).hexdigest()


def golden_cases():
    """A small, mixed selection: every kind of case, few of the (large)
    shortcut ones."""
    picked, shortcuts = [], 0
    for case in generate(GOLDEN_SEED, 600):
        if case.startswith("short "):
            if shortcuts >= GOLDEN_SHORTCUTS:
                continue
            shortcuts += 1
        picked.append(case)
        if len(picked) == GOLDEN_COUNT:
            break
    return picked


def update_goldens(tmp):
    cases = golden_cases()
    write(os.path.join(FIXTURES, "cases.txt"), cases)
    swift, _ = run_side(SWIFT, cases, tmp, "golden.swift")
    write(os.path.join(FIXTURES, "cases.swift.sha256"), [sha(l) for l in canonical(swift, tmp, "golden.swift")])
    # 1.0.8's view of what Rust writes, for the downgrade test.
    _, rust_finals = run_side(RUST, cases, tmp, "golden.rust")
    _, written = run_side(RUST, readback_cases(rust_finals, "fresh touch"), tmp, "golden.rust.b")
    items = view_lines(written)[:GOLDEN_VIEWS]
    write(os.path.join(FIXTURES, "views108.txt"), items)
    seen, _ = views_108(items, tmp, "golden")
    write(os.path.join(FIXTURES, "views108.swift.sha256"), [sha(line.split(" || ")[0]) for line in seen])
    div = lines(os.path.join(FIXTURES, "divergences.txt"))
    ids = lines(os.path.join(FIXTURES, "divergences.ids"))
    swift, _ = run_side(SWIFT, div, tmp, "golden.div.swift")
    # A fix ported to the app keeps its pre-fix Swift line: the record of
    # what the case tells the fix apart from.
    ported = ported_fixes()
    now = [sha(l) for l in divergence_canonical(swift, ids, tmp, "golden.div.swift")]
    before = lines(os.path.join(FIXTURES, "divergences.swift.sha256"))
    before += [None] * (len(now) - len(before))
    recorded = []
    for i, new, old in zip(ids, now, before):
        if i in ported and old is None:
            sys.exit(f"update-goldens: no record of the Swift before the fix for {i}")
        recorded.append(old if i in ported else new)
    write(os.path.join(FIXTURES, "divergences.swift.sha256"), recorded)
    rust, _ = run_side(RUST, div, tmp, "golden.div.rust")
    write(os.path.join(FIXTURES, "divergences.rust.sha256"),
          [sha(l) for l in divergence_canonical(rust, ids, tmp, "golden.div.rust")])


def sweep_preferences():
    """cfprefsd can write a suite's empty plist back after it was removed:
    sweep what the Swift side's suites (w5.store.<pid>.<n>) left."""
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
            update_goldens(tmp)
            print("goldens updated")
            return
        quick = "--quick" in args
        seeds, count = (QUICK_SEEDS, QUICK_COUNT) if quick else (SEEDS, COUNT)
        totals, failures = [], []
        for seed in seeds:
            run_seed(seed, count, tmp, totals, failures)
            run_pairings(seed, count // 5, tmp, totals, failures)
            run_adapter_pairings(seed, count // 5, tmp, totals, failures)
        divergences = check_divergences(tmp, failures)
        summary = {}
        for name, n, same, raw, unfolded in totals:
            entry = summary.setdefault(name, [0, 0, 0, 0])
            entry[0] += n
            entry[1] += same
            entry[2] += raw
            entry[3] += unfolded
        print(f"{'phase':42} {'cases':>7} {'equal':>7} {'byte-equal':>10} {'unfolded':>8}")
        for name, (n, same, raw, unfolded) in summary.items():
            print(f"{name:42} {n:7} {same:7} {raw:10} {unfolded:8}")
        print(f"divergence cases: {divergences}")
        if failures:
            with open(os.path.join(WORK, "failures.txt"), "w") as f:
                for phase, case, want, got in failures:
                    f.write(f"[{phase}] {case}\n{want}\n{got}\n")
            for phase, case, want, got in failures[:20]:
                print(f"\n[{phase}] {case[:300]}\n  rust/expected: {want[:600]}\n  swift/got:     {got[:600]}")
            print(f"\n{len(failures)} failure(s); all of them in {os.path.join(WORK, 'failures.txt')}")
            sys.exit(1)
        print("no differences")


if __name__ == "__main__":
    main()
