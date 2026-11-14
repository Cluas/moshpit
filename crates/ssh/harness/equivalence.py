#!/usr/bin/env python3
"""Cross-check: the app's Swift SSH pieces against moshpit-ssh.

    equivalence.py [run] [--no-build]
    equivalence.py update-goldens [--no-build]

`run` builds both front ends (harness/build-swift.sh over the app's own
SSHKeyFactory, HostKeyValidator.fingerprint(of:) and ExecUploadCommands,
Citadel's key detection and the swift-nio-ssh the app pins; and
examples/ssh_harness.rs) and asks both the same questions:

- fingerprint: public key lines — the fixtures, keys ssh-keygen makes for
  the occasion (Ed25519, ECDSA on all three curves, RSA of four sizes) and
  this machine's own /etc/ssh host keys — through NIOSSH's parser and the
  app's fingerprint function, and through ssh-key. Both must also equal
  `ssh-keygen -lf`.
- live: the key a real host shows each client in a real key exchange, and
  its fingerprint: this machine's sshd when it listens on port 22, and an
  sshd started here as the current user for each set of host keys (the
  machine's sshd and ~/.ssh are left alone).
- keytype, import: every fixture private key and its variants (CRLF,
  rewrapped, padded, truncated, garbled) through Citadel's detection and
  SSHKeyFactory.importKey, and through inspect_private_key.
- upload: ExecUploadCommands' plans for homes and names with spaces, quotes
  and non-ASCII.
- socks: the app's SOCKSProxyDialer and the Rust dialler through a proxy
  on the loopback that tunnels, refuses the CONNECT, wants a password,
  never answers, or isn't SOCKS.

Every answer must be identical, except the documented divergences in
harness/divergences.json: their Swift answers must be what is recorded
there, and the Rust answers must differ from them.

`update-goldens` regenerates the cases (tests/fixtures/swift/cases.txt and
the key variants beside it) and records the Swift answers
(cases.swift.txt) and, for the divergent cases, the Rust ones
(divergences.rust.txt) that tests/swift_equivalence.rs replays without
swiftc.
Everything runs niced, one process at a time.
"""
import glob
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
FIXTURES = os.path.join(CRATE, "tests", "fixtures")
GOLDENS = os.path.join(FIXTURES, "swift")
WORK = os.path.join(REPO, "target", "ssh-harness")
SWIFT = os.path.join(WORK, "ssh-swift")
RUST = os.path.join(REPO, "target", "debug", "examples", "ssh_harness")
NICE = ["nice", "-n", "10"]
SSHD = os.environ.get("MOSHPIT_SSH_TEST_SSHD", "/usr/sbin/sshd")

KEYS = ["ed25519", "rsa2048", "rsa4096", "ecdsa256", "ecdsa384", "ecdsa521",
        "ed25519-encrypted", "rsa2048-pem", "ecdsa256-pkcs8"]
GENERATED = [("ed25519", None, 8), ("ecdsa", 256, 4), ("ecdsa", 384, 4), ("ecdsa", 521, 4),
             ("rsa", 1024, 2), ("rsa", 2048, 2), ("rsa", 3072, 2), ("rsa", 4096, 2)]
UPLOADS = [("/home/dev", "IMG-1.jpg"), ("/Users/dev o'brien", "IMG-1.jpg"),
           ("/home/ünï", "Screenshot 2026-11-11 at 10.00.00.png"), ("/tmp/a b", "it's.png"),
           ("/root", "IMG-2.heic")]
HOST_KEY_SETS = [["ed25519"], ["ecdsa256"], ["ecdsa384"], ["ecdsa521"], ["rsa2048"],
                 ["rsa2048", "ecdsa256", "ecdsa384", "ed25519"], ["ecdsa256", "ecdsa384"],
                 ["ecdsa521", "ecdsa256", "rsa2048"], ["ecdsa521", "rsa2048"]]


def sh(argv, **kwargs):
    return subprocess.run(NICE + argv, **kwargs)


