#!/usr/bin/env python3
"""Equivalence harness: the app's Swift mosh client against its Rust port.

    equivalence.py [run] [--quick] [--no-build]
    equivalence.py update-goldens [--no-build]
    equivalence.py bench [--no-build]

`run` builds both front ends (harness/build-swift.sh over the app's
unchanged Moshpit/Services/Mosh sources, and examples/mosh_harness.rs) and
feeds both the same inputs:

  wire cases   seeded fuzz (src/harness/wire.rs), the committed cases,
               the recorded mosh-server sessions, zlib streams made by the
               real zlib (Python's zlib links the same 1.2.12 that
               mosh-server does), and DEFLATE blocks with odd Huffman code
               sets (`dynamic_block`), each one result line per case;
  scenarios    seeded SSP sessions (src/harness/ssp.rs) through the Swift
               MoshTransport actor and the Rust state machine (typing,
               resizes, redraws, idling that makes idle states, time passing
               between events so the send timer paces input, retransmits
               and acks by mosh's rules, input split and sent in chained
               instructions under small instruction budgets, a server with
               loss, reordering and garbage that now and then ends the
               session with its shutdown state), and replays of the
               recorded sessions;
  divergences  one minimal input per fix in harness/divergences.json
               (tests/fixtures/wire/divergences.txt).

Outputs must be byte-identical: the Swift client now carries every fix the
port made (see harness/divergences.json), so no difference is accepted,
except on the divergence fixtures of the one fix still open
(excerpt-unicode, display only), whose recorded outputs must reappear.
Every divergence fixture must still differ from what the Swift client
printed before the fixes (divergences.swift-b4b8e1b4.txt), which proves it
tells a fixed client from an unfixed one. `zlib.compress` outputs are not
compared (Apple's DEFLATE is not miniz's); instead each side's output must
inflate to the input on the other side and in the real zlib. Every
`zlib.decompress` result of both sides must be what the real zlib says
(error class included: mosh's length and header checks, then zlib), with
no exception: the Rust port inflates by zlib's rules (src/inflate.rs) since
miniz_oxide's acceptance of a single code longer than one bit was fixed. In
scenarios `thr` must be the acked state (`old`) on both sides (the
throwaway fix).

`update-goldens` rewrites the committed Swift outputs that tests/
equivalence.rs checks without swiftc (never the pre-fix history). `bench`
times both send and receive paths. Everything runs niced, one process at a
time.
"""
import os
import re
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures")
WORK = os.path.join(REPO, "target", "mosh-harness")
SWIFT = os.path.join(WORK, "mosh-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "mosh_harness")
NICE = ["nice", "-n", "10"]

GOLDEN_WIRE = (1, 800, 200)          # seed, count, max length of the committed cases
GOLDEN_SSP = [(1, 120), (2, 120), (3, 120)]
SESSIONS = ["swift", "rust"]
# Fixes the Swift client does not carry (harness/divergences.json): their
# divergence fixtures keep differing, and no other input may trigger them.
OPEN_FIXES = {"excerpt-unicode"}
# The Swift mosh sources every fix in harness/divergences.json is measured
# against; their outputs on the divergence fixtures are history.
PRE_FIX = "b4b8e1b4"


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-mosh", "--example", "mosh_harness"],
       check=True, cwd=REPO, env=env)


def rust(*args):
    return sh([RUST, *map(str, args)], check=True, capture_output=True, text=True).stdout


def run_both(mode, path):
    """(swift lines, rust lines) of a wire or ssp run over `path`."""
    out = {}
    for name, binary in (("swift", SWIFT), ("rust", RUST)):
        target = os.path.join(WORK, os.path.basename(path) + "." + name)
        done = sh([binary, mode, path, target], capture_output=True, text=True)
        if done.returncode != 0:
            sys.exit(f"{name} {mode} {path} failed ({done.returncode}): {done.stderr[-2000:]}")
        with open(target) as f:
            out[name] = f.read().split("\n")[:-1]
    return out["swift"], out["rust"]


def case_lines(path):
    with open(path) as f:
        return [line for line in f.read().split("\n") if line and not line.startswith("#")]


