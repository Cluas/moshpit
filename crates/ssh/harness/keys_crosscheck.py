#!/usr/bin/env python3
"""Cross-check: the key reader (src/keyfile) against the app's Swift one it
replaced (SSHKeyFactory / SSHPrivateKeyFile / BcryptPBKDF as of 81c121a1,
built by harness/build-swift.sh), case by case.

    python3 -I harness/keys_crosscheck.py [--no-build] [--reuse]

Every fixture key (tests/fixtures/keys: ssh-keygen's, the app's Swift test
keys, the matrix of every form, cipher and PRF, PuTTY's) in every way a
paste or a file can bring it — as is, without its passphrase, with an empty
or a wrong one, CRLF, indented, on one line, padded, cut short at quanta and
off them, with or without its end line, a byte garbled in its body, beside
its .pub, with a .pub line and a typed comment — through import, the
passphrase question, the stored-key assessment and the orphan description;
and hand-made texts (hollow bodies, security keys, DSA, noise).

`--reuse` classifies the answers the last run saved again, without
running either side. Both answer every case; each answer that differs must match one of the
documented divergences in harness/key_divergences.json (an `id` regex, the
Swift answer's and the Rust answer's), each with its why. Anything else is
a failure. Writes the cases and both sides' answers under
target/ssh-harness/keys/ and prints the tally.
"""
import base64
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
KEYS = os.path.join(CRATE, "tests", "fixtures", "keys")
WORK = os.path.join(REPO, "target", "ssh-harness", "keys")
SWIFT = os.path.join(REPO, "target", "ssh-harness", "ssh-swift")
RUST = os.path.join(REPO, "target", "release", "examples", "ssh_harness")

os.makedirs(os.path.join(WORK, "texts"), exist_ok=True)
cases = []


def text_file(name, text):
    path = os.path.join(WORK, "texts", name)
    with open(path, "w", newline="") as f:
        f.write(text)
    return path


def case(cid, op, path, passphrase=None, public=None, comment=""):
    cases.append([cid, op, path, passphrase or "-", public or "-", comment])


def known_keys():
    """(name, text, passphrase, .pub path or None) for every fixture key."""
    out = []
    for name in sorted(os.listdir(KEYS)):
        path = os.path.join(KEYS, name)
        if os.path.isfile(path) and not name.endswith(".pub"):
            pub = path + ".pub" if os.path.exists(path + ".pub") else None
            out.append(("keygen-" + name, open(path).read(),
                        "fixture" if name.endswith("-encrypted") else None, pub))
    app = json.load(open(os.path.join(KEYS, "app", "manifest.json")))
    for key in app["keys"]:
        text = open(os.path.join(KEYS, "app", key["file"])).read()
        pub = text_file("app-" + key["file"] + ".pub", key["publicKey"] + "\n")
        out.append(("app-" + key["file"], text, app["passphrase"] if key["encrypted"] else None, pub))
    for name in ["ecdsa-p384", "chacha-encrypted", "pkcs12-encrypted"]:
        out.append(("app-" + name, open(os.path.join(KEYS, "app", name)).read(),
                    None if name == "ecdsa-p384" else app["passphrase"], None))
    for line in open(os.path.join(KEYS, "matrix", "manifest.tsv")):
        if line.startswith("#"):
            continue
        name, passphrase, _ = line.rstrip("\n").split("\t")
        out.append(("matrix-" + name, open(os.path.join(KEYS, "matrix", name)).read(),
                    None if passphrase == "-" else passphrase, None))
    for name in sorted(os.listdir(os.path.join(KEYS, "putty"))):
        out.append(("putty-" + name, open(os.path.join(KEYS, "putty", name)).read(),
                    "123" if "_enc" in name else None, None))
    return out


def armour(text):
    """(first line, base64 body lines, last line) of a PEM text, or None."""
    lines = text.strip().split("\n")
    if not lines[0].startswith("-----BEGIN") or not lines[-1].startswith("-----END"):
        return None
    return lines[0], lines[1:-1], lines[-1]


