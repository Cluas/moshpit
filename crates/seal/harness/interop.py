#!/usr/bin/env python3
"""Three-way interop of the push envelope, and the send token.

    interop.py [run] [--no-build]      check everything live
    interop.py record [--no-build]     also rewrite tests/fixtures/interop/

Four implementations of format v1 meet here, all of them unchanged:

  host     scripts/host/moshpit-push.sh, run for real: under every POSIX shell
           on this Mac (sh, dash, bash 3 and 5, ksh) and every openssl build
           (LibreSSL, OpenSSL 1.1, OpenSSL 3), with a stub curl that keeps the
           request it was given — titles with quotes, backslashes, newlines,
           control characters, CJK, emoji and 200 characters, sessions and
           hosts with quotes, several confs in push.d, the --test mode;
  go       push-relay/sealbox (imported through a replace directive) and
           push-relay/token.go (copied beside harness/go/interop.go);
  swift    Packages/MoshpitKit PushSealedBox, through the equivalence
           harness's Swift binary;
  rust     this crate, through examples/seal_harness.rs.

Every envelope the host script, Go and Swift seal must open in Rust to the
status Swift and Go read, and in openssl (`enc -d`, the MAC recomputed with
`dgst -hmac`) to the same bytes; every envelope Rust seals must open in
Swift, Go and every openssl build. Pinned-IV seals must agree byte for byte
across Go, Swift and Rust. Every request the host script sent must carry a
send token the relay's Go verifySendToken accepts over the routing facts in
the same request, minted by Rust. Pairing files written by Swift's
PushPairingFile must read in Rust, and Rust's in Swift.

`record` writes what tests/interop.rs checks without any of the tools: the
envelopes each side sealed with the status Swift opened them to, the
pinned-IV seals, Go's send tokens, Swift-written pairing files.
"""
import binascii
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures", "interop")
WORK = os.path.join(REPO, "target", "seal-harness")
SWIFT = os.path.join(WORK, "seal-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "seal_harness")
GO_DIR = os.path.join(WORK, "go")
GO = os.path.join(GO_DIR, "interop")
SCRIPT = os.path.join(REPO, "scripts", "host", "moshpit-push.sh")
NICE = ["nice", "-n", "10"]

SHELLS = [s for s in ["/bin/sh", "/bin/dash", "/bin/bash", "/opt/homebrew/bin/bash", "/bin/ksh"] if os.path.exists(s)]
OPENSSLS = [o for o in ["/usr/bin/openssl", "/opt/homebrew/opt/openssl@1.1/bin/openssl",
                        "/opt/homebrew/opt/openssl@3/bin/openssl"] if os.path.exists(o)]
MASTER = b"moshpit-interop-master-0123456789abcdef"

TITLES = [
    'Bash: rm -rf "build" 构建',
    "plain",
    "",
    "back\\slash and \"quotes\" and 'single'",
    "line one\nline two\ttabbed",
    "ctrl \x01\x02\x1b[31mred\x1b[0m",
    "emoji 😀👨‍👩‍👧 🇨🇳",
    "x" * 200,
    "长" * 60,
    "$(echo pwned) `id` $HOME %s %d",
    "-n -e \\n",
    "trailing   ",
    "  separator   nbsp",
]
PLAINTEXTS = [
    "",
    "x",
    "a" * 16,
    "字" * 200,
    '{"conn":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","host":"m1-pro","sess":"work","pane":"%3",'
    '"agent":"claude","state":"attention","title":"Bash: rm -rf \\"build\\" 构建","ts":1755900000}',
    '{"conn":"C","host":"h","pane":"%1","state":"done","ts":1,"dur":240}',
    "not json",
]


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def hexs(text):
    return "~" if text == "" else text.encode().hex()


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "--release", "-p", "moshpit-seal", "--example", "seal_harness"],
       check=True, cwd=REPO, env=env)
    shutil.rmtree(GO_DIR, ignore_errors=True)
    os.makedirs(GO_DIR)
    shutil.copy(os.path.join(HERE, "go", "interop.go"), GO_DIR)
    # The relay's own token code, unchanged, beside the harness.
    shutil.copy(os.path.join(REPO, "push-relay", "token.go"), GO_DIR)
    with open(os.path.join(GO_DIR, "go.mod"), "w") as f:
        f.write("module moshpit-seal-interop\n\ngo 1.24\n\n"
                "require github.com/cluas/moshpit/push-relay v0.0.0\n\n"
                f"replace github.com/cluas/moshpit/push-relay => {os.path.join(REPO, 'push-relay')}\n")
    env = dict(os.environ, GOFLAGS="-mod=mod", GOTOOLCHAIN="local", GOPROXY="off")
    sh(["go", "build", "-o", GO, "."], check=True, cwd=GO_DIR, env=env)