def field(line, name):
    match = re.search(r"(?:^| )" + name + r"=(\S*)", line)
    return match.group(1) if match else None


def strip_compress(line):
    return re.sub(r" out=\S*", "", line)


# MARK: the real zlib as the oracle for zlib.decompress

MAX_DECOMPRESSED = 2048 * 2048        # mosh's Compressor::BUFFER_SIZE


def header_ok(cmf, flg):
    return ((cmf << 8) | flg) % 31 == 0 and cmf & 0x0F == 8 and cmf >> 4 <= 7 and not flg & 0x20


def zlib_oracle(data):
    """(result line, zlib's message) for a zlib.decompress case: mosh's
    length and header checks, then the real zlib and mosh's 4 MiB buffer."""
    if len(data) < 6 or not header_ok(data[0], data[1]):
        return "zlib.decompress err badHeader", None
    try:
        out = zlib.decompress(data)
    except zlib.error as error:
        message = str(error).split(": ", 1)[-1]
        name = "badChecksum" if message == "incorrect data check" else "inflateFailed"
        return f"zlib.decompress err {name}", message
    if len(out) > MAX_DECOMPRESSED:
        return "zlib.decompress err tooLarge", None
    return f"zlib.decompress ok data={out.hex()}", None


def compare_wire(cases, swift, rust, label):
    """Exits on any difference."""
    if not (len(cases) == len(swift) == len(rust)):
        sys.exit(f"{label}: {len(cases)} cases, {len(swift)} Swift lines, {len(rust)} Rust lines")
    differences = []
    for index, (case, a, b) in enumerate(zip(cases, swift, rust)):
        if case.startswith("zlib.compress"):
            a, b = strip_compress(a), strip_compress(b)
        if a != b:
            differences.append((index, case[:160], a[:160], b[:160]))
    for index, case, a, b in differences[:10]:
        print(f"  #{index}: {case}\n    swift {a}\n    rust  {b}")
    if differences:
        sys.exit(f"{label}: {len(differences)} differences")


def check_compression(cases, swift, rust, label):
    """Each side's zlib output inflates to the input in the other side and in
    the real zlib; header and trailer are compared with the lines."""
    cross = []
    for case, a, b in zip(cases, swift, rust):
        if not case.startswith("zlib.compress"):
            continue
        data = bytes.fromhex(field(case, "data"))
        swift_out, rust_out = field(a, "out"), field(b, "out")
        if zlib.decompress(bytes.fromhex(rust_out)) != data:
            sys.exit(f"{label}: the real zlib does not inflate Rust's output for {case[:80]}")
        cross.append((data, rust_out, "swift"))
        cross.append((data, swift_out, "rust"))
    if not cross:
        return 0
    path = os.path.join(WORK, label + ".cross.txt")
    with open(path, "w") as f:
        for _, out, _ in cross:
            f.write(f"zlib.decompress data={out}\n")
    swift_lines, rust_lines = run_both("wire", path)
    for (data, _, side), a, b in zip(cross, swift_lines, rust_lines):
        line = a if side == "swift" else b
        if line != f"zlib.decompress ok data={data.hex()}":
            sys.exit(f"{label}: {side} cannot inflate the other side's output: {line[:120]}")
    return len(cross) // 2