for name, text, passphrase, pub in known_keys():
    def add(variant, body, op="import", pp=passphrase, public=None, comment=""):
        case(f"{name}/{variant}/{op}", op, text_file(f"{name}.{variant}", body), pp, public, comment)

    add("as-is", text)
    add("as-is", text, op="isencrypted")
    add("as-is", text, op="assess")
    add("as-is", text, op="assess", pp=None)
    add("as-is", text, op="describe", pp=None)
    if passphrase:
        add("no-passphrase", text, pp=None)
        add("empty-passphrase", text, pp="")
        add("wrong-passphrase", text, pp="wrong one")
        add("wrong-passphrase", text, op="assess", pp="wrong one")
    add("crlf", text.replace("\n", "\r\n"))
    add("crlf", text.replace("\n", "\r\n"), op="assess")
    add("indented", "\n".join("\t  " + l for l in text.split("\n")))
    add("indented", "\n".join("    " + l for l in text.split("\n")), op="assess")
    add("one-line", text.replace("\n", " "))
    add("padded", "\n\n  " + text + "  \n\n")
    add("typed-comment", text, comment="typed comment")
    if pub:
        add("with-pub", text, public=pub)
        add("with-pub-and-comment", text, public=pub, comment="typed")
        pub_text = open(pub).read()
        add("pub-then-key", pub_text + "\n" + text)
        add("key-then-pub", text + "\n" + pub_text)
    parts = armour(text)
    if parts is None:
        continue
    first, body, last = parts
    headers = [l for l in body if ":" in l]
    data = "".join(l for l in body if ":" not in l)
    head = "\n".join([first, *headers])
    for percent in [10, 25, 50, 75, 90, 99]:
        keep = len(data) * percent // 100
        on_quantum = keep - keep % 4
        add(f"cut{percent}", f"{head}\n{data[:on_quantum]}\n{last}\n")
        add(f"cut{percent}-no-end", f"{head}\n{data[:on_quantum]}\n")
        add(f"cut{percent}-off-quantum", f"{head}\n{data[:on_quantum + 2]}\n{last}\n")
    raw = base64.b64decode(data)
    for i in range(16):
        at = (len(raw) * i) // 16 + 3
        if at >= len(raw):
            continue
        garbled = bytearray(raw)
        garbled[at] ^= 0x5A
        encoded = base64.b64encode(bytes(garbled)).decode()
        lines = [encoded[j:j + 64] for j in range(0, len(encoded), 64)]
        add(f"garbled{at}", "\n".join([head, *lines, last]) + "\n")

# Hand-made texts.
def ssh_string(b):
    return len(b).to_bytes(4, "big") + b


def container(public_blob, section=b"\0" * 16, cipher=b"none"):
    payload = b"openssh-key-v1\0" + ssh_string(cipher)
    if cipher == b"none":
        payload += ssh_string(b"none") + ssh_string(b"")
    else:
        payload += ssh_string(b"bcrypt") + ssh_string(ssh_string(b"\7" * 16) + b"\0\0\0\x10")
    payload += b"\0\0\0\1" + ssh_string(public_blob) + ssh_string(section)
    encoded = base64.b64encode(payload).decode()
    lines = [encoded[j:j + 70] for j in range(0, len(encoded), 70)]
    return "-----BEGIN OPENSSH PRIVATE KEY-----\n" + "\n".join(lines) + "\n-----END OPENSSH PRIVATE KEY-----\n"


sk = ssh_string(b"sk-ecdsa-sha2-nistp256@openssh.com") + ssh_string(b"nistp256") + ssh_string(b"\4" * 65) + ssh_string(b"ssh:")
sk_ed = ssh_string(b"sk-ssh-ed25519@openssh.com") + ssh_string(b"\1" * 32) + ssh_string(b"ssh:")
dss = ssh_string(b"ssh-dss") + ssh_string(b"\1" * 20)
p384 = ssh_string(b"ecdsa-sha2-nistp384") + ssh_string(b"nistp384") + ssh_string(b"\4" * 97)
unknown = ssh_string(b"ssh-xmss@openssh.com") + ssh_string(b"\1" * 20)
made = {
    "sk-ecdsa": container(sk), "sk-ecdsa-encrypted": container(sk, cipher=b"aes256-ctr"),
    "sk-ed25519": container(sk_ed), "dss": container(dss), "dss-encrypted": container(dss, cipher=b"aes256-ctr"),
    "p384-stand-in": container(p384), "unknown-type": container(unknown),
    "3des-cbc": container(ssh_string(b"ssh-ed25519") + ssh_string(b"\1" * 32), cipher=b"3des-cbc"),
    "noise": "just some pasted text", "empty": "", "spaces": "   \n ",
    "public-line": open(os.path.join(KEYS, "ed25519.pub")).read(),
    "dsa-pem": "-----BEGIN DSA PRIVATE KEY-----\nAAAA\n-----END DSA PRIVATE KEY-----\n",
    "other-label": "-----BEGIN FOO PRIVATE KEY-----\nAAAA\n-----END FOO PRIVATE KEY-----\n",
    "certificate": "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
    "no-end": "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n",
    "bad-base64": "-----BEGIN PRIVATE KEY-----\nAA!A\n-----END PRIVATE KEY-----\n",
    "unknown-dek": "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: CAMELLIA-128-CBC,00112233445566778899AABBCCDDEEFF\n\nAAAA\n-----END RSA PRIVATE KEY-----\n",
}
for label in ["RSA PRIVATE KEY", "EC PRIVATE KEY", "PRIVATE KEY", "OPENSSH PRIVATE KEY", "ENCRYPTED PRIVATE KEY"]:
    made["hollow-" + label.replace(" ", "-").lower()] = f"-----BEGIN {label}-----\nAAAA\n-----END {label}-----"