def run_lines(argv, lines, cwd=None):
    out = sh(argv, input="\n".join(lines) + "\n", capture_output=True, text=True, check=True, cwd=cwd).stdout
    return [line for line in out.split("\n") if line]


def harness(binary, cases):
    with tempfile.TemporaryDirectory(prefix="seal-interop-") as tmp:
        path, result, scratch = (os.path.join(tmp, n) for n in ("cases", "out", "scratch"))
        os.mkdir(scratch)
        with open(path, "w") as f:
            f.write("\n".join(cases) + "\n")
        sh([binary, "run", path, result, scratch], check=True)
        with open(result) as f:
            return [line for line in f.read().split("\n") if line]


def go(lines):
    return run_lines([GO], lines)


def go_token(apns, conn, iat):
    """A send token as the relay mints it (tests/interop.rs holds Rust's
    mint to Go's)."""
    return go([f"mint {MASTER.hex()} {apns} {conn} {iat}"])[0]


# MARK: - openssl

def openssl_open(openssl, secret, env):
    """The phone's half done with openssl, as a shell would: recompute the
    MAC over the wire text, then `enc -d`. Returns the plaintext or None."""
    secret = secret.lower()

    def dgst(key, message, binary=False):
        argv = [openssl, "dgst", "-sha256", "-hmac", key] + (["-binary"] if binary else [])
        return subprocess.run(argv, input=message, capture_output=True, check=True).stdout

    ke = dgst(secret, b"moshpit-push-enc-v1").decode().split()[-1]
    km = dgst(secret, b"moshpit-push-mac-v1").decode().split()[-1]
    tag = dgst(km, f"v1|{env['iv'].lower()}|{env['ct']}".encode(), binary=True)
    if binascii.b2a_base64(tag, newline=False).decode() != env["mac"]:
        return None
    result = subprocess.run([openssl, "enc", "-d", "-aes-256-cbc", "-K", ke, "-iv", env["iv"], "-a", "-A"],
                            input=env["ct"].encode(), capture_output=True)
    return result.stdout if result.returncode == 0 else None


# MARK: - the host script

STUB_CURL = """#!/bin/sh
body=""; config=""; w=""
while [ $# -gt 0 ]; do
  case "$1" in
    --data-binary) body="$2"; shift 2 ;;
    --config) config="$2"; shift 2 ;;
    -w) w="$2"; shift 2 ;;
    *) shift ;;
  esac
done
f=$(mktemp "$MOSHPIT_CAPTURE/req.XXXXXX")
printf '%s' "$body" > "$f"
cat "$config" > "$f.config"
[ -n "$w" ] && printf '200'
exit 0
"""

STUB_TMUX = """#!/bin/sh
printf '%s\\n' "$MOSHPIT_STUB_SESSION"
"""


def conf_text(secret, conn, apns, iat, relay="https://push.example.org", env="production"):
    token = go_token(apns, conn, iat)
    return (f"RELAY_URL={relay}\nSEND_TOKEN={token}\nSECRET={secret}\nCONN={conn}\n"
            f"APNS_TOKEN={apns}\nAPNS_ENV={env}\nSEND_IAT={iat}\n"), token