def real_zlib_cases(seed, count):
    """zlib.decompress cases made by the real zlib at every level, some
    corrupted the way the generator corrupts."""
    import random
    rng = random.Random(seed)
    cases = []
    for _ in range(count):
        n = rng.choice([0, 1, 5, 100, 1000, 3000])
        kind = rng.randrange(3)
        if kind == 0:
            data = bytes(rng.randrange(256) for _ in range(n))
        elif kind == 1:
            data = bytes([rng.randrange(256)]) * n
        else:
            data = (b"\x1b[1;1Hline of terminal output " * (n // 30 + 1))[:n]
        stream = bytearray(zlib.compress(data, rng.randrange(10)))
        if rng.randrange(3) == 0 and stream:
            at = rng.randrange(len(stream))
            stream[at] ^= 1 << rng.randrange(8)
        cases.append(f"zlib.decompress data={bytes(stream).hex()}")
    return cases


# MARK: DEFLATE blocks with odd code sets

DEFLATE_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]


class BitWriter:
    """DEFLATE's bit order: fields LSB first, Huffman codes MSB first."""

    def __init__(self):
        self.out, self.acc, self.count = bytearray(), 0, 0

    def put(self, value, count):
        self.acc |= value << self.count
        self.count += count
        while self.count >= 8:
            self.out.append(self.acc & 0xFF)
            self.acc >>= 8
            self.count -= 8

    def code(self, code, length):
        self.put(int(format(code, f"0{length}b")[::-1], 2), length)

    def bytes(self):
        return bytes(self.out) + (bytes([self.acc]) if self.count else b"")


def canonical_codes(lengths):
    """symbol -> (code, length), assigned canonically even for a set zlib
    refuses (over-subscribed sets get colliding codes, which is fine here)."""
    codes, code = {}, 0
    for length in range(1, 16):
        for symbol, have in enumerate(lengths):
            if have == length:
                codes[symbol] = (code, length)
                code += 1
        code <<= 1
    return codes


def dynamic_block(literal_lengths, distance_lengths, data):
    """A final dynamic block coding `data` with literals only, under the
    given literal/length and distance code lengths (any lengths: that is the
    point), its own header coded with a complete code-length code."""
    symbols, lengths, i = [], literal_lengths + distance_lengths, 0
    while i < len(lengths):
        run = 0
        while i + run < len(lengths) and lengths[i + run] == 0 and run < 138:
            run += 1
        if run >= 11:
            symbols.append((18, run - 11, 7))
        elif run >= 3:
            symbols.append((17, run - 3, 3))
        else:
            run = 1
            symbols.append((lengths[i], 0, 0))
        i += run
    used = sorted({symbol for symbol, _, _ in symbols})
    code_lengths = [0] * 19
    if len(used) == 1:
        code_lengths[used[0]] = 1
        code_lengths[next(s for s in range(19) if s != used[0])] = 1
    else:
        for rank, symbol in enumerate(used):
            code_lengths[symbol] = min(rank + 1, len(used) - 1)
    count = max(4, max(i for i, s in enumerate(DEFLATE_ORDER) if code_lengths[s]) + 1)
    out = BitWriter()
    out.put(1, 1)
    out.put(2, 2)
    out.put(len(literal_lengths) - 257, 5)
    out.put(len(distance_lengths) - 1, 5)
    out.put(count - 4, 4)
    for symbol in DEFLATE_ORDER[:count]:
        out.put(code_lengths[symbol], 3)
    header = canonical_codes(code_lengths)
    for symbol, extra, bits in symbols:
        out.code(*header[symbol])
        out.put(extra, bits)
    literals = canonical_codes(literal_lengths)
    for symbol in [*data, 256]:
        if symbol in literals:      # without an end-of-block code, none goes
            out.code(*literals[symbol])
    return b"\x78\x9c" + out.bytes() + zlib.adler32(data).to_bytes(4, "big")


def code_set_cases(seed, count):
    """zlib.decompress cases whose blocks carry code sets at zlib's edges:
    a single literal/length or distance code of every length (zlib takes
    only one bit), then random small sets, some streams corrupted. Every
    trailer is right, so a decoder lenient about a set accepts the stream."""
    import random
    rng = random.Random(seed)
    streams = []
    for length in range(1, 16):
        literals = [0] * 257
        literals[ord("a")] = 1
        literals[256] = 1
        streams.append(dynamic_block(literals, [length], b"aaa"))
        literals = [0] * 257
        literals[256] = length
        streams.append(dynamic_block(literals, [0], b""))
    while len(streams) < count:
        literals = [0] * rng.choice([257, 260, 286])
        alphabet = rng.sample(range(256), rng.randrange(0, 4))
        for symbol in alphabet:
            literals[symbol] = rng.randrange(1, 16)
        literals[256] = rng.randrange(0, 16)
        distances = [0] * rng.randrange(1, 31)
        for i in rng.sample(range(len(distances)), rng.randrange(0, min(3, len(distances)) + 1)):
            distances[i] = rng.randrange(0, 16)
        data = bytes(rng.choice(alphabet) for _ in range(rng.randrange(0, 5))) if alphabet else b""
        stream = bytearray(dynamic_block(literals, distances, data))
        if rng.randrange(3) == 0:
            stream[rng.randrange(2, len(stream))] ^= 1 << rng.randrange(8)
        streams.append(bytes(stream))
    return [f"zlib.decompress data={stream.hex()}" for stream in streams]


def check_against_real_zlib(cases, lines, side, label):
    """`side` gives the oracle's result on every zlib.decompress case."""
    for case, line in zip(cases, lines):
        if not case.startswith("zlib.decompress "):
            continue
        expected, _ = zlib_oracle(bytes.fromhex(field(case, "data")))
        if line != expected:
            sys.exit(f"{label}: {side} differs from zlib: {case[:120]}\n  {side} {line[:120]}\n  zlib  {expected[:120]}")


def wire_round(label, cases):
    path = os.path.join(WORK, label + ".txt")
    with open(path, "w") as f:
        f.write("\n".join(cases) + "\n")
    swift, rust_lines = run_both("wire", path)
    compare_wire(cases, swift, rust_lines, label)
    compressed = check_compression(cases, swift, rust_lines, label)
    check_against_real_zlib(cases, swift, "Swift", label)
    check_against_real_zlib(cases, rust_lines, "Rust", label)
    return compressed


def divergence_cases():
    """[(fix id, case)] of tests/fixtures/wire/divergences.txt: each case
    follows a '# <fix id>: ...' comment."""
    out, fix = [], None
    with open(os.path.join(FIXTURES, "wire", "divergences.txt")) as f:
        for line in f.read().split("\n"):
            if line.startswith("# ") and ":" in line:
                fix = line[2:].split(":", 1)[0]
            elif line and not line.startswith("#"):
                out.append((fix, line))
    return out


def recorded(name):
    with open(os.path.join(FIXTURES, "wire", name)) as f:
        return f.read().split("\n")[:-1]


def check_divergences():
    """Both sides print the recorded results on the divergence fixtures:
    identical except for open fixes, and each unlike the pre-fix Swift."""
    pairs = divergence_cases()
    swift, rust_lines = run_both("wire", os.path.join(FIXTURES, "wire", "divergences.txt"))
    if rust_lines != recorded("divergences.rust.txt"):
        sys.exit("divergences: Rust no longer prints divergences.rust.txt")
    if swift != recorded("divergences.swift.txt"):
        sys.exit("divergences: Swift no longer prints divergences.swift.txt")
    before = recorded(f"divergences.swift-{PRE_FIX}.txt")
    if not (len(pairs) == len(swift) == len(rust_lines) == len(before)):
        sys.exit("divergences: case and result counts differ")
    fixed = {}
    for (fix, case), a, b, old in zip(pairs, swift, rust_lines, before):
        if fix not in OPEN_FIXES and a != b:
            sys.exit(f"divergences: {fix} still differs: {case[:100]}\n  swift {a[:120]}\n  rust  {b[:120]}")
        if fix in OPEN_FIXES and a == b:
            sys.exit(f"divergences: open fix {fix} no longer differs; close it in divergences.json")
        if old == b:
            sys.exit(f"divergences: {fix} fixture does not tell the pre-fix Swift from the fix: {case[:100]}")
        if fix not in OPEN_FIXES:
            fixed[fix] = fixed.get(fix, 0) + 1
    return fixed


# MARK: scenarios

def compare_ssp(path, label):
    swift, rust_lines = run_both("ssp", path)
    if swift != rust_lines:
        for index, (a, b) in enumerate(zip(swift, rust_lines)):
            if a != b:
                sys.exit(f"{label}: line {index}\n  swift {a[:160]}\n  rust  {b[:160]}")
        sys.exit(f"{label}: {len(swift)} Swift lines, {len(rust_lines)} Rust lines")
    for line in rust_lines:
        if line.startswith("inst ") and field(line, "thr") != field(line, "old"):
            sys.exit(f"{label}: throwaway rule broken: {line}")
    frames = sum(1 for line in rust_lines if line.startswith("frame "))
    applied = [line for line in rust_lines if line.startswith("diag ")][-1]
    steps = [line.split()[2] for line in rust_lines if line.startswith("step ")]
    redraws = sum(1 for step in steps if step in ("redraw", "resume-force"))
    # Datagrams the send timer sent (a `tick` step that transmitted).
    timed, current = 0, None
    for line in rust_lines:
        if line.startswith("step "):
            current = line.split()[2]
        elif line.startswith("tx ") and current == "tick":
            timed += 1
            current = None
    # Steps whose flush went out as a chain of instructions (split input).
    per_step, chained = 0, 0
    for line in rust_lines + ["step"]:
        if line.startswith("step"):
            chained += per_step > 1
            per_step = 0
        elif line.startswith("inst "):
            per_step += 1
    ended = sum(1 for line in rust_lines if line == "ended")
    return frames, applied, redraws, steps.count("idle"), chained, timed, ended


# MARK: commands

def run(quick):
    os.makedirs(WORK, exist_ok=True)
    seeds = range(1, 3) if quick else range(1, 17)
    count, max_len = (2000, 1500) if quick else (3000, 3000)
    compressed, total_cases = 0, 0
    for seed in seeds:
        cases = rust("gen-wire", seed, count, max_len).split("\n")[:-1]
        compressed += wire_round(f"wire-{seed}", cases)
        total_cases += len(cases)
    for label, cases in [
        ("wire-golden", case_lines(os.path.join(FIXTURES, "wire", "cases.txt"))),
        ("wire-real-zlib", real_zlib_cases(1, 200 if quick else 2000)),
        ("wire-code-sets", code_set_cases(1, 300 if quick else 3000)),
    ] + [(f"wire-session-{name}", rust("session-cases", os.path.join(FIXTURES, "live", f"session-{name}.txt"))
          .split("\n")[:-1]) for name in SESSIONS]:
        compressed += wire_round(label, cases)
        total_cases += len(cases)
    print(f"wire: {total_cases} cases identical; {compressed} compressions inflate across both "
          f"sides and in the real zlib; every zlib.decompress result is zlib's")
    fixed = check_divergences()
    print(f"divergence fixtures: identical on {sum(fixed.values())} cases of the fixed {sorted(fixed)}, "
          f"recorded difference on {sorted(OPEN_FIXES)}, all unlike the Swift client at {PRE_FIX}")

    scenario_seeds = range(1, 11) if quick else range(1, 101)
    frames = redraws = idles = chains = timed = ended = 0
    for seed in scenario_seeds:
        path = os.path.join(WORK, f"ssp-{seed}.txt")
        with open(path, "w") as f:
            f.write(rust("gen-ssp", seed, 200 if quick else 300))
        n, _, r, i, c, t, e = compare_ssp(path, f"ssp-{seed}")
        frames, redraws, idles, chains, timed, ended = (frames + n, redraws + r, idles + i,
                                                        chains + c, timed + t, ended + e)
    for name in SESSIONS:
        path = os.path.join(WORK, f"ssp-session-{name}.txt")
        with open(path, "w") as f:
            f.write(rust("session-ssp", os.path.join(FIXTURES, "live", f"session-{name}.txt")))
        n, applied, _, _, _, _, _ = compare_ssp(path, f"ssp-session-{name}")
        frames += n
        print(f"ssp replay of the {name}-client session: identical, {n} frames, {applied}")
    for seed, steps in GOLDEN_SSP:
        n, _, r, i, c, t, e = compare_ssp(os.path.join(FIXTURES, "ssp", f"s{seed}.txt"),
                                          f"ssp-golden-{seed}")
        frames, redraws, idles, chains, timed, ended = (frames + n, redraws + r, idles + i,
                                                        chains + c, timed + t, ended + e)
    print(f"ssp: {len(scenario_seeds) + len(SESSIONS) + len(GOLDEN_SSP)} scenarios identical "
          f"({frames} host frames, {redraws} redraws, {idles} idle steps, {timed} timer steps "
          f"that sent, {chains} steps sending chained instructions, {ended} sessions the "
          f"server ended), throwaway rule holds on both sides")


def update_goldens():
    os.makedirs(WORK, exist_ok=True)
    wire = os.path.join(FIXTURES, "wire")
    os.makedirs(wire, exist_ok=True)
    seed, count, max_len = GOLDEN_WIRE
    cases = rust("gen-wire", seed, count, max_len).split("\n")[:-1]
    path = os.path.join(wire, "cases.txt")
    with open(path, "w") as f:
        f.write(f"# gen-wire {seed} {count} {max_len} (src/harness/wire.rs)\n" + "\n".join(cases) + "\n")
    swift, rust_lines = run_both("wire", path)
    compare_wire(cases, swift, rust_lines, "golden")
    with open(os.path.join(wire, "cases.swift.txt"), "w") as f:
        f.write("\n".join(swift) + "\n")
    swift, rust_lines = run_both("wire", os.path.join(wire, "divergences.txt"))
    for name, lines in (("swift", swift), ("rust", rust_lines)):
        with open(os.path.join(wire, f"divergences.{name}.txt"), "w") as f:
            f.write("\n".join(lines) + "\n")
    check_divergences()
    for name in SESSIONS:
        session = os.path.join(FIXTURES, "live", f"session-{name}.txt")
        path = os.path.join(WORK, f"golden-session-{name}.txt")
        with open(path, "w") as f:
            f.write(rust("session-cases", session))
        swift, rust_lines = run_both("wire", path)
        assert swift == rust_lines
        with open(os.path.join(FIXTURES, "live", f"session-{name}.wire.swift.txt"), "w") as f:
            f.write("\n".join(swift) + "\n")
        path = os.path.join(WORK, f"golden-session-{name}.ssp")
        with open(path, "w") as f:
            f.write(rust("session-ssp", session))
        swift, _ = run_both("ssp", path)
        with open(os.path.join(FIXTURES, "live", f"session-{name}.ssp.swift.txt"), "w") as f:
            f.write("\n".join(swift) + "\n")
    ssp_dir = os.path.join(FIXTURES, "ssp")
    os.makedirs(ssp_dir, exist_ok=True)
    for seed, steps in GOLDEN_SSP:
        path = os.path.join(ssp_dir, f"s{seed}.txt")
        with open(path, "w") as f:
            f.write(rust("gen-ssp", seed, steps))
        compare_ssp(path, f"golden-ssp-{seed}")
        os.replace(os.path.join(WORK, f"s{seed}.txt.swift"), os.path.join(ssp_dir, f"s{seed}.swift.txt"))
    print("goldens updated")


BENCHES = [("send", 16, 200_000), ("send", 1_024, 50_000), ("send", 65_536, 1_000),
           ("recv", 1_024, 50_000), ("recv", 16_384, 5_000), ("seal", 1_300, 100_000)]


def bench():
    print(f"{'path':6} {'bytes':>6} {'Swift ns/op':>12} {'Rust ns/op':>11} {'Swift MB/s':>11} {'Rust MB/s':>10} {'speed-up':>9}")
    for what, size, iterations in BENCHES:
        results = {}
        for name, binary in (("swift", SWIFT), ("rust", RUST)):
            best = None
            for _ in range(3):
                line = sh([binary, "bench", what, str(size), str(iterations)], check=True,
                          capture_output=True, text=True).stdout
                ns = float(field(line, "ns_per_op"))
                best = ns if best is None else min(best, ns)
            results[name] = best
        s, r = results["swift"], results["rust"]
        print(f"{what:6} {size:>6} {s:>12.0f} {r:>11.0f} {size / s * 1000:>11.1f} {size / r * 1000:>10.1f} {s / r:>8.1f}x")


def main():
    args = sys.argv[1:]
    command = args[0] if args and not args[0].startswith("--") else "run"
    if "--no-build" not in args:
        build()
    if command == "run":
        run("--quick" in args)
    elif command == "update-goldens":
        update_goldens()
    elif command == "bench":
        bench()
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
