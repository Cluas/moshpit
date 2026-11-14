#!/usr/bin/env python3
"""Shell matrix: every command line Moshpit hands a login shell, parsed by
sh, dash, bash, zsh and fish 3 and 4 alike (B138).

    harness/shells.py [--keep] [--no-build]

sshd runs an exec request as `<login shell> -c <command>`, and a line typed
into a terminal is parsed by that same shell. Moshpit's commands are POSIX
sh; fish reads single quotes differently (`\\'` and `\\\\` are escapes in
them, so a value ending in `\\` never closes its quote) and refuses `$?`,
`$$` and `while … do … done` outright. So every exec request goes out as
`sh -c <payload>` (moshpit_ssh::shell::portable, `Shell.portable` in the
app), every quoted word keeps `'` and `\\` outside its quotes
(`shell::quote`, `Shell.quote`), and the lines typed into a shell use only
what all of them read alike.

The cases come from the Rust crates (examples/shell_cases.rs), which build
the lines exactly as the app does — the herdr, install and tmux harnesses
hold the app's Swift to the same bytes. They run in two throwaway alpine
containers (3.20: fish 3.7; latest: fish 4) with busybox sh, dash, bash,
zsh and fish; nothing is installed on this machine, and the containers are
removed afterwards unless --keep.

Under each shell a case runs as `<shell> -c <line>` with PATH starting at a
directory of recorders standing in for tmux, herdr, python3, openssl, curl
and stty: each writes its argv to a log and answers as the program would.
A case passes when every shell leaves the record `sh` leaves running the
payload itself (exec cases), the line itself (typed lines), or prints the
value back (quoted words). The old forms run too, for the record of what
fish did with them.
"""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
IMAGES = {"fish3": "alpine:3.20", "fish4": "alpine:latest"}
PACKAGES = "fish dash bash zsh python3"
NICE = ["nice", "-n", "10"]

RUNNER = r'''
import json, os, shutil, subprocess, sys, tempfile

SHELLS = [s for s in ["/bin/sh", "/usr/bin/dash", "/bin/bash", "/bin/zsh", "/usr/bin/fish"]
          if os.path.exists(s)]
STUBS = ["tmux", "herdr", "python3", "openssl", "curl", "stty", "mosh-server"]
RECORDER = """#!/bin/sh
# Records who was called with what; answers like the real program.
{ printf 'CALL %s' "${0##*/}"; for a in "$@"; do printf ' <%s>' "$a"; done; printf '\\n'; } >> "$MOSHPIT_LOG"
case "${0##*/}" in
  tmux) [ "$1" = attach ] && printf 'PID %s\\n' "$$" >> "$MOSHPIT_LOG" ;;
  herdr) printf '{"ok":true}\\n' ;;
  python3) [ "$1" = -c ] && printf 'PROGRAM %s\\n' "$2" >> "$MOSHPIT_LOG" ;;
esac
exit 0
"""

def run(shell, line, home, stubs):
    log = os.path.join(home, "calls.log")
    open(log, "w").close()
    env = {"PATH": stubs + ":/usr/local/bin:/usr/bin:/bin", "HOME": home, "MOSHPIT_LOG": log,
           "LC_ALL": "C.UTF-8", "TERM": "dumb", "SHELL": "/usr/local/bin/the-login-shell"}
    argv = [shell, "-c", line] if shell else ["/bin/sh", "-c", line]
    try:
        done = subprocess.run(argv, env=env, cwd=home, stdin=subprocess.DEVNULL,
                              capture_output=True, timeout=20)
        out, status = done.stdout, done.returncode
    except subprocess.TimeoutExpired:
        out, status = b"", "timeout"
    # A command that backgrounds a program (`nohup herdr server … &`) is
    # done before the program is: let the log settle.
    import time
    last, steady = -1, 0
    for _ in range(40):
        size = os.path.getsize(log)
        steady = steady + 1 if size == last else 0
        if steady >= 3:
            break
        last = size
        time.sleep(0.05)
    calls = open(log, "rb").read().decode("utf-8", "replace")
    # The renderer line records its shell's pid: the pidfile must name the
    # tmux client that exec'd from it.
    pids = [l[4:] for l in calls.split("\n") if l.startswith("PID ")]
    calls = "\n".join(l for l in calls.split("\n") if not l.startswith("PID "))
    pidfiles = []
    moshpit = os.path.join(home, ".moshpit")
    if os.path.isdir(moshpit):
        for name in sorted(os.listdir(moshpit)):
            if name.endswith(".pid"):
                text = open(os.path.join(moshpit, name)).read().strip()
                pidfiles.append("pidfile names the client" if pids and text == pids[-1]
                                else "pidfile " + ("holds a pid" if text.isdigit() else repr(text)))
    return {"status": status, "out": out.decode("utf-8", "replace"), "calls": calls,
            "pidfiles": pidfiles}

def main():
    cases = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
    results = []
    base = tempfile.mkdtemp(prefix="shells-")
    stubs = os.path.join(base, "stubs")
    os.makedirs(stubs)
    for name in STUBS:
        path = os.path.join(stubs, name)
        open(path, "w").write(RECORDER)
        os.chmod(path, 0o755)
    # One home for every run, emptied in between: its path is in what the
    # commands print.
    home = os.path.join(base, "home")
    for index, case in enumerate(cases):
        for shell in [None] + SHELLS:
            shutil.rmtree(home, ignore_errors=True)
            os.makedirs(home)
            for rel, text in case.get("files", {}).items():
                path = os.path.join(home, rel)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                open(path, "w").write(text)
            line = case["reference"] if shell is None else case["line"]
            result = run(shell, line, home, stubs)
            result.update(case=index, shell=shell or "reference")
            results.append(result)
    json.dump(results, open(sys.argv[2], "w"))

main()
'''


