#!/usr/bin/env python3
"""Client equivalence runs: the app's HerdrControlClient.swift against this
crate's client, on the same fake host, scenario by scenario.

    equivalence.py [--no-build] [<scenario>...]   (default: every scenarios/*.txt)
    equivalence.py update-goldens [--no-build]

`update-goldens` records both logs next to each scenario (<name>.rust.log,
<name>.swift.log): tests/client_scenarios.rs replays every scenario and
checks the Rust log without swiftc, and that each documented divergence
still separates it from the recorded Swift one.

Both sides print a log (src/client/scenario.rs): the commands they run, the
focus they report, the outcomes they answer and when, then the mutations the
host saw land in order and the final state. Each scenario is reported as

  identical      the two logs are the same, line for line;
  same effects   the same mutations landed in the same order, the same
                 outcomes (the Rust one at most one read's delay earlier: the
                 app awaits a re-read before it returns), the same commands
                 but reads (`api snapshot`, `herdr status`), the same panes
                 reported focused in the same order, the same final state:
                 only the reads' schedule differs (the Rust client keeps one
                 read out at a time and restarts the cadence after each; the
                 app's poll loop runs beside the extra reads);
  differs        anything else, with the differences.

divergences.json says which differences are the app's bugs (with evidence)
and which are on purpose; a scenario that differs without an entry fails.
Builds niced: the Swift side (build-swift.sh), then the Rust examples.
"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(os.path.dirname(HERE))
REPO = os.path.dirname(os.path.dirname(CRATE))
SWIFT = os.path.join(REPO, "target", "w2b-harness", "client-swift")
RUST = os.path.join(REPO, "target", "debug", "examples", "client_harness")
HOST = os.path.join(REPO, "target", "debug", "examples", "herdr_host")


def build():
    subprocess.run(["nice", "-n", "10", os.path.join(HERE, "build-swift.sh"), SWIFT],
                   check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    subprocess.run(["nice", "-n", "10", "cargo", "build", "--quiet", "-p", "moshpit-herdr",
                    "--example", "client_harness", "--example", "herdr_host"],
                   check=True, cwd=REPO, env=env)


# An outcome may come this much earlier on the Rust side: the app awaits a
# re-read before it returns one (one exec's delay on the fake host).
TOLERANCE_MS = 100


def parse(log):
    parts = {"execs": [], "reads": [], "focus": [], "outcomes": [], "landed": [], "state": [],
             "times": []}
    for line in log.splitlines():
        if line.startswith("landed "):
            parts["landed"].append(line)
        elif line.startswith("state "):
            parts["state"].append(line)
        elif line.startswith("t="):
            at, _, rest = line.partition(" ")
            if rest in ("exec api snapshot", "exec status"):
                parts["reads"].append(f"{at} {rest}")
            elif rest.startswith("exec "):
                parts["execs"].append(f"{at} {rest}")
            elif rest.startswith("focus "):
                # Reported after every read: the sequence of panes counts,
                # not when each read happened to land.
                if not parts["focus"] or parts["focus"][-1] != rest:
                    parts["focus"].append(rest)
            elif rest.startswith("outcome "):
                parts["outcomes"].append(rest)
                parts["times"].append(int(at[2:]))
    return parts


def same_times(rust, swift):
    return len(rust) == len(swift) and all(0 <= s - r <= TOLERANCE_MS for r, s in zip(rust, swift))


def run(path):
    rust = subprocess.run([RUST, path], capture_output=True, text=True, timeout=120)
    swift = subprocess.run([SWIFT, HOST, path], capture_output=True, text=True, timeout=600)
    return rust.stdout, swift.stdout


def short(line, width=150):
    line = str(line)
    return line if len(line) <= width else line[: width - 1] + "…"


def main():
    args = [arg for arg in sys.argv[1:] if not arg.startswith("--")]
    update = bool(args) and args[0] == "update-goldens"
    if update:
        args = args[1:]
    if "--no-build" not in sys.argv:
        build()
    with open(os.path.join(HERE, "divergences.json"), encoding="utf-8") as f:
        documented = json.load(f)
    explained = {}
    for entry in documented.get("divergences", []):
        for scenario in entry.get("scenarios", []):
            explained.setdefault(scenario, []).append(entry["id"])
    names = args or sorted(name[:-4] for name in os.listdir(os.path.join(HERE, "scenarios"))
                           if name.endswith(".txt"))
    unexplained = 0
    counts = {"identical": 0, "same effects": 0, "differs": 0}
    for name in names:
        path = os.path.join(HERE, "scenarios", name + ".txt")
        rust_log, swift_log = run(path)
        if update:
            for side, log in (("rust", rust_log), ("swift", swift_log)):
                with open(os.path.join(HERE, "scenarios", f"{name}.{side}.log"), "w", encoding="utf-8") as f:
                    f.write(log if log.endswith("\n") else log + "\n")
        rust, swift = parse(rust_log), parse(swift_log)
        if rust_log == swift_log:
            verdict = "identical"
        elif all(rust[key] == swift[key] for key in ("execs", "focus", "outcomes", "landed", "state")) \
                and same_times(rust["times"], swift["times"]):
            verdict = "same effects"
        else:
            verdict = "differs"
        counts[verdict] += 1
        ids = explained.get(name, [])
        note = f" [{', '.join(ids)}]" if ids else ""
        print(f"{name}: {verdict}{note}  (reads: rust {len(rust['reads'])}, swift {len(swift['reads'])})")
        if verdict == "differs":
            if not ids:
                unexplained += 1
            for key in ("outcomes", "times", "landed", "state", "execs", "focus"):
                if rust[key] == swift[key] or (key == "times" and same_times(rust[key], swift[key])):
                    continue
                print(f"  {key}:")
                print("    rust : " + (" | ".join(short(x, 110) for x in rust[key][:8]) or "(none)"))
                print("    swift: " + (" | ".join(short(x, 110) for x in swift[key][:8]) or "(none)"))
    print(f"{len(names)} scenarios: {counts['identical']} identical, {counts['same effects']} same effects, "
          f"{counts['differs']} differ ({unexplained} without a documented divergence)")
    return 1 if unexplained else 0


if __name__ == "__main__":
    sys.exit(main())