for name, text in made.items():
    path = text_file("made-" + name, text)
    for op, pp in [("import", None), ("import", "pass"), ("isencrypted", None), ("assess", None), ("describe", None)]:
        case(f"made-{name}/{op}/{pp or '-'}", op, path, pp)


def build():
    env = dict(os.environ, CARGO_BUILD_JOBS="4")
    subprocess.run(["nice", "-n", "10", "cargo", "build", "-q", "--release", "-p", "moshpit-ssh",
                    "--example", "ssh_harness"], cwd=REPO, check=True, env=env)
    subprocess.run(["bash", os.path.join(HERE, "build-swift.sh")], check=True)


if "--no-build" not in sys.argv:
    build()
def answers(binary, which):
    path = os.path.join(WORK, f"cases.{os.path.basename(binary)}.tsv")
    with open(path, "w") as f:
        for c in which:
            f.write("\t".join(c) + "\n")
    out = subprocess.run(["nice", "-n", "10", binary, "keys", path], check=True,
                         capture_output=True).stdout.decode()
    return dict(line.split("\t", 1) for line in out.split("\n") if line)


def saved(side):
    with open(os.path.join(WORK, f"answers.{side}.tsv")) as f:
        return dict(line.rstrip("\n").split("\t", 1) for line in f if line.strip())


# Rust first: a case it refuses for asking more key derivation work than
# any key gets (a garbled bcrypt round count or PBKDF2 iteration count of
# billions) would hold the Swift reader for hours; it isn't given to it.
endless = re.compile(r"error UnsupportedEncryption (bcrypt with \d+ rounds|PBKDF2 with \d+ iterations)")
if "--reuse" in sys.argv:
    # Both sides' answers from the last run, classified again (after a
    # change to key_divergences.json).
    rust, swift = saved("rust"), saved("swift")
else:
    rust = answers(RUST, cases)
    swift = answers(SWIFT, [c for c in cases if not endless.fullmatch(rust[c[0]])])
for c in cases:
    swift.setdefault(c[0], "not run: hangs")
for side, got in [("swift", swift), ("rust", rust)]:
    with open(os.path.join(WORK, f"answers.{side}.tsv"), "w") as f:
        for c in cases:
            f.write(f"{c[0]}\t{got[c[0]]}\n")

divergences = json.load(open(os.path.join(HERE, "key_divergences.json")))
tally = {d["id"]: 0 for d in divergences}
same, unexplained = 0, []
for c in cases:
    cid = c[0]
    s, r = swift[cid], rust[cid]
    if s == r:
        same += 1
        continue
    for d in divergences:
        if re.search(d["case"], cid) and re.fullmatch(d["swift"], s, re.S) and re.fullmatch(d["rust"], r, re.S):
            tally[d["id"]] += 1
            break
    else:
        unexplained.append((cid, s, r))

print(f"{len(cases)} cases: {same} answered alike, "
      f"{sum(tally.values())} documented divergences, {len(unexplained)} unexplained")
for d in divergences:
    print(f"  {tally[d['id']]:5}  {d['id']}")
for cid, s, r in unexplained[:60]:
    print(f"UNEXPLAINED {cid}\n    swift: {s}\n    rust:  {r}")
sys.exit(1 if unexplained else 0)
