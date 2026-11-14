#!/usr/bin/env python3
"""Makes tests/fixtures/keys/matrix: throwaway private keys in every form the
key reader meets, made by the tools people make them with, and what each must
come to.

    python3 -I harness/make_key_matrix.py [<ssh-keygen> [<openssl>]]

ssh-keygen (OpenSSH's, default /usr/bin/ssh-keygen) writes Ed25519, ECDSA on
the three NIST curves and RSA as openssh-key-v1 — plain, and behind a
passphrase with every cipher `-Z` offers — and as `-m PEM` and `-m PKCS8`,
plain and encrypted. OpenSSL 3 (default `openssl` on PATH) writes PKCS #8
under every PBES2 cipher and PRF, its traditional encrypted PEM, SEC1 with
the curve named or spelled out and with or without the public point, PKCS #1,
and the kinds the app refuses: multi-prime RSA, secp256k1, Brainpool, Ed448,
X25519, DSA, scrypt and PKCS #12 encryption.

`manifest.tsv`: `file <TAB> passphrase or - <TAB> expected`, expected being
`ssh-keygen -l`'s fingerprint for a key that imports (for OpenSSL's, worked
out here from `openssl pkey -pubout`'s public key, framed as SSH frames it —
never by the code under test) or `error <case> [<detail>]` for one that is
refused. The keys authorize nothing anywhere.
"""
import base64
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.dirname(HERE), "tests", "fixtures", "keys", "matrix")
SSH_KEYGEN = sys.argv[1] if len(sys.argv) > 1 else "/usr/bin/ssh-keygen"
OPENSSL = sys.argv[2] if len(sys.argv) > 2 else shutil.which("openssl")
PASS = "matrix pass"
CIPHERS = ["aes128-ctr", "aes192-ctr", "aes256-ctr", "aes128-cbc", "aes192-cbc", "aes256-cbc",
           "aes128-gcm@openssh.com", "aes256-gcm@openssh.com", "chacha20-poly1305@openssh.com"]

rows = []
work = tempfile.mkdtemp(prefix="key-matrix-")


def run(argv, **kw):
    return subprocess.run(argv, check=True, capture_output=True, **kw)


def keep(name, path, passphrase, expected):
    shutil.copyfile(path, os.path.join(OUT, name))
    rows.append((name, passphrase or "-", expected))


def keygen_fingerprint(pub_path):
    out = run([SSH_KEYGEN, "-lf", pub_path]).stdout.decode().split()
    return out[1]


# MARK: - ssh-keygen

KEYGEN_TYPES = [("ed25519", ["-t", "ed25519"]), ("ecdsa256", ["-t", "ecdsa", "-b", "256"]),
                ("ecdsa384", ["-t", "ecdsa", "-b", "384"]), ("ecdsa521", ["-t", "ecdsa", "-b", "521"]),
                ("rsa2048", ["-t", "rsa", "-b", "2048"])]

for name, args in KEYGEN_TYPES:
    base = os.path.join(work, name)
    run([SSH_KEYGEN, "-q", *args, "-N", "", "-C", f"{name}@matrix", "-f", base])
    fingerprint = keygen_fingerprint(base + ".pub")
    keep(f"keygen-{name}-openssh", base, None, fingerprint)
    for cipher in CIPHERS:
        path = os.path.join(work, f"{name}-{cipher}")
        shutil.copyfile(base, path)
        os.chmod(path, 0o600)
        run([SSH_KEYGEN, "-q", "-p", "-f", path, "-P", "", "-N", PASS, "-a", "2", "-Z", cipher])
        keep(f"keygen-{name}-openssh-{cipher.split('@')[0]}", path, PASS, fingerprint)
    for form in ["PEM", "PKCS8"]:
        for passphrase in [None, PASS]:
            path = os.path.join(work, f"{name}-{form}-{bool(passphrase)}")
            shutil.copyfile(base, path)
            os.chmod(path, 0o600)
            try:
                run([SSH_KEYGEN, "-q", "-p", "-f", path, "-P", "", "-N", passphrase or "", "-m", form])
            except subprocess.CalledProcessError:
                continue  # ssh-keygen writes Ed25519 only as its own format.
            if "OPENSSH" in open(path).readline():
                continue
            suffix = "-encrypted" if passphrase else ""
            keep(f"keygen-{name}-{form.lower()}{suffix}", path, passphrase, fingerprint)


# MARK: - OpenSSL

def ssh_string(data):
    return len(data).to_bytes(4, "big") + data


