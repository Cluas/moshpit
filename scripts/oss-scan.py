#!/usr/bin/env python3
"""The leak scan: what must never reach this public repository, checked on
what is about to, the lines a range of commits adds and their messages.

    python3 -I scripts/oss-scan.py --head <rev> [--base <rev>] [--public]
    python3 -I scripts/oss-scan.py --tree <rev> [--public]

--head/--base: the commits base..head (base omitted, or all zeros as a new
ref's is: where head leaves the remote's main, else the whole history).
--tree: every file of one tree (an audit, not a gate).

Two sets of rules:

- Public ones, here, that anyone and CI run: access tokens, App Store
  Connect key ids, private-key material, device UDIDs, home folders other
  than the fixtures' generic ones, tailnet (CGNAT) addresses outside the
  fixtures' 100.64.0.0/24.
- Private ones, never in this repository: the maintainer's denylist of
  literal strings and the team's agent names (in commit messages), read
  from a checkout of the private ops repository ($MOSHPIT_OPS, default
  ../moshpit-ops next to this checkout: app/oss/denylist.txt,
  app/oss/agent-names.txt), and the signing Team ID of the local
  Signing.xcconfig. Without them the scan fails closed; --public runs the
  public rules alone (CI, contributors outside the team).

Exit 0 when clean, 2 with every leak listed, 64 on bad usage.
The pre-push hook (scripts/git-hooks/pre-push) runs it on what a push
publishes; .github/workflows/leak-scan.yml runs the public rules on every
push and pull request.
"""

import argparse
import ipaddress
import os
import re
import subprocess
import sys

ZERO = "0" * 40
SELF = "scripts/oss-scan.py"

TOKENS = re.compile(
    r"ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKI[AD][A-Z0-9]{16,}"
    r"|tskey-[a-z]+-[A-Za-z0-9]{10,}|xox[bap]-[A-Za-z0-9-]{10,}"
    r"|(?<![A-Za-z0-9])sk-[A-Za-z0-9_-]{24,}|hf_[A-Za-z0-9]{30,}"
)
ASC = re.compile(r"ASC_(KEY|ISSUER)_ID=[A-Za-z0-9]|AuthKey_[A-Z0-9]*[A-WYZ0-9][A-Z0-9]*\.p8")
# A bare 64-character base64 line, as a PEM body wraps; not lowercase hex (a
# SHA-256, as the store's goldens list).
KEY_LINE = re.compile(r"^[A-Za-z0-9+/]{64}$")
HEX_LINE = re.compile(r"^[0-9a-f]{64}$")
# Throwaway keys made for the key reader's tests: they authorize nothing.
KEY_FIXTURES = re.compile(
    r"^(crates/ssh/tests/fixtures/(keys|swift/keys)|Tests/MoshpitTests/Fixtures/keys-written-by-rust)/"
)
UDID = re.compile(r"(?<![0-9A-Fa-f])0000[0-9]{4}-[0-9A-Fa-f]{16}(?![0-9A-Fa-f])")
MAC_HOME = re.compile(r"(?<![\w:/])/Users/([A-Za-z0-9._-]+)")
LINUX_HOME = re.compile(r"(?<![\w:/])/home/([A-Za-z0-9._-]+)")
# The homes the fixtures use; anyone else's is a person's.
MAC_HOMES = {"dev", "Shared", "runner", "example", "me", "user"}
LINUX_HOMES = {"dev", "deploy", "me", "user", "ubuntu", "runner", "example", "ci", "git", "cluas", "pi"}
IPV4 = re.compile(r"(?<![\d.])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?![\d.])")
TAILNET = ipaddress.ip_network("100.64.0.0/10")
TAILNET_FIXTURES = ipaddress.ip_network("100.64.0.0/24")