def run_script(shell, openssl, args, confs, env_extra=None, multi=False):
    """Run the host script once; return the captured requests."""
    with tempfile.TemporaryDirectory(prefix="seal-host-") as tmp:
        bin_dir = os.path.join(tmp, "bin")
        capture = os.path.join(tmp, "capture")
        home = os.path.join(tmp, "home")
        os.makedirs(bin_dir)
        os.makedirs(capture)
        os.makedirs(os.path.join(home, ".moshpit", "push.d"))
        for name, text in (("curl", STUB_CURL), ("tmux", STUB_TMUX)):
            path = os.path.join(bin_dir, name)
            with open(path, "w") as f:
                f.write(text)
            os.chmod(path, 0o755)
        os.symlink(openssl, os.path.join(bin_dir, "openssl"))
        env = {
            "PATH": bin_dir + ":/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": home,
            "MOSHPIT_CAPTURE": capture,
            "MOSHPIT_PUSH_HOST": "m1-pro",
            "LANG": "en_US.UTF-8",
        }
        if multi:
            for i, text in enumerate(confs):
                with open(os.path.join(home, ".moshpit", "push.d", f"{i}.conf"), "w") as f:
                    f.write(text)
        else:
            path = os.path.join(tmp, "push.conf")
            with open(path, "w") as f:
                f.write(confs[0])
            env["MOSHPIT_PUSH_CONF"] = path
        env.update(env_extra or {})
        started = int(time.time())
        result = subprocess.run(NICE + [shell, SCRIPT] + args, env=env, capture_output=True)
        ended = int(time.time())
        assert result.returncode == 0, (shell, openssl, args, result.stderr)
        requests = []
        for name in sorted(os.listdir(capture)):
            if name.endswith(".config"):
                continue
            with open(os.path.join(capture, name), "rb") as f:
                body = f.read()
            with open(os.path.join(capture, name + ".config")) as f:
                config = f.read()
            requests.append((json.loads(body), config, started, ended))
        return requests, result.stdout


def script_vectors(problems):
    """Every shell x openssl x case; returns (secret, request, expectation)."""
    out = []
    conn = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"
    apns = "ab" * 32
    iat = int(time.time()) - 3600
    for shell in SHELLS:
        for openssl in OPENSSLS:
            secret = random.Random(f"{shell}{openssl}").getrandbits(256).to_bytes(32, "big").hex()
            conf, token = conf_text(secret.upper() if "dash" in shell else secret, conn, apns, iat)
            runs = [(["attention", "claude", t], {"TMUX_PANE": "%7"}, {"state": "attention", "title": t})
                    for t in TITLES]
            runs += [
                (["done", "codex", "the build", "185"], {}, {"state": "done", "dur": 185, "pane": "%0"}),
                (["done", "codex", "", "abc"], {}, {"state": "done", "dur": None}),
                (["done", "codex", "t", "0"], {}, {"state": "done", "dur": None}),
                (["--test", "selftest-abc123", conn], {}, {"state": "done", "agent": "moshpit-selftest",
                                                          "title": "selftest-abc123"}),
                (["attention", 'ag "q" \\', "t"], {"TMUX": "/tmp/sock,1,0", "TMUX_PANE": "%1",
                                                    "MOSHPIT_STUB_SESSION": 'wo"rk\\é',
                                                    "MOSHPIT_PUSH_HOST": 'h"o\\st\x01'},
                 {"state": "attention", "sess": 'wo"rk\\é', "host": 'h"o\\st'}),
                (["attention", "claude", "长" * 60], {"LANG": "C", "TMUX_PANE": "%2"}, {"state": "attention"}),
                (["working", "claude", "nothing"], {}, None),
            ]
            for args, extra, want in runs:
                requests, stdout = run_script(shell, openssl, args, [conf], extra)
                if want is None:
                    if requests:
                        problems.append(f"{shell} {openssl}: 'working' pushed")
                    continue
                if len(requests) != 1:
                    problems.append(f"{shell} {openssl} {args!r}: {len(requests)} requests")
                    continue
                body, config, started, ended = requests[0]
                if config != f'header = "authorization: Bearer {token}"\n':
                    problems.append(f"{shell} {openssl}: bearer {config!r}")
                out.append((f"host {os.path.basename(shell)} {os.path.basename(os.path.dirname(os.path.dirname(openssl)))}",
                            secret, body, dict(want, args=args, pane=want.get("pane", extra.get("TMUX_PANE", "%0")),
                                               started=started, ended=ended, conn=conn, token=token)))
        # Several paired devices: one request each, each sealed with its own secret.
        secrets = [random.Random(f"{shell}{i}").getrandbits(256).to_bytes(32, "big").hex() for i in range(2)]
        conns = ["11111111-2222-3333-4444-555555555555", "66666666-7777-8888-9999-000000000000"]
        confs = [conf_text(s, c, apns, iat)[0] for s, c in zip(secrets, conns)]
        requests, _ = run_script(shell, OPENSSLS[0], ["done", "claude", "both", "61"], confs, multi=True)
        seen = sorted(body["conn"] for body, _, _, _ in requests)
        if seen != sorted(conns):
            problems.append(f"{shell}: push.d sent to {seen}")
        for body, config, started, ended in requests:
            secret = secrets[conns.index(body["conn"])]
            out.append((f"host {os.path.basename(shell)} push.d", secret, body,
                        {"state": "done", "dur": 61, "title": "both", "pane": "%0", "args": [],
                         "started": started, "ended": ended, "conn": body["conn"], "token": None}))
    return out


