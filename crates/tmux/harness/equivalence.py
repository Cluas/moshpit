#!/usr/bin/env python3
"""G2 equivalence harness: the Swift -CC parser against its Rust port.

    equivalence.py [run] [--quick] [--no-build] [--no-record]
    equivalence.py update-goldens [--no-build]
    equivalence.py bench [--no-build] [--iterations N]

`run` builds both command-line front ends (harness/build-swift.sh and the
cc_events example), then feeds every input to both parsers under every plan
and compares the JSON-line output byte for byte:

  tests/fixtures/captures/*.bin    real tmux 3.6a streams (record.py)
  tests/fixtures/synthetic/*.bin   recovery and malformed-line paths
  target/g2/captures/burst.bin     seq 1 200000, recorded here if missing
  target/g2/fuzz/*.bin             seeded random streams (synth.py fuzz)

Plans cut the input whole, byte by byte, in random sizes (fixed seeds),
inside UTF-8 sequences and octal escapes, and with resets. No difference is
accepted: the Swift parser carries every fix the port made (see
harness/divergences.json). Each plan's events must also equal the
whole-feed events (chunking invariance). For each divergence fixture both
parsers must print its recorded fix (<fixture>.rust.jsonl) under every
divergence plan, and that output must differ from what the Swift parser
printed before the port (<fixture>.swift-fd87248f.jsonl, or a trap).

`update-goldens` rewrites the committed outputs: <capture>.swift.jsonl for
captures and synthetic inputs, and <fixture>.rust.jsonl for the
divergences (once both parsers agree on it); the pre-port Swift outputs are
history and never rewritten. `bench` times both parsers on the burst
capture.

Everything runs niced, one process at a time.
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures")
WORK = os.path.join(REPO, "target", "g2")
SWIFT = os.path.join(WORK, "cc-events-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "cc_events")
NICE = ["nice", "-n", "10"]

FULL_PLANS = (["whole", "bytes"]
              + [f"rand:{seed}:{size}" for seed in range(1, 9) for size in (3, 17, 256, 4096)]
              + [f"inside:{seed}" for seed in range(1, 5)]
              + ["reset:1:64", "reset:2:512"])
BIG_PLANS = ["whole", "bytes", "rand:1:3", "rand:2:17", "rand:3:256", "rand:4:4096",
             "rand:5:65536", "inside:1", "inside:2", "reset:1:4096"]
QUICK_PLANS = ["whole", "bytes", "rand:1:3", "rand:2:256", "inside:1", "reset:1:64"]
DIVERGENCE_PLANS = ["whole", "bytes", "rand:1:7", "inside:1"]
FUZZ_COUNT = 16


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-tmux", "--example", "cc_events"],
       check=True, cwd=REPO, env=env)


def events(binary, capture, plan):
    """(returncode, stdout bytes) of one run."""
    done = sh([binary, "events", capture, plan], capture_output=True)
    return done.returncode, done.stdout


def body(output: bytes):
    """Event lines without the trailing `end` record."""
    lines = output.split(b"\n")
    return lines[:-2] if len(lines) >= 2 else lines


def difference(swift: bytes, rust: bytes):
    """Where two outputs first differ, for the report."""
    s_lines, r_lines = swift.split(b"\n"), rust.split(b"\n")
    for number, (s, r) in enumerate(zip(s_lines, r_lines), 1):
        if s != r:
            return f"line {number}: swift {s[:160]!r} / rust {r[:160]!r}"
    return f"line counts differ: swift {len(s_lines)} rust {len(r_lines)}"


def inputs(record=True):
    found = []
    for group in ("captures", "synthetic"):
        folder = os.path.join(FIXTURES, group)
        for name in sorted(os.listdir(folder)):
            if name.endswith(".bin"):
                found.append((group, os.path.join(folder, name)))
    burst = os.path.join(WORK, "captures", "burst.bin")
    if not os.path.exists(burst) and record:
        os.makedirs(os.path.dirname(burst), exist_ok=True)
        sh([sys.executable, os.path.join(HERE, "record.py"), "burst", burst], check=True)
    if os.path.exists(burst):
        found.append(("recorded", burst))
    fuzz = os.path.join(WORK, "fuzz")
    sh([sys.executable, os.path.join(HERE, "synth.py"), "fuzz", fuzz, str(FUZZ_COUNT)], check=True)
    for name in sorted(os.listdir(fuzz), key=lambda n: (len(n), n)):
        found.append(("fuzz", os.path.join(fuzz, name)))
    return found


def check_input(group, capture, plans):
    """One row of the report."""
    name = os.path.basename(capture)[:-4]
    big = os.path.getsize(capture) > 1_000_000
    plans = BIG_PLANS if big and plans is FULL_PLANS else plans
    row = {"group": group, "input": name, "bytes": os.path.getsize(capture), "plans": 0,
           "identical": 0, "cuts": 0, "in_utf8": 0, "in_octal": 0, "failures": []}
    whole_events = None
    for plan in plans:
        s_rc, s_out = events(SWIFT, capture, plan)
        r_rc, r_out = events(RUST, capture, plan)
        row["plans"] += 1
        if s_rc != 0 or r_rc != 0:
            row["failures"].append(f"{plan}: exit swift {s_rc} rust {r_rc}")
            continue
        end = json.loads(r_out.rstrip(b"\n").rsplit(b"\n", 1)[-1])
        for key in ("cuts", "in_utf8", "in_octal"):
            row[key] += end[key]
        if plan == "whole":
            whole_events = body(r_out)
            row["events"] = len(whole_events)
            golden = capture[:-4] + ".swift.jsonl"
            if group in ("captures", "synthetic"):
                if not os.path.exists(golden) or open(golden, "rb").read() != s_out:
                    row["failures"].append("whole: Swift output differs from the committed golden")
        if s_out == r_out:
            row["identical"] += 1
        else:
            row["failures"].append(f"{plan}: {difference(s_out, r_out)}")
        if not plan.startswith("reset") and whole_events is not None and body(r_out) != whole_events:
            row["failures"].append(f"{plan}: events differ from the whole feed")
    return row


def check_divergences():
    """Both parsers print each fixture's recorded fix under every divergence
    plan, and the fixture still tells them from the pre-port Swift parser."""
    manifest = json.load(open(os.path.join(HERE, "divergences.json"), encoding="utf-8"))
    rows = []
    for fix in manifest["fixes"]:
        for fixture in fix["fixtures"]:
            capture = os.path.join(FIXTURES, "divergences", fixture + ".bin")
            row = {"fix": fix["id"], "fixture": fixture, "failures": []}
            expected = open(capture[:-4] + ".rust.jsonl", "rb").read()
            outputs = {}
            for plan in DIVERGENCE_PLANS:
                for name, binary in (("swift", SWIFT), ("rust", RUST)):
                    rc, out = events(binary, capture, plan)
                    outputs[name] = out
                    if rc != 0:
                        row["failures"].append(f"{name} {plan}: exit {rc}")
                    elif body(out) != body(expected):
                        row["failures"].append(f"{name} {plan}: {difference(out, expected)}")
                if outputs["swift"] != outputs["rust"]:
                    row["failures"].append(f"both {plan}: {difference(outputs['swift'], outputs['rust'])}")
            for name in ("swift", "rust"):
                row[name] = "WRONG" if any(f.startswith((name, "both")) for f in row["failures"]) else "fixed"
            before = capture[:-4] + ".swift-fd87248f.jsonl"
            if fix["swift_before"] == "trap":
                row["before"] = "trap"
                if os.path.exists(before):
                    row["failures"].append("the pre-port Swift parser trapped: no output to keep")
            else:
                row["before"] = "differs"
                if body(open(before, "rb").read()) == body(expected):
                    row["before"] = "SAME"
                    row["failures"].append("the pre-port Swift output equals the fix: not a regression capture")
            rows.append(row)
    return rows


def update_goldens():
    for group in ("captures", "synthetic"):
        folder = os.path.join(FIXTURES, group)
        for name in sorted(os.listdir(folder)):
            if name.endswith(".bin"):
                capture = os.path.join(folder, name)
                rc, out = events(SWIFT, capture, "whole")
                assert rc == 0, (capture, rc)
                open(capture[:-4] + ".swift.jsonl", "wb").write(out)
                print(f"{group}/{name[:-4]}.swift.jsonl")
    manifest = json.load(open(os.path.join(HERE, "divergences.json"), encoding="utf-8"))
    for fix in manifest["fixes"]:
        for fixture in fix["fixtures"]:
            capture = os.path.join(FIXTURES, "divergences", fixture + ".bin")
            s_rc, s_out = events(SWIFT, capture, "whole")
            rc, out = events(RUST, capture, "whole")
            assert (s_rc, rc) == (0, 0), (capture, s_rc, rc)
            assert s_out == out, (capture, difference(s_out, out))
            open(capture[:-4] + ".rust.jsonl", "wb").write(out)
            print(f"divergences/{fixture}")


def environment():
    def first_line(argv):
        try:
            return subprocess.run(argv, capture_output=True, text=True).stdout.strip().splitlines()[0]
        except (OSError, IndexError):
            return "?"
    return {
        "tmux": first_line(["tmux", "-V"]),
        "swiftc": first_line(["swiftc", "--version"]),
        "rustc": first_line(["rustc", "--version"]),
        "cpu": first_line(["sysctl", "-n", "machdep.cpu.brand_string"]),
        "load": os.getloadavg(),
    }


def run(quick, record):
    started = time.monotonic()
    plans = QUICK_PLANS if quick else FULL_PLANS
    rows = [check_input(group, capture, plans) for group, capture in inputs(record)]
    divergences = check_divergences()
    env = environment()
    print(f"tmux {env['tmux']} | {env['swiftc']} | {env['rustc']} | {env['cpu']} | "
          f"load {env['load'][0]:.0f}")
    print(f"\n{'group':10} {'input':16} {'bytes':>9} {'events':>7} {'plans':>5} "
          f"{'cuts':>9} {'in-utf8':>8} {'in-esc':>8} {'identical':>9}  result")
    failed = False
    for row in rows:
        ok = not row["failures"]
        failed |= not ok
        print(f"{row['group']:10} {row['input']:16} {row['bytes']:>9} {row.get('events', '-'):>7} "
              f"{row['plans']:>5} {row['cuts']:>9} {row['in_utf8']:>8} {row['in_octal']:>8} "
              f"{row['identical']:>9}  {'ok' if ok else 'FAIL'}")
        for failure in row["failures"][:5]:
            print(f"    {failure}")
    print(f"\n{'fix':28} {'fixture':28} {'swift':8} {'rust':8} {'pre-port swift':15} result")
    for row in divergences:
        ok = not row["failures"]
        failed |= not ok
        print(f"{row['fix']:28} {row['fixture']:28} {row['swift']:8} {row['rust']:8} "
              f"{row['before']:15} {'ok' if ok else 'FAIL'}")
        for failure in row["failures"][:5]:
            print(f"    {failure}")
    runs = sum(row["plans"] for row in rows)
    cuts = sum(row["cuts"] for row in rows)
    print(f"\n{len(rows)} inputs, {runs} plan runs per parser, {cuts} feed boundaries "
          f"({sum(row['in_utf8'] for row in rows)} inside UTF-8 sequences, "
          f"{sum(row['in_octal'] for row in rows)} inside escapes), {len(divergences)} divergence "
          f"fixtures, {time.monotonic() - started:.0f} s: {'FAIL' if failed else 'all equivalent'}")
    return 1 if failed else 0


def bench(iterations):
    burst = os.path.join(WORK, "captures", "burst.bin")
    if not os.path.exists(burst):
        os.makedirs(os.path.dirname(burst), exist_ok=True)
        sh([sys.executable, os.path.join(HERE, "record.py"), "burst", burst], check=True)
    env = environment()
    print(f"# {env['cpu']} | load {env['load'][0]:.0f} | {os.path.getsize(burst)} bytes")
    for chunk, count in (("4096", iterations), ("32768", iterations), ("whole", 1)):
        for binary in (SWIFT, RUST):
            done = sh([binary, "bench", burst, chunk, str(count)], capture_output=True, text=True,
                      check=True)
            print(done.stdout.strip())


def main(argv):
    args = argv[1:]
    command = args.pop(0) if args and not args[0].startswith("--") else "run"
    if "--no-build" not in args:
        build()
    if command == "run":
        sys.exit(run("--quick" in args, "--no-record" not in args))
    elif command == "update-goldens":
        update_goldens()
    elif command == "bench":
        iterations = int(args[args.index("--iterations") + 1]) if "--iterations" in args else 5
        bench(iterations)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