def git(*args, check=True):
    out = subprocess.run(["git", *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        sys.exit(f"oss-scan: git {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout


def private_lists(root):
    """The denylist and the agent names, from the ops checkout; None when it
    isn't there."""
    ops = os.environ.get("MOSHPIT_OPS") or os.path.join(os.path.dirname(root), "moshpit-ops")
    deny, names = (os.path.join(ops, "app", "oss", f) for f in ("denylist.txt", "agent-names.txt"))
    if not (os.path.isfile(deny) and os.path.isfile(names)):
        return None

    def read(path):
        with open(path, encoding="utf-8") as f:
            return [w.rstrip("\n") for w in f if w.strip() and not w.startswith("#")]

    return read(deny), read(names)


def team_id(root):
    try:
        with open(os.path.join(root, "Signing.xcconfig"), encoding="utf-8") as f:
            m = re.search(r"^DEVELOPMENT_TEAM = *([A-Z0-9]{10})", f.read(), re.M)
            return m.group(1) if m else None
    except OSError:
        return None


def added_lines(base, head):
    """(path, line number, text) for every line base..head adds."""
    diff = git("diff", "--no-color", "--no-ext-diff", "-U0", "--no-renames", base, head)
    path, number = None, 0
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@"):
            m = re.search(r"\+(\d+)", line)
            number = int(m.group(1)) if m else 0
        elif line.startswith("+") and path:
            yield path, number, line[1:]
            number += 1


def tree_lines(rev):
    for path in git("ls-tree", "-r", "--name-only", rev).splitlines():
        try:
            text = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True).stdout
        except OSError:
            continue
        if b"\0" in text[:8000]:
            continue
        for number, line in enumerate(text.decode("utf-8", "replace").splitlines(), 1):
            yield path, number, line


def public_hits(path, line):
    for rule, pattern in (("token", TOKENS), ("App Store Connect id", ASC), ("device UDID", UDID)):
        if pattern.search(line):
            yield rule
    if KEY_LINE.match(line) and not HEX_LINE.match(line) and not KEY_FIXTURES.match(path):
        yield "private-key material"
    for m in MAC_HOME.finditer(line):
        if m.group(1) not in MAC_HOMES:
            yield f"a person's home (/Users/{m.group(1)})"
    for m in LINUX_HOME.finditer(line):
        if m.group(1) not in LINUX_HOMES:
            yield f"a person's home (/home/{m.group(1)})"
    for m in IPV4.finditer(line):
        try:
            ip = ipaddress.ip_address(m.group(0))
        except ValueError:
            continue
        if ip in TAILNET and ip not in TAILNET_FIXTURES:
            yield f"a tailnet address ({ip}; fixtures use 100.64.0.0/24)"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--head")
    parser.add_argument("--base")
    parser.add_argument("--tree")
    parser.add_argument("--public", action="store_true", help="the public rules only")
    args = parser.parse_args()
    if not (args.head or args.tree) or (args.head and args.tree):
        parser.print_usage(sys.stderr)
        return 64
    root = git("rev-parse", "--show-toplevel").strip()

    lists = None if args.public else private_lists(root)
    if not args.public and lists is None:
        print("oss-scan: the private lists aren't here (MOSHPIT_OPS, or ../moshpit-ops: "
              "app/oss/denylist.txt, agent-names.txt). A team member checks out the ops "
              "repository there; anyone else runs with --public.", file=sys.stderr)
        return 2
    deny, names = lists if lists else ([], [])
    team = None if args.public else team_id(root)
    if team:
        deny = deny + [team]
    name_pattern = (re.compile(r"(?<![\w:/-])(?:" + "|".join(map(re.escape, names)) + r")(?![\w-])")
                    if names else None)

    leaks = []
    if args.tree:
        lines = tree_lines(args.tree)
        messages = []
    else:
        base = args.base
        if not base or base == ZERO:
            remote_main = git("rev-parse", "--verify", "-q", "refs/remotes/origin/main", check=False).strip()
            base = (git("merge-base", args.head, remote_main, check=False).strip() if remote_main else "")
            base = base or git("hash-object", "-t", "tree", "/dev/null").strip()
        lines = added_lines(base, args.head)
        commits = git("rev-list", f"{base}..{args.head}", check=False).split() if len(base) == 40 else \
            git("rev-list", args.head).split()
        messages = [(c, git("log", "-1", "--format=%B", c)) for c in commits]

    for path, number, line in lines:
        if path == SELF:
            continue
        for rule in public_hits(path, line):
            leaks.append(f"{path}:{number}: {rule}: {line.strip()[:120]}")
        for word in deny:
            if word in line:
                leaks.append(f"{path}:{number}: the private denylist (a line of it)")
    for commit, message in messages:
        for word in deny:
            if word in message:
                leaks.append(f"commit {commit[:10]}'s message: the private denylist (a line of it)")
        if name_pattern and name_pattern.search(message):
            leaks.append(f"commit {commit[:10]}'s message: a team agent's name "
                         f"({name_pattern.search(message).group(0)}): say what changed, not who")
        for rule in public_hits("", message):
            if rule != "private-key material":
                leaks.append(f"commit {commit[:10]}'s message: {rule}")

    if leaks:
        print(f"LEAK: {len(leaks)} finding(s); nothing may go public until they are gone:", file=sys.stderr)
        for leak in leaks[:60]:
            print("  " + leak, file=sys.stderr)
        return 2
    print(f"oss-scan: clean ({'public rules' if args.public else 'public and private rules'})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