def check_request(source, secret, body, want, status, problems):
    """What the host script sent, against what it was asked to send."""
    env = body["env"]
    if set(body) != {"env", "cat", "thread", "tok", "tokEnv", "conn", "iat"}:
        problems.append(f"{source}: body keys {sorted(body)}")
    if body["cat"] != want["state"]:
        problems.append(f"{source}: cat {body['cat']}")
    pane = want["pane"]
    thread = f"moshpit.attention.{want['conn']}" if want["state"] == "attention" else \
        f"moshpit.done.{want['conn']}.{pane}"
    if body["thread"] != thread:
        problems.append(f"{source}: thread {body['thread']} != {thread}")
    if want.get("token"):
        ok = go([f"verify {MASTER.hex()} {want['token']} {body['tok']} {body['conn']} {body['iat']} "
                 f"{int(time.time())} {45 * 86400}"])[0]
        if ok != "true":
            problems.append(f"{source}: the relay would refuse the send token")
    if status is None:
        problems.append(f"{source}: Swift could not open it")
        return
    fields = status.split(" ")
    text = lambda token: "" if token == "~" else (None if token == "-" else bytes.fromhex(token).decode())
    conn, host, sess, pane_got, agent, state, title, ts, dur = fields
    checks = {
        "conn": (text(conn), want["conn"]),
        "state": (text(state), want["state"]),
        "pane": (text(pane_got), pane),
        "host": (text(host), want.get("host", "m1-pro")),
        "sess": (text(sess), want.get("sess", "")),
    }
    if "agent" in want:
        checks["agent"] = (text(agent), want["agent"])
    if "dur" in want:
        checks["dur"] = (None if dur == "-" else int(dur), want["dur"])
    title_in = want.get("title")
    simple = title_in is not None and all(32 <= ord(c) < 127 or c in "\n\t" for c in title_in) and len(title_in) <= 120
    if simple:
        checks["title"] = (text(title), title_in.replace("\n", " ").replace("\t", " "))
    for name, (got, expected) in checks.items():
        if got != expected:
            problems.append(f"{source}: {name} {got!r} != {expected!r}")
    if not want["started"] <= int(ts) <= want["ended"]:
        problems.append(f"{source}: ts {ts} outside the run")


# MARK: - main