def sh(argv, **kwargs):
    return subprocess.run(argv, **kwargs)


def build_cases():
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    done = sh(NICE + ["cargo", "run", "--quiet", "--release", "-p", "moshpit-ssh", "--example", "shell_cases"],
              cwd=REPO, env=env, capture_output=True, text=True, check=True)
    return [json.loads(line) for line in done.stdout.split("\n") if line.strip()]


def container(tag, image):
    name = f"moshpit-r21-{tag}"
    exists = sh(["docker", "ps", "-a", "--format", "{{.Names}}"], capture_output=True, text=True).stdout.split()
    if name not in exists:
        sh(["docker", "run", "-d", "--name", name, image, "sleep", "infinity"], check=True,
           stdout=subprocess.DEVNULL)
        sh(["docker", "exec", name, "apk", "add", "--no-cache"] + PACKAGES.split(), check=True,
           stdout=subprocess.DEVNULL)
    version = sh(["docker", "exec", name, "fish", "--version"], capture_output=True, text=True).stdout.strip()
    return name, version


def run_in(name, cases, tmp):
    cases_path = os.path.join(tmp, "cases.jsonl")
    runner_path = os.path.join(tmp, "runner.py")
    with open(cases_path, "w") as f:
        f.write("\n".join(json.dumps(c) for c in cases) + "\n")
    with open(runner_path, "w") as f:
        f.write(RUNNER)
    sh(["docker", "exec", name, "mkdir", "-p", "/matrix"], check=True)
    for path in (cases_path, runner_path):
        sh(["docker", "cp", path, f"{name}:/matrix/"], check=True, stdout=subprocess.DEVNULL)
    sh(["docker", "exec", name, "python3", "-I", "/matrix/runner.py", "/matrix/cases.jsonl",
        "/matrix/results.json"], check=True)
    out = os.path.join(tmp, f"{name}.json")
    sh(["docker", "cp", f"{name}:/matrix/results.json", out], check=True, stdout=subprocess.DEVNULL)
    return json.load(open(out))


def shell_name(path, version):
    base = os.path.basename(path)
    if base == "fish":
        return version.replace("fish, version ", "fish ")
    return {"sh": "sh (busybox)"}.get(base, base)


def main():
    keep = "--keep" in sys.argv
    cases = build_cases()
    names = []
    try:
        table, failures, before = {}, [], {}
        with tempfile.TemporaryDirectory(prefix="shell-matrix-") as tmp:
            for tag, image in IMAGES.items():
                name, version = container(tag, image)
                names.append(name)
                results = run_in(name, cases, tmp)
                reference = {r["case"]: r for r in results if r["shell"] == "reference"}
                for r in results:
                    if r["shell"] == "reference":
                        continue
                    case = cases[r["case"]]
                    want = reference[r["case"]]
                    same = all(r[k] == want[k] for k in ("status", "out", "calls", "pidfiles"))
                    if case.get("value") is not None:
                        same = same and r["out"] == case["value"] and r["status"] == 0
                    shell = shell_name(r["shell"], version)
                    key = (case["group"], shell)
                    entry = table.setdefault(key, [0, 0])
                    entry[0] += 1
                    entry[1] += 1 if same else 0
                    if not same:
                        if case.get("old"):
                            before.setdefault(shell, []).append(case["name"])
                        else:
                            failures.append((shell, f"{case['group']}: {case['name']}", want, r))
        groups = sorted({g for g, _ in table})
        shells = sorted({s for _, s in table}, key=lambda s: (s.startswith("fish"), s))
        width = max(len(g) for g in groups) + 2
        print(" " * width + "".join(f"{s:>16}" for s in shells))
        for g in groups:
            print(f"{g:{width}}" + "".join(
                f"{'%d/%d' % (table[(g, s)][1], table[(g, s)][0]) if (g, s) in table else '-':>16}"
                for s in shells))
        for shell, cases_ in sorted(before.items()):
            print(f"old forms that {shell} read differently from sh: {len(cases_)} — {', '.join(cases_[:6])}")
        for shell, name, want, got in failures[:20]:
            print(f"FAIL {shell} {name}\n  want {json.dumps(want)[:400]}\n  got  {json.dumps(got)[:400]}")
        print(f"{len(failures)} failure(s)")
        return 1 if failures else 0
    finally:
        if not keep:
            for name in names:
                sh(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    sys.exit(main())