def mpint(value):
    raw = value.to_bytes((value.bit_length() + 7) // 8, "big")
    if raw and raw[0] & 0x80:
        raw = b"\0" + raw
    return ssh_string(raw)


def der_items(data):
    """The elements of a DER SEQUENCE's contents: (tag, contents)."""
    items, i = [], 0
    while i < len(data):
        tag, length = data[i], data[i + 1]
        i += 2
        if length & 0x80:
            n = length & 0x7F
            length = int.from_bytes(data[i:i + n], "big")
            i += n
        items.append((tag, data[i:i + length]))
        i += length
    return items


CURVES = {"prime256v1": ("ecdsa-sha2-nistp256", "nistp256"),
          "secp384r1": ("ecdsa-sha2-nistp384", "nistp384"),
          "secp521r1": ("ecdsa-sha2-nistp521", "nistp521")}


def openssl_fingerprint(path, kind, passphrase=None):
    """The SSH fingerprint of the key in `path`, from OpenSSL's public key."""
    argv = [OPENSSL, "pkey", "-in", path, "-pubout", "-outform", "DER"]
    if passphrase:
        argv += ["-passin", f"pass:{passphrase}"]
    spki = der_items(der_items(run(argv).stdout)[0][1])
    bits = spki[1][1][1:]  # BIT STRING, unused-bits byte first
    if kind == "ed25519":
        blob = ssh_string(b"ssh-ed25519") + ssh_string(bits)
    elif kind == "rsa":
        n, e = (int.from_bytes(v, "big") for _, v in der_items(der_items(bits)[0][1]))
        blob = ssh_string(b"ssh-rsa") + mpint(e) + mpint(n)
    else:
        key_type, curve = CURVES[kind]
        blob = ssh_string(key_type.encode()) + ssh_string(curve.encode()) + ssh_string(bits)
    digest = base64.b64encode(hashlib.sha256(blob).digest()).decode().rstrip("=")
    return f"SHA256:{digest}"


def openssl(*args, stdin=None):
    return run([OPENSSL, *args], input=stdin).stdout


def make(name, argv):
    path = os.path.join(work, name)
    run([OPENSSL, *argv, "-out", path])
    return path


def convert(source, name, argv, passphrase=None):
    path = os.path.join(work, name)
    extra = ["-passout", f"pass:{passphrase}"] if passphrase else []
    run([OPENSSL, *argv, "-in", source, "-out", path, *extra])
    return path


# Ed25519, PKCS #8 only.
ed = make("ed25519", ["genpkey", "-algorithm", "ed25519"])
ed_fp = openssl_fingerprint(ed, "ed25519")
keep("openssl-ed25519-pkcs8", ed, None, ed_fp)
for cipher in ["aes-128-cbc", "aes-192-cbc", "aes-256-cbc", "des3"]:
    path = convert(ed, f"ed-{cipher}", ["pkcs8", "-topk8", "-v2", cipher], PASS)
    keep(f"openssl-ed25519-pkcs8-pbes2-{cipher}", path, PASS, ed_fp)
for prf in ["hmacWithSHA1", "hmacWithSHA224", "hmacWithSHA256", "hmacWithSHA384", "hmacWithSHA512"]:
    path = convert(ed, f"ed-{prf}", ["pkcs8", "-topk8", "-v2", "aes-256-cbc", "-v2prf", prf], PASS)
    keep(f"openssl-ed25519-pkcs8-pbes2-{prf}", path, PASS, ed_fp)

# ECDSA on the NIST curves: SEC1 and PKCS #8, named and explicit, with and
# without the public point, and OpenSSL's traditional encryption.
for curve, kind in [("prime256v1", "prime256v1"), ("secp384r1", "secp384r1"), ("secp521r1", "secp521r1")]:
    short = {"prime256v1": "p256", "secp384r1": "p384", "secp521r1": "p521"}[curve]
    sec1 = make(f"ec-{short}", ["ecparam", "-genkey", "-noout", "-name", curve])
    fp = openssl_fingerprint(sec1, kind)
    keep(f"openssl-{short}-sec1", sec1, None, fp)
    keep(f"openssl-{short}-sec1-no-public",
         convert(sec1, f"ec-{short}-nopub", ["ec", "-no_public"]), None, fp)
    keep(f"openssl-{short}-sec1-explicit",
         convert(sec1, f"ec-{short}-explicit", ["ec", "-param_enc", "explicit"]), None, fp)
    keep(f"openssl-{short}-pkcs8", convert(sec1, f"ec-{short}-p8", ["pkcs8", "-topk8", "-nocrypt"]), None, fp)
    explicit = os.path.join(work, f"ec-{short}-explicit")
    keep(f"openssl-{short}-pkcs8-explicit",
         convert(explicit, f"ec-{short}-p8x", ["pkcs8", "-topk8", "-nocrypt"]), None, fp)
    keep(f"openssl-{short}-pkcs8-pbes2",
         convert(sec1, f"ec-{short}-p8e", ["pkcs8", "-topk8", "-v2", "aes-256-cbc"], PASS), PASS, fp)
    for cipher in ["aes128", "aes192", "aes256", "des3"]:
        keep(f"openssl-{short}-sec1-traditional-{cipher}",
             convert(sec1, f"ec-{short}-{cipher}", ["ec", f"-{cipher}"], PASS), PASS, fp)

# RSA: PKCS #8, PKCS #1, traditional encryption, PBES2, and multi-prime.
rsa = make("rsa", ["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"])
rsa_fp = openssl_fingerprint(rsa, "rsa")
keep("openssl-rsa2048-pkcs8", rsa, None, rsa_fp)
pkcs1 = convert(rsa, "rsa-pkcs1", ["rsa", "-traditional"])
keep("openssl-rsa2048-pkcs1", pkcs1, None, rsa_fp)
for cipher in ["aes128", "aes192", "aes256", "des3"]:
    keep(f"openssl-rsa2048-pkcs1-traditional-{cipher}",
         convert(rsa, f"rsa-{cipher}", ["rsa", "-traditional", f"-{cipher}"], PASS), PASS, rsa_fp)
keep("openssl-rsa2048-pkcs8-pbes2",
     convert(rsa, "rsa-p8e", ["pkcs8", "-topk8", "-v2", "aes-128-cbc"], PASS), PASS, rsa_fp)
keep("openssl-rsa2048-pkcs8-pkcs12-3des",
     convert(rsa, "rsa-pbe", ["pkcs8", "-topk8", "-v1", "PBE-SHA1-3DES"], PASS), PASS,
     "error UnsupportedEncryption PKCS #12 PBE")
keep("openssl-rsa2048-pkcs8-scrypt",
     convert(rsa, "rsa-scrypt", ["pkcs8", "-topk8", "-scrypt"], PASS), PASS,
     "error UnsupportedEncryption 1.3.6.1.4.1.11591.4.11")
multi = make("rsa-3primes", ["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048",
                             "-pkeyopt", "rsa_keygen_primes:3"])
keep("openssl-rsa2048-3primes-pkcs1", convert(multi, "rsa-3p-pkcs1", ["rsa", "-traditional"]), None,
     "error Unsupported multi-prime RSA")
keep("openssl-rsa2048-3primes-pkcs8", multi, None, "error Unsupported multi-prime RSA")

# The kinds the app can't sign in with.
k1 = make("ec-k1", ["ecparam", "-genkey", "-noout", "-name", "secp256k1"])
keep("openssl-secp256k1-sec1", k1, None, "error Unsupported ECDSA secp256k1")
keep("openssl-secp256k1-pkcs8", convert(k1, "ec-k1-p8", ["pkcs8", "-topk8", "-nocrypt"]), None,
     "error Unsupported ECDSA secp256k1")
keep("openssl-secp256k1-sec1-explicit", convert(k1, "ec-k1-x", ["ec", "-param_enc", "explicit"]), None,
     "error Unsupported ECDSA (custom curve)")
bp = make("ec-bp", ["ecparam", "-genkey", "-noout", "-name", "brainpoolP256r1"])
keep("openssl-brainpool256-sec1", bp, None, "error Unsupported ECDSA 1.3.36.3.3.2.8.1.1.7")
keep("openssl-ed448-pkcs8", make("ed448", ["genpkey", "-algorithm", "ed448"]), None,
     "error Unsupported Ed448")
keep("openssl-x25519-pkcs8", make("x25519", ["genpkey", "-algorithm", "x25519"]), None,
     "error Unsupported 1.3.101.110")
dsa_params = make("dsa-params", ["genpkey", "-genparam", "-algorithm", "DSA", "-pkeyopt", "dsa_paramgen_bits:1024"])
dsa = os.path.join(work, "dsa")
run([OPENSSL, "genpkey", "-paramfile", dsa_params, "-out", dsa])
keep("openssl-dsa-pkcs8", dsa, None, "error Unsupported DSA")
keep("openssl-dsa-traditional", convert(dsa, "dsa-trad", ["dsa"]), None, "error Unsupported DSA")

shutil.rmtree(work)
with open(os.path.join(OUT, "manifest.tsv"), "w") as manifest:
    manifest.write("# Made by harness/make_key_matrix.py with %s and %s; passphrase: %s\n"
                   % (subprocess.run(["ssh", "-V"], capture_output=True).stderr.decode().strip(),
                      run([OPENSSL, "version"]).stdout.decode().strip(), PASS))
    for row in rows:
        manifest.write("\t".join(row) + "\n")
print(f"{len(rows)} keys in {OUT}")
