#!/usr/bin/env python3
"""Hold the core's host care to the app's Swift host care it replaced (W15b).

Runs every scenario through both — `harness/build-swift.sh`'s binary (the
app's HostAutoCare, HostSetupModel and PushService at 81c121a1) and
`examples/crosscheck.rs` (the machine) — against the same simulated host,
and compares, per scenario: every command sent to the host (a file write
by what it writes), the files the host ends with, the pairings the phone
ends with, the consent question, the mints, and what a Host Setup screen
shows. What a run draws at random or from the clock — a pairing's secret, a
self-test's nonce, a manifest's dates — is masked. Differences are listed
against `harness/divergences.json`, which explains each one; an
unexplained difference fails.

    crates/hostcare/harness/crosscheck.py [scenario ...]
"""

import base64
import hashlib
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))

SCENARIOS = [
    "inspect", "inspect-fails", "install-claude", "install-codex", "install-broken-json",
    "install-then-remove", "pair", "pair-missing-tools", "pair-empty-relay", "unpair",
    "unpair-cannot-forget", "prove-hooks-no-pane", "prove-hooks-elsewhere", "prove-hooks-stamped",
    "prove-hooks-codex-empty", "prove-push-sender-output", "care-fresh", "care-hooked",
    "care-stale-sender", "care-stale-stamp", "care-ready-pairing", "care-wrong-relay",
    "care-notifications-off", "care-unsaved", "care-grant", "care-declined",
    "prove-push-exec-fails", "care-relay-trailing-slash",
    "care-conf-deleted", "care-hooks-removed",
]

# The machine's look at what the host actually holds (B315,
# `moshpit_install::drift`): one read-only command the Swift never sent.
# Left out of the comparison; what it found changes what is done next,
# which is compared.
DRIFT = "printf '%s=%s\\n' 0 "

# The machine's look at which pairings push to this phone (B354,
# `moshpit_install::commands::PAIRING_PHONES`): read-only, never sent by
# the Swift. Left out like DRIFT; what it finds (a conf taken off) is
# compared.
PHONES = 'for f in "$HOME/.moshpit/push.d"/*.conf; do'

# The machine's look at which agents' hooks have run on the host (B380,
# `moshpit_install::commands::AGENTS_PROBE`): read-only, never sent by the
# Swift. Left out like DRIFT; what it finds (Codex's trust step) is
# compared through what a Host Setup screen shows.
AGENTS = "printf 'seen=%s\\n' "

WRITE = re.compile(r"^set -e; mkdir -p \"[^\"]*\"; umask 077; printf %s '([A-Za-z0-9+/=]*)' \| base64 -d > \"([^\"]*)\"; chmod (\d+) \"[^\"]*\"$")

# The machine's writes land beside the file and replace it whole, after
# following its links (install's atomic-write, B351/B379), and an agent
# config keeps its own mode (config-mode, B352): both are documented in
# crates/install/harness/divergences.json. Compared here by what they
# write, as Swift's in-place write is.
ATOMIC_WRITE = re.compile(r"^set -e; mkdir -p \"[^\"]*\"; umask 077; p=\"([^\"]*)\"; .*? t=\"\$p\.moshpit-new\.\$\$\"; "
                          r"trap 'rm -f \"\$t\"' EXIT; (?:n=; if \[ -f \"\$p\" \]; then cp -p \"\$p\" \"\$t\"; else n=1; fi; )?"
                          r"printf %s '([A-Za-z0-9+/=]*)' \| base64 -d > \"\$t\"; (?:\[ -z \"\$n\" \] \|\| )?chmod (\d+) \"\$t\"; "
                          r"mv -f \"\$t\" \"\$p\"$")

# The host scripts each side ships: Swift's as they were at the revision
# build-swift.sh compiles (its HostScripts literals were those files byte
# for byte), the machine's as they are now. What changed in them since
# (install's B350, B357, B358) is not host care's to judge: each side's
# scripts and their digests are masked to the same names.
SWIFT_REV = os.environ.get("MOSHPIT_SWIFT_CARE_REV", "81c121a1")
SCRIPTS = {"stamp": "scripts/host/moshpit-stamp.sh", "sender": "scripts/host/moshpit-push.sh"}


def scripts_at(rev):
    out = {}
    for name, rel in SCRIPTS.items():
        if rev is None:
            text = open(os.path.join(ROOT, rel)).read()
        else:
            text = subprocess.run(["git", "-C", ROOT, "show", f"{rev}:{rel}"], check=True,
                                  capture_output=True, text=True).stdout
        out[name] = text.rstrip("\n")
    return out


def script_masker(scripts):
    swaps = []
    for name, body in scripts.items():
        digest = hashlib.sha256(body.encode()).hexdigest()
        swaps += [(body, f"<{name} script>"), (digest, f"<{name} digest>"), (digest[:8], f"<{name} digest>")]
    def apply(text):
        for old, new in swaps:
            text = text.replace(old, new)
        return text
    return apply


# The manifest lock (install's manifest-lock, B373): the machine takes it,
# reads the manifest again and lets go around every manifest write; the
# Swift wrote back what it read first. The lock, that read and the release
# are left out; the write is compared.
LOCK = 'd="$HOME/.moshpit/.lock"; mkdir -p '
UNLOCK = 'd="$HOME/.moshpit/.lock"; a=$(cat '