def run(record):
    problems = []
    random.seed(20261111)
    vectors = []  # (source, secret, envelope dict)

    # 1. The host script, every shell and openssl.
    script = script_vectors(problems)
    for source, secret, body, want in script:
        vectors.append((source, secret, body["env"], ("host", body, want)))

    # 2. Go and Swift seal the same plaintexts, IV pinned and drawn.
    pinned = []
    for i, plain in enumerate(PLAINTEXTS):
        secret = random.getrandbits(256).to_bytes(32, "big").hex()
        iv = random.getrandbits(128).to_bytes(16, "big").hex()
        pt = plain.encode().hex()
        g = json.loads(go([f"seal {secret} {iv} {pt}"])[0])
        gr = json.loads(go([f"seal {secret} - {pt}"])[0])
        s = harness(SWIFT, [f"seal {hexs(secret)} {hexs(iv)} {pt}"])[0]
        r = harness(RUST, [f"seal {hexs(secret)} {hexs(iv)} {pt}"])[0]
        go_line = f"ok 1 {hexs(g['iv'])} {hexs(g['ct'])} {hexs(g['mac'])}"
        if not (s == r == go_line):
            problems.append(f"pinned seal {i}: go {go_line[:80]} swift {s[:80]} rust {r[:80]}")
        pinned.append((f"seal {hexs(secret)} {hexs(iv)} {pt}", go_line))
        vectors.append((f"go pinned {i}", secret, g, None))
        vectors.append((f"go random {i}", secret, gr, None))
        _, v, siv, sct, smac = s.split(" ")
        vectors.append((f"swift pinned {i}", secret, {"v": int(v), "iv": bytes.fromhex(siv).decode(),
                                                      "ct": bytes.fromhex(sct).decode(),
                                                      "mac": bytes.fromhex(smac).decode()}, None))

    # 3. Rust seals with drawn IVs: Swift, Go and openssl must open them.
    rust_sealed = []
    for i, plain in enumerate(PLAINTEXTS):
        secret = random.getrandbits(256).to_bytes(32, "big").hex()
        # The harness pins IVs; draw one here so every run differs.
        iv = os.urandom(16).hex()
        line = harness(RUST, [f"seal {hexs(secret)} {hexs(iv)} {plain.encode().hex()}"])[0]
        _, v, riv, rct, rmac = line.split(" ")
        env = {"v": int(v), "iv": bytes.fromhex(riv).decode(), "ct": bytes.fromhex(rct).decode(),
               "mac": bytes.fromhex(rmac).decode()}
        vectors.append((f"rust {i}", secret, env, None))
        rust_sealed.append((secret, env, plain.encode()))

    # 4. Every envelope opened by every side.
    cases = [f"open {e['v']} {hexs(e['iv'])} {hexs(e['ct'])} {hexs(e['mac'])} {hexs(secret)}"
             for _, secret, e, _ in vectors]
    swift = harness(SWIFT, cases)
    rust = harness(RUST, cases)
    gos = go([f"open {secret} {json.dumps(e, separators=(',', ':')).encode().hex()}" for _, secret, e, _ in vectors])
    for (source, secret, env, extra), case, s, r, g in zip(vectors, cases, swift, rust, gos):
        if s != r:
            problems.append(f"{source}: swift {s[:120]} rust {r[:120]}")
        raw = bytes.fromhex(s.split(" ")[0][4:]) if s.startswith("raw=") else None
        if raw is None:
            problems.append(f"{source}: Swift did not open it: {s[:80]}")
            continue
        if g != f"ok {raw.hex()}":
            problems.append(f"{source}: go {g[:80]}")
        for openssl in OPENSSLS:
            if openssl_open(openssl, secret, env) != raw:
                problems.append(f"{source}: {openssl} did not open it")
        if extra and extra[0] == "host":
            status = s.split(" status=", 1)[1] if " status=" in s else None
            check_request(source, secret, extra[1], extra[2], status, problems)
    for secret, env, plain in rust_sealed:
        for openssl in OPENSSLS:
            if openssl_open(openssl, secret, env) != plain:
                problems.append(f"rust seal: {openssl} read something else")

    # 5. Send tokens: Go mints, Rust must agree (tests/interop.rs), the relay verifies.
    tokens = []
    for i in range(24):
        apns = random.getrandbits(256).to_bytes(32, "big").hex()
        apns = apns.upper() if i % 3 == 0 else apns
        conn = str(random.choice(["3F2504E0-4F89-11D3-9A0C-0305E82C3301", "c", "x" * 64]))
        iat = random.choice([1, 1_756_000_000, 2_000_000_000 + i])
        master = random.getrandbits(256).to_bytes(32, "big")
        token = go([f"mint {master.hex()} {apns} {conn} {iat}"])[0]
        tokens.append((master.hex(), apns, conn, iat, token))

    # 6. Pairing files: Swift writes, Rust reads, and back.
    pairing_cases = pairing_files(problems)

    print(f"{len(script)} host-script requests ({len(SHELLS)} shells x {len(OPENSSLS)} openssl builds), "
          f"{len(vectors) - len(script)} Go/Swift/Rust envelopes, all opened by Swift, Go, Rust and "
          f"{len(OPENSSLS)} openssl builds; {len(pinned)} pinned seals identical in Go, Swift and Rust; "
          f"{len(tokens)} send tokens; {len(pairing_cases)} pairing-file round trips")
    for problem in problems:
        print("PROBLEM", problem)
    print(f"{len(problems)} problems")

    if record and not problems:
        os.makedirs(FIXTURES, exist_ok=True)
        # Every Go, Swift and Rust envelope, and a quarter of the host
        # script's (every title still under some shell and openssl).
        keep = [i for i, (source, _, _, _) in enumerate(vectors) if not source.startswith("host") or i % 4 == 0]
        with open(os.path.join(FIXTURES, "envelopes.txt"), "w") as f:
            for i in keep:
                f.write(f"# {vectors[i][0]}\n{cases[i]}\n")
        with open(os.path.join(FIXTURES, "envelopes.swift.txt"), "w") as f:
            f.write("\n".join(swift[i] for i in keep) + "\n")
        with open(os.path.join(FIXTURES, "sealed.txt"), "w") as f:
            f.write("# seal cases with the IV pinned, and what Go's sealbox wrote for them\n")
            for case, result in pinned:
                f.write(f"{case}\n{result}\n")
        with open(os.path.join(FIXTURES, "tokens.txt"), "w") as f:
            f.write("# master apnsToken conn iat sendToken, minted by push-relay/token.go\n")
            for t in tokens:
                f.write(" ".join(str(x) for x in t) + "\n")
        with open(os.path.join(FIXTURES, "pairings.txt"), "w") as f:
            f.write("# pfile cases, and what Swift's PushPairingFile did\n")
            for case, result in pairing_cases:
                f.write(f"{case}\n{result}\n")
        print(f"recorded into {FIXTURES}")
    return 1 if problems else 0