def build():
    sh([os.path.join(HERE, "build-swift.sh"), SWIFT], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    sh(["cargo", "build", "--quiet", "-p", "moshpit-ssh", "--example", "ssh_harness"],
       check=True, cwd=REPO, env=env)


def ask(binary, args):
    out = subprocess.run([binary] + args, capture_output=True, text=True, timeout=60)
    if out.returncode != 0:
        raise SystemExit("%s %s failed: %s" % (os.path.basename(binary), args, out.stderr))
    return out.stdout.rstrip("\n")


def keygen_fingerprint(public_line):
    with tempfile.NamedTemporaryFile("w", suffix=".pub", delete=False) as f:
        f.write(public_line + "\n")
    try:
        out = subprocess.run(["ssh-keygen", "-lf", f.name], capture_output=True, text=True, check=True)
        return out.stdout.split()[1]
    finally:
        os.unlink(f.name)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def key_variants():
    """Variants of the Ed25519 fixture, as a paste might bring them."""
    text = read(os.path.join(FIXTURES, "keys", "ed25519"))
    lines = text.strip("\n").split("\n")
    body = "".join(lines[1:-1])
    wrap = lambda width: "\n".join(body[i:i + width] for i in range(0, len(body), width))
    return {
        "crlf": text.replace("\n", "\r\n"),
        "wrap64": "%s\n%s\n%s\n" % (lines[0], wrap(64), lines[-1]),
        "one-line": "%s\n%s\n%s\n" % (lines[0], body, lines[-1]),
        "padded": "\n\n  " + text + "\n\n",
        "no-final-newline": text.rstrip("\n"),
        "indented": "\n".join("    " + l for l in lines) + "\n",
        "truncated": "%s\n%s\n%s\n" % (lines[0], body[: len(body) // 2], lines[-1]),
        "garbled": "%s\n%s\n%s\n" % (lines[0], body[:40] + "*" + body[41:], lines[-1]),
        "body-only": body + "\n",
    }


def update_goldens():
    keys_dir = os.path.join(GOLDENS, "keys")
    if os.path.isdir(keys_dir):
        shutil.rmtree(keys_dir)
    os.makedirs(keys_dir)
    cases = []
    # Public key lines: the fixtures', and fresh ones.
    for name in KEYS:
        line = read(os.path.join(FIXTURES, "keys", name + ".pub")).strip()
        cases.append(("fp-%s" % name, "fingerprint", [line], keygen_fingerprint(line)))
    with tempfile.TemporaryDirectory() as tmp:
        n = 0
        for kind, bits, count in GENERATED:
            for i in range(count):
                n += 1
                path = os.path.join(tmp, "k%d" % n)
                argv = ["ssh-keygen", "-q", "-t", kind, "-N", "", "-C", "", "-f", path]
                if bits:
                    argv[4:4] = ["-b", str(bits)]
                subprocess.run(argv, check=True)
                line = read(path + ".pub").strip()
                label = "%s%s-%d" % (kind, bits or "", i)
                cases.append(("fp-gen-" + label, "fingerprint", [line], keygen_fingerprint(line)))
    # Private keys: the fixtures, and variants.
    for name in KEYS:
        rel = os.path.join("keys", name)
        cases.append(("keytype-%s" % name, "keytype", ["fixtures/" + rel], None))
        cases.append(("import-%s" % name, "import", ["fixtures/" + rel], None))
    for label, text in key_variants().items():
        with open(os.path.join(keys_dir, "ed25519-" + label), "w", encoding="utf-8", newline="") as f:
            f.write(text)
        rel = "fixtures/swift/keys/ed25519-" + label
        cases.append(("keytype-ed25519-" + label, "keytype", [rel], None))
        cases.append(("import-ed25519-" + label, "import", [rel], None))
    for i, (home, name) in enumerate(UPLOADS):
        cases.append(("upload-%d" % i, "upload", [home, name], None))

    with open(os.path.join(GOLDENS, "cases.txt"), "w", encoding="utf-8") as f:
        f.write("# id <TAB> command <TAB> arguments as JSON <TAB> ssh-keygen's fingerprint or -\n")
        f.write("# Made by harness/equivalence.py update-goldens; file arguments are under tests/.\n")
        for case_id, command, args, keygen in cases:
            f.write("%s\t%s\t%s\t%s\n" % (case_id, command, json.dumps(args, ensure_ascii=False), keygen or "-"))
    rules = divergences()
    with open(os.path.join(GOLDENS, "cases.swift.txt"), "w", encoding="utf-8") as swift, \
            open(os.path.join(GOLDENS, "divergences.rust.txt"), "w", encoding="utf-8") as rust:
        swift.write("# id <TAB> what the Swift side answered, as JSON\n")
        rust.write("# id <TAB> divergence <TAB> what the Rust side answers instead, as JSON\n")
        for case_id, command, args, _ in cases:
            swift.write("%s\t%s\n" % (case_id, json.dumps(answer(SWIFT, command, args), ensure_ascii=False)))
            rule = divergent(rules, case_id, command, args)
            if rule:
                rust.write("%s\t%s\t%s\n" % (case_id, rule["id"], json.dumps(answer(RUST, command, args), ensure_ascii=False)))
    print("%d cases recorded" % len(cases))


def load_cases():
    cases = []
    for line in read(os.path.join(GOLDENS, "cases.txt")).splitlines():
        if not line or line.startswith("#"):
            continue
        case_id, command, args, keygen = line.split("\t")
        cases.append((case_id, command, json.loads(args), None if keygen == "-" else keygen))
    return cases


def answer(binary, command, args):
    if command in ("keytype", "import"):
        args = [os.path.join(CRATE, "tests", args[0])]
    return ask(binary, [command] + args)


def divergences():
    return json.loads(read(os.path.join(HERE, "divergences.json")))


def divergent(rules, case_id, command, args):
    for rule in rules:
        if rule["command"] == command and re.search(rule["case"], case_id):
            return rule
    return None


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def start_sshd(tmp, host_keys):
    config = []
    for name in host_keys:
        path = os.path.join(tmp, "host_" + name)
        shutil.copy(os.path.join(FIXTURES, "keys", name), path)
        os.chmod(path, 0o600)
        config.append("HostKey " + path)
    port = free_port()
    config += ["Port %d" % port, "ListenAddress 127.0.0.1", "PidFile " + os.path.join(tmp, "pid"),
               "UsePAM no", "PasswordAuthentication no", "KbdInteractiveAuthentication no",
               "AuthorizedKeysFile none", "StrictModes no"]
    path = os.path.join(tmp, "sshd_config")
    with open(path, "w") as f:
        f.write("\n".join(config) + "\n")
    process = subprocess.Popen([SSHD, "-D", "-f", path, "-E", os.path.join(tmp, "log")],
                               stdin=subprocess.DEVNULL)
    for _ in range(100):
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
            return process, port
        except OSError:
            time.sleep(0.05)
    process.kill()
    raise SystemExit("sshd didn't start: " + read(os.path.join(tmp, "log")))


def live_cases():
    """(id, host, port, keys) for every live host available here."""
    hosts = []
    if os.environ.get("MOSHPIT_SSH_HARNESS_NO_SYSTEM_SSHD"):
        # A run that must not knock on the machine's own sshd (an agent on a
        # shared Mac): its key exchange is left out, the private ones stay.
        print("live: MOSHPIT_SSH_HARNESS_NO_SYSTEM_SSHD set, skipping this machine's sshd")
    else:
        try:
            socket.create_connection(("127.0.0.1", 22), timeout=1).close()
            hosts.append(("live-this-machine", "127.0.0.1", 22, None))
        except OSError:
            print("live: nothing on port 22, skipping this machine's sshd")
    if not os.access(SSHD, os.X_OK):
        print("live: no sshd at %s, skipping private hosts" % SSHD)
        return hosts, []
    return hosts, HOST_KEY_SETS


class Socks5:
    """A SOCKS5 proxy on the loopback, behaving as `mode` says."""

    def __init__(self, mode):
        self.mode = mode
        self.server = socket.socket()
        self.server.bind(("127.0.0.1", 0))
        self.server.listen(16)
        self.port = self.server.getsockname()[1]
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while True:
            try:
                client, _ = self.server.accept()
            except OSError:
                return
            threading.Thread(target=self.one, args=(client,), daemon=True).start()

    def one(self, client):
        try:
            if self.mode == "silent":
                time.sleep(30)
                return
            if self.mode == "not-socks":
                client.sendall(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                time.sleep(2)
                return
            head = client.recv(2)
            client.recv(head[1])
            if self.mode == "wants-password":
                client.sendall(bytes([5, 0xFF]))
                time.sleep(2)
                return
            client.sendall(bytes([5, 0]))
            request = client.recv(4)
            if request[3] == 1:
                host = socket.inet_ntoa(client.recv(4))
            elif request[3] == 4:
                host = socket.inet_ntop(socket.AF_INET6, client.recv(16))
            else:
                host = client.recv(client.recv(1)[0]).decode()
            port = int.from_bytes(client.recv(2), "big")
            if self.mode == "refuse":
                client.sendall(bytes([5, 5, 0, 1, 0, 0, 0, 0, 0, 0]))
                time.sleep(2)
                return
            upstream = socket.create_connection((host, port), timeout=5)
            client.sendall(bytes([5, 0, 0, 1, 127, 0, 0, 1]) + port.to_bytes(2, "big"))
            time.sleep(2)
            upstream.close()
        except OSError:
            pass
        finally:
            client.close()

    def close(self):
        self.server.close()


SOCKS_MODES = ["tunnel", "refuse", "wants-password", "silent", "not-socks"]


def run():
    rules = divergences()
    failures = []
    seen_rules = set()
    count = 0

    def check(case_id, command, args, swift, rust, extra=None):
        nonlocal count
        count += 1
        rule = divergent(rules, case_id, command, args)
        if rule:
            seen_rules.add(rule["id"])
            if not re.fullmatch(rule["swift"], swift):
                failures.append("%s: Swift answered %r, divergence %s expects %r" % (case_id, swift, rule["id"], rule["swift"]))
            if rust == swift or not re.fullmatch(rule["rust"], rust):
                failures.append("%s: Rust answered %r, divergence %s expects %r" % (case_id, rust, rule["id"], rule["rust"]))
        elif swift != rust:
            failures.append("%s: Swift %r, Rust %r" % (case_id, swift, rust))
        if extra and extra != rust:
            failures.append("%s: %s" % (case_id, extra(rust)))

    for case_id, command, args, keygen in load_cases():
        swift = answer(SWIFT, command, args)
        rust = answer(RUST, command, args)
        expected_rust = None
        if keygen:
            kind = args[0].split()[0]
            expected_rust = "%s %s" % (kind, keygen)
        check(case_id, command, args, swift, rust,
              (lambda r, e=expected_rust: "Rust %r, ssh-keygen %r" % (r, e)) if expected_rust and rust != expected_rust else None)

    # This machine's own host keys (not recorded: they are its identity).
    for path in sorted(glob.glob("/etc/ssh/ssh_host_*_key.pub")):
        line = read(path).strip()
        if line.split()[0] not in ("ssh-ed25519", "ssh-rsa") and not line.startswith("ecdsa-"):
            continue
        case_id = "fp-this-machine-" + os.path.basename(path).split("_")[2]
        swift, rust = ask(SWIFT, ["fingerprint", line]), ask(RUST, ["fingerprint", line])
        expected = "%s %s" % (line.split()[0], keygen_fingerprint(line))
        print("%s: Swift %s / Rust %s / ssh-keygen %s" % (case_id, swift, rust, expected.split()[1]))
        check(case_id, "fingerprint", [line], swift, rust,
              (lambda r, e=expected: "Rust %r, ssh-keygen %r" % (r, e)) if rust != expected else None)

    hosts, sets = live_cases()
    for case_id, host, port, _ in hosts:
        swift = ask(SWIFT, ["live", host, str(port)])
        rust = ask(RUST, ["live", host, str(port)])
        print("%s: Swift %s / Rust %s" % (case_id, swift, rust))
        check(case_id, "live", [host, str(port)], swift, rust)
    for keys in sets:
        with tempfile.TemporaryDirectory() as tmp:
            process, port = start_sshd(tmp, keys)
            try:
                case_id = "live-" + "+".join(keys)
                swift = ask(SWIFT, ["live", "127.0.0.1", str(port)])
                rust = ask(RUST, ["live", "127.0.0.1", str(port)])
                print("%s: Swift %s / Rust %s" % (case_id, swift, rust))
                check(case_id, "live", keys, swift, rust)
            finally:
                process.kill()
                process.wait()

    target = socket.socket()
    target.bind(("127.0.0.1", 0))
    target.listen(16)
    target_port = target.getsockname()[1]
    for mode in SOCKS_MODES:
        proxy = Socks5(mode)
        try:
            case_id = "socks-" + mode
            args = ["socks", str(proxy.port), "127.0.0.1", str(target_port)]
            swift, rust = ask(SWIFT, args), ask(RUST, args)
            print("%s: Swift %s / Rust %s" % (case_id, swift, rust))
            check(case_id, "socks", [mode], swift, rust)
        finally:
            proxy.close()
    target.close()

    unused = [r["id"] for r in rules if r["id"] not in seen_rules]
    print("%d cases, %d divergences used%s" % (count, len(seen_rules),
          ("; unused: " + ", ".join(unused)) if unused else ""))
    if failures:
        print("\n".join(failures))
        sys.exit(1)
    print("all equal, apart from the documented divergences")


def main():
    args = sys.argv[1:]
    no_build = "--no-build" in args
    args = [a for a in args if a != "--no-build"]
    if not no_build:
        build()
    if args and args[0] == "update-goldens":
        update_goldens()
    else:
        run()


if __name__ == "__main__":
    main()
