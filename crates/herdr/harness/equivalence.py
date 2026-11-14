#!/usr/bin/env python3
"""Equivalence harness: the app's Swift herdr code against its Rust port.

    equivalence.py [run] [--quick] [--no-build] [--seeds N] [--per-family N]
    equivalence.py update-goldens [--no-build]
    equivalence.py bench [--no-build]

`run` builds both front ends (harness/build-swift.sh over the app's
UNCHANGED herdr sources, and examples/herdr_harness.rs) and feeds both the
same cases, one result line each:

  base         tests/fixtures/corpus/base.txt: every Swift test's inputs, the
               app's real herdr captures, and the edges of Foundation's JSON,
               base64 and string rules (harness/corpus.py base);
  recorded     tests/fixtures/corpus/recorded.txt: frames, snapshots, status
               and socket events from a live herdr 0.9.5 (harness/record.py);
  fuzz         seeded random cases, N per family and seed (harness/fuzz.py):
               JSON documents and mutations, noisy objects, frame streams cut
               by random plans, single lines, snapshots, socket streams with
               pending requests, versions, command lines, and the control
               client driven through its real Swift implementation;
  divergences  tests/fixtures/corpus/divergences.txt: one or more minimal
               cases per fix in harness/divergences.json.

Every base and recorded case must print the same on both sides (request
lines are compared as JSON plus their exact string tokens: Swift's
JSONEncoder writes keys in a random order). A fuzz case may differ only when
fuzz.tags() names a documented divergence it can trigger; each difference is
counted against its tags. Each divergence case must print its recorded Rust
output (divergences.rust.txt) on the Rust side and something else on the
Swift side (divergences.swift.txt), or trap there (`trap`), which proves the
case tells the fix from the old behaviour. When the Swift binary dies on a
batch, the batch is split until the trapping cases are found, and each is
reported. A fix since ported to the app (`"status": "fixed"` in
divergences.json) is the other way round: its cases must now print the same
on both sides, and divergences.swift.txt keeps what the Swift code printed
before the fix.

`update-goldens` rewrites base.swift.txt, recorded.swift.txt and the two
divergence outputs from the current builds (keeping the pre-fix Swift lines
of fixed divergences). Everything runs niced, one process at a time.
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
CORPUS = os.path.join(CRATE, "tests", "fixtures", "corpus")
WORK = os.path.join(REPO, "target", "herdr-harness")
SWIFT = os.path.join(WORK, "herdr-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "herdr_harness")
NICE = ["nice", "-n", "10"]
sys.path.insert(0, HERE)

import fuzz  # noqa: E402


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL,
       stderr=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-herdr", "--example", "herdr_harness"],
       check=True, cwd=REPO, env=env)


def read_cases(path):
    with open(path, encoding="utf-8") as f:
        return [line for line in f.read().split("\n") if line.strip() and not line.startswith("#")]


def run_binary(binary, cases):
    """(returncode, output lines) for a batch of cases."""
    os.makedirs(WORK, exist_ok=True)
    path = os.path.join(WORK, f"batch-{os.getpid()}.txt")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(cases) + "\n")
    done = sh([binary, "cases", path], capture_output=True)
    lines = done.stdout.decode("utf-8", "replace").split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    return done.returncode, lines


def run_swift(cases):
    """Swift outputs, one per case; `trap` for a case that kills the binary
    (found by splitting the batch)."""
    code, lines = run_binary(SWIFT, cases)
    if code == 0 and len(lines) == len(cases):
        return lines
    if len(cases) == 1:
        return ["trap"]
    half = len(cases) // 2
    return run_swift(cases[:half]) + run_swift(cases[half:])


def run_rust(cases):
    code, lines = run_binary(RUST, cases)
    if code != 0 or len(lines) != len(cases):
        sys.exit(f"the Rust harness failed (exit {code}, {len(lines)} lines for {len(cases)} cases)")
    return lines


def unquote(text):
    """A harness string literal back to its value."""
    assert text.startswith('"') and text.endswith('"'), text
    out, i, body = [], 0, text[1:-1]
    while i < len(body):
        if body[i] == "\\":
            if body[i + 1] in '"\\':
                out.append(body[i + 1])
                i += 2
            else:
                end = body.index("}", i)
                out.append(chr(int(body[i + 3:end], 16)))
                i = end + 1
        else:
            out.append(body[i])
            i += 1
    return "".join(out)


STRING_TOKEN = None


def same(case, swift, rust):
    """Whether two outputs agree; request lines as JSON."""
    if swift == rust:
        return True
    kind = case.split(" ", 1)[0]
    if kind not in ("request", "subscribe"):
        return False
    import re
    global STRING_TOKEN
    STRING_TOKEN = STRING_TOKEN or re.compile(r'"(?:[^"\\]|\\.)*"')
    a, b = unquote(swift.split(" ", 1)[1]), unquote(rust.split(" ", 1)[1])
    try:
        return json.loads(a) == json.loads(b) and sorted(STRING_TOKEN.findall(a)) == sorted(STRING_TOKEN.findall(b))
    except ValueError:
        return False


def first_difference(swift, rust):
    at = next((i for i in range(min(len(swift), len(rust))) if swift[i] != rust[i]), min(len(swift), len(rust)))
    return f"swift …{swift[max(0, at - 60):at + 100]!r}\n          rust  …{rust[max(0, at - 60):at + 100]!r}"


def check_exact(name, cases, swift, rust):
    bad = [(c, s, r) for c, s, r in zip(cases, swift, rust) if not same(c, s, r)]
    for case, s, r in bad[:10]:
        print(f"  DIFF {name}: {case[:120]}\n          {first_difference(s, r)}")
    print(f"{name}: {len(cases)} cases, {len(cases) - len(bad)} identical")
    return not bad


def check_fuzz(seeds, per_family):
    """Every difference must be explained by a tag."""
    counts = {}
    total = identical = tagged_cases = 0
    unexplained = []
    traps = []
    for seed in seeds:
        cases = fuzz.cases(seed, per_family)
        swift, rust = run_swift(cases), run_rust(cases)
        for case, s, r in zip(cases, swift, rust):
            total += 1
            tags = fuzz.tags(case)
            if tags:
                tagged_cases += 1
            if s == "trap":
                traps.append((seed, case))
                continue
            if same(case, s, r):
                identical += 1
                continue
            if not tags:
                unexplained.append((seed, case, s, r))
            for tag in tags:
                counts[tag] = counts.get(tag, 0) + 1
    print(f"fuzz: {len(seeds)} seeds × {len(fuzz.FAMILIES)} families × {per_family}: {total} cases, "
          f"{identical} identical, {total - tagged_cases} untagged (all must be identical)")
    for tag in sorted(counts):
        print(f"  differs under {tag}: {counts[tag]}")
    for seed, case in traps[:10]:
        print(f"  TRAP (Swift crashed) seed {seed}: {case[:200]}")
    for seed, case, s, r in unexplained[:10]:
        print(f"  UNEXPLAINED seed {seed}: {case[:160]}\n          {first_difference(s, r)}")
    return not unexplained and not traps, total, identical


def check_divergences(update=False):
    cases = read_cases(os.path.join(CORPUS, "divergences.txt"))
    with open(os.path.join(HERE, "divergences.json"), encoding="utf-8") as f:
        fixes = {fix["id"]: fix for fix in json.load(f)["fixes"]}
    labels = []
    plain = []
    for line in cases:
        label, case = line.split(" ", 1)
        if label not in fixes:
            sys.exit(f"divergences.txt: {label} is not in divergences.json")
        labels.append(label)
        plain.append(case)
    swift, rust = run_swift(plain), run_rust(plain)
    rust_path = os.path.join(CORPUS, "divergences.rust.txt")
    swift_path = os.path.join(CORPUS, "divergences.swift.txt")
    ported = [fixes[label].get("status") == "fixed" for label in labels]
    if update:
        write(rust_path, rust)
        before = read_lines(swift_path)
        write(swift_path, [old if fixed else s for fixed, s, old in zip(ported, swift, before)])
    recorded = read_lines(rust_path)
    ok = True
    for label, fixed, case, s, r, want in zip(labels, ported, plain, swift, rust, recorded):
        if r != want:
            print(f"  DIVERGENCE {label}: Rust no longer prints its recorded fix for {case[:100]}")
            ok = False
        if fixed and not same(case, s, r):
            print(f"  DIVERGENCE {label}: fixed in the app, but Swift still differs for {case[:100]}")
            ok = False
        if not fixed and same(case, s, r):
            print(f"  DIVERGENCE {label}: Swift prints the same as Rust for {case[:100]}")
            ok = False
    print(f"divergences: {len(plain)} cases over {len(set(labels))} fixes ({sum(ported)} cases of fixes "
          f"ported to the app), {sum(1 for s in swift if s == 'trap')} Swift traps, all as documented: {ok}")
    return ok


def check_buffer_cap():
    """Fix buffer-cap, generated (its input is 32 MiB of hex): a chunk that
    ends a 16 MiB line keeps the lines after it on the Rust side only."""
    cap = 16 * 1024 * 1024
    junk = b"x" * (cap - 10)
    frame = b'{"type":"terminal.frame","seq":1,"width":80,"height":24,"full":true,"bytes":"YQ=="}'
    reply = b'{"id":"mp1","result":{}}'
    cases = [f"frames fixed:{len(junk)} {fuzz.x(junk + b'x' * 20 + b'\n' + frame + b'\n' + frame + b'\n')}",
             f"socket fixed:{len(junk)} 1 {fuzz.x(junk + b'x' * 20 + b'\n' + reply + b'\n')}"]
    swift, rust = run_swift(cases), run_rust(cases)
    want = ["frames [screen(1,80,24,true,b:61) screen(1,80,24,true,b:61)]",
            'socket ready=false events[] replies[mp1=ok("{\\"id\\":\\"mp1\\",\\"result\\":{}}")]']
    ok = rust == want and all(s != r for s, r in zip(swift, rust))
    print(f"buffer-cap: Rust keeps the lines, Swift drops them: {ok}")
    if not ok:
        print(f"  swift {[s[:120] for s in swift]}\n  rust  {[r[:120] for r in rust]}")
    return ok


def read_lines(path):
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    return lines[:-1] if lines and lines[-1] == "" else lines


def write(path, lines):
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


def corpus_check(name, update):
    path = os.path.join(CORPUS, f"{name}.txt")
    if not os.path.exists(path):
        print(f"{name}: no corpus")
        return True
    cases = read_cases(path)
    swift, rust = run_swift(cases), run_rust(cases)
    if update:
        write(os.path.join(CORPUS, f"{name}.swift.txt"), swift)
    return check_exact(name, cases, swift, rust)


def main():
    args = sys.argv[1:]
    mode = args[0] if args and not args[0].startswith("--") else "run"
    if "--no-build" not in args:
        build()
    if mode == "update-goldens":
        ok = all([corpus_check("base", True), corpus_check("recorded", True), check_divergences(update=True)])
        sys.exit(0 if ok else 1)
    if mode == "bench":
        return bench()
    quick = "--quick" in args
    seeds = int(args[args.index("--seeds") + 1]) if "--seeds" in args else (2 if quick else 40)
    seed_list = range(1, seeds + 1)
    if "--seed-list" in args:
        seed_list = [int(s) for s in args[args.index("--seed-list") + 1].split(",")]
    per_family = int(args[args.index("--per-family") + 1]) if "--per-family" in args else (50 if quick else 120)
    started = time.time()
    ok = corpus_check("base", False)
    ok = corpus_check("recorded", False) and ok
    ok = check_divergences() and ok
    ok = check_buffer_cap() and ok
    fuzz_ok, _, _ = check_fuzz(seed_list, per_family)
    ok = fuzz_ok and ok
    print(f"{'PASS' if ok else 'FAIL'} in {time.time() - started:.0f} s")
    sys.exit(0 if ok else 1)


def bench():
    """The frame parser alone, in process, both sides: real-sized frames
    (2 000, 40 B to 4 KB of screen each) and many small ones (50 000 cursor
    moves), fed whole, in 64 KiB and in 4 KiB pieces."""
    import base64
    import random
    rng = random.Random(7)

    def line(seq, payload):
        body = {"bytes": base64.b64encode(payload).decode(), "encoding": "ansi", "full": seq % 50 == 0,
                "height": 40, "seq": seq, "type": "terminal.frame", "width": 120}
        return json.dumps(body, separators=(",", ":"))

    streams = {
        "real-sized": [line(seq, bytes(rng.randrange(32, 127) for _ in range(rng.choice([40, 400, 4000]))))
                       for seq in range(2000)],
        "small": [line(seq, b"\x1b[%d;%dHx" % (seq % 40 + 1, seq % 120 + 1)) for seq in range(50000)],
    }
    os.makedirs(WORK, exist_ok=True)
    for name, lines in streams.items():
        path = os.path.join(WORK, f"bench-{name}.jsonl")
        with open(path, "w") as f:
            f.write("\n".join(lines) + "\n")
        size = os.path.getsize(path)
        print(f"{name}: {size / 1e6:.1f} MB, {len(lines)} frames")
        for chunk in ("whole", "65536", "4096"):
            iterations = "1" if name == "small" and chunk == "whole" else "3"
            for binary in (SWIFT, RUST):
                done = sh([binary, "bench", path, chunk, iterations], capture_output=True, text=True)
                print(f"  {chunk:>6}  {done.stdout.strip()}")


if __name__ == "__main__":
    main()