def pairing_files(problems):
    """Swift's PushPairingFile writes; Rust reads what it wrote; and back."""
    rng = random.Random(7)
    def pairing(i, optional):
        p = {
            "connectionId": f"{rng.getrandbits(32):08X}-{rng.getrandbits(16):04X}-4{rng.getrandbits(12):03X}-"
                            f"8{rng.getrandbits(12):03X}-{rng.getrandbits(48):012X}",
            "hostLabel": ["m1-pro", "mac/mini", "构建机 😀", "q\"uote\\"][i % 4],
            "secretHex": rng.getrandbits(256).to_bytes(32, "big").hex(),
            "sendToken": "" if i % 3 == 0 else rng.getrandbits(256).to_bytes(32, "big").hex(),
            "relayURL": "https://push.example.org",
            "createdAt": [784000000, 784000000.5, 784123456.789012, 0.1][i % 4],
        }
        if optional:
            p.update({"apnsToken": "ab" * 32, "apnsEnv": "sandbox", "sendTokenIssuedAt": 784000001.25})
        return p
    lists = [[pairing(i, i % 2 == 0) for i in range(n)] for n in (0, 1, 3)]
    out = []
    for pairings in lists:
        listed = json.dumps(pairings, separators=(",", ":"), ensure_ascii=False).encode().hex()
        case = f"pfile - w:{listed} R"
        s = harness(SWIFT, [case])[0]
        written = s.split("file=", 1)[1].split(" ")[0]
        # What Swift wrote, read by Rust; what Rust wrote, read by Swift.
        back = f"pfile {written} R"
        if harness(RUST, [back])[0].split(" || ")[0] != harness(SWIFT, [back])[0].split(" || ")[0]:
            problems.append(f"Rust reads Swift's pairings file differently: {back[:80]}")
        r = harness(RUST, [case])[0]
        rust_written = r.split("file=", 1)[1].split(" ")[0]
        forth = f"pfile {rust_written} R"
        if harness(SWIFT, [forth])[0].split(" || ")[0] != s.split(" || ")[0].split(" | ", 1)[1]:
            problems.append(f"Swift reads Rust's pairings file differently: {forth[:80]}")
        out.append((back, harness(SWIFT, [back])[0]))
    # One with an entry from a future build, carried through a pairing.
    foreign = {"connectionId": "0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F9", "secret": {"v": 2}, "createdAt": 1}
    initial = json.dumps([pairing(5, True), foreign], separators=(",", ":"), ensure_ascii=False).encode().hex()
    case = f"pfile {initial} u:{json.dumps(pairing(6, False), separators=(',', ':')).encode().hex()} C R"
    s = harness(SWIFT, [case])[0]
    written = s.split("file=", 1)[1].split(" ")[0]
    back = f"pfile {written} C R"
    out.append((back, harness(SWIFT, [back])[0]))
    if harness(RUST, [back])[0].split(" || ")[0] != harness(SWIFT, [back])[0].split(" || ")[0]:
        problems.append("Rust reads Swift's carried pairings file differently")
    return out


def main():
    args = sys.argv[1:]
    command = args[0] if args and not args[0].startswith("--") else "run"
    if "--no-build" not in args:
        build()
    if command not in ("run", "record"):
        sys.exit(__doc__)
    sys.exit(run(command == "record"))


if __name__ == "__main__":
    main()