def without_lock(commands):
    out, skip = [], False
    for c in commands:
        if skip:
            skip = False
            continue
        if c.startswith(LOCK):
            skip = True
            continue
        if c.startswith(UNLOCK):
            continue
        out.append(c)
    return out


# The preflight's question about agents (install's agent-probe, B374) is
# the machine's alone: compared without it.
AGENT_PROBE = "; for a in 'claude "


def mask(text):
    text = re.sub(r"SECRET=[0-9a-f]{64}", "SECRET=<secret>", text)
    text = re.sub(r"selftest-[0-9a-f]+", "selftest-<nonce>", text)
    text = re.sub(r"SEND_IAT=\d+", "SEND_IAT=<now>", text)
    text = re.sub(r'"installedAt" ?: ?"[^"]*"', '"installedAt":"<date>"', text)
    return text


def command(c):
    m = WRITE.match(c)
    if m:
        payload, path, mode = m.group(1), m.group(2), m.group(3)
    else:
        m = ATOMIC_WRITE.match(c)
        if not m:
            if c.startswith("printf 'home=") and AGENT_PROBE in c:
                c = c[: c.index(AGENT_PROBE)]
            return mask(c)
        path, payload, mode = m.group(1), m.group(2), m.group(3)
    body = base64.b64decode(payload).decode("utf-8", "replace")
    return f"WRITE {path} {mode}\n{mask(body)}"


PAIRING_DIGEST = re.compile(r'("pairing\.[0-9A-F-]+" : \{[^}]*"digest" : ")([0-9a-f]{64})(")')


def mask_pairing_digests(text, confs):
    """A pairing's recorded digest is of a conf holding a random secret:
    masked, once checked against the conf the host holds (when it holds
    one)."""
    def one(m):
        conn = m.group(1).split('"')[1].split(".", 1)[1]
        conf = confs.get(f"$HOME/.moshpit/push.d/{conn}.conf")
        if conf is not None and hashlib.sha256(conf.encode()).hexdigest() != m.group(2):
            return m.group(1) + "<digest of another conf>" + m.group(3)
        return m.group(1) + "<conf digest>" + m.group(3)
    return PAIRING_DIGEST.sub(one, text)


def normal(result, scripts=lambda t: t):
    out = dict(result)
    confs = result["files"]
    out["commands"] = [scripts(mask_pairing_digests(command(c), {})) for c in without_lock(result["commands"])
                       if not c.startswith(DRIFT) and not c.startswith(PHONES)
                       and not c.startswith(AGENTS)]
    out["looked"] = sum(1 for c in result["commands"] if c.startswith(DRIFT))
    out["files"] = {k: scripts(mask(mask_pairing_digests(v, confs))) for k, v in result["files"].items()}
    out["phone"] = [dict(p, secret="<secret>") for p in result["phone"]]
    return out


def runs(binary, names, scripts):
    text = subprocess.run([binary, *names], check=True, capture_output=True, text=True).stdout
    return {r["scenario"]: normal(r, scripts) for r in map(json.loads, text.splitlines())}


def diff(a, b, path=""):
    """Where `a` (Swift) and `b` (Rust) differ, as (path, swift, rust).
    Time waited is the Swift side's wall clock against the Rust side's
    simulated one: within a second is the same."""
    if path == ".waited" and isinstance(a, int) and isinstance(b, int) and abs(a - b) <= 1:
        return []
    if path == ".looked":
        return []
    if isinstance(a, dict) and isinstance(b, dict):
        out = []
        for key in sorted(set(a) | set(b)):
            out += diff(a.get(key), b.get(key), f"{path}.{key}")
        return out
    if isinstance(a, list) and isinstance(b, list):
        if a == b:
            return []
        # The first index that differs, and the lengths.
        n = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        return [(f"{path}[{n}] (of {len(a)} / {len(b)})",
                 a[n] if n < len(a) else None, b[n] if n < len(b) else None)]
    return [] if a == b else [(path, a, b)]


def main():
    names = sys.argv[1:] or SCENARIOS
    swift_bin = os.path.join(ROOT, "target", "hostcare-harness", "care-swift")
    if not os.path.exists(swift_bin):
        subprocess.run([os.path.join(HERE, "build-swift.sh")], check=True)
    target = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
    subprocess.run(["cargo", "build", "--quiet", "-p", "moshpit-hostcare", "--example", "crosscheck"],
                   check=True, cwd=ROOT)
    rust_bin = os.path.join(target, "debug", "examples", "crosscheck")
    swift = runs(swift_bin, names, script_masker(scripts_at(SWIFT_REV)))
    rust = runs(rust_bin, names, script_masker(scripts_at(None)))
    known = json.load(open(os.path.join(HERE, "divergences.json")))
    same = explained = 0
    unexplained = []
    for name in names:
        found = diff(swift[name], rust[name])
        if not found:
            same += 1
            print(f"same        {name}")
            continue
        why = known.get(name)
        if why:
            explained += 1
            print(f"explained   {name}: {why['id']}")
        else:
            unexplained.append(name)
            print(f"DIFFERENT   {name}")
        for where, a, b in found:
            print(f"    {where}\n      swift: {json.dumps(a)[:400]}\n      rust:  {json.dumps(b)[:400]}")
    print(f"\n{len(names)} scenarios: {same} the same, {explained} explained, {len(unexplained)} unexplained")
    sys.exit(1 if unexplained else 0)


if __name__ == "__main__":
    main()
