#!/usr/bin/env python3
"""Seeded random scripts for the controller harness.

    fuzz.py <dir> <seed> <count>

Each script builds a random tmux world (1-3 sessions of 1-3 windows of 1-3
panes, layouts at the client's width or another's, alternate screens,
copy-mode, agent stamps, another device's size lease, mouse apps) and runs
a random sequence of notifications, intents, view reports, replies and time
through both controllers. It stays off the paths a documented divergence
changes (error replies, the sidecar's window following, a second control
session), so every script must come out identical but for the dead-pane
normalisation.
"""
import os
import random
import sys


def layout(panes, width, height, ids):
    if len(panes) == 1:
        return f"b25d,{width}x{height},0,0,{ids[0]}"
    part = max(1, width // len(panes) - 1)
    leaves = ",".join(f"{part}x{height},{i * (part + 1)},0,{pid}" for i, pid in enumerate(ids))
    return f"c0de,{width}x{height},0,0{{{leaves}}}"


class World:
    def __init__(self, rng, cols, rows):
        self.rng = rng
        self.sessions, self.windows, self.panes = [], [], []
        w = p = 0
        for s in range(rng.randint(1, 3)):
            sid = f"${s}"
            self.sessions.append([sid, rng.choice(["main", "dev", "a|b", "my proj", "été"]) + str(s)])
            nwin = rng.randint(1, 3)
            active_w = rng.randrange(nwin)
            for k in range(nwin):
                wid = f"@{w}"
                w += 1
                npanes = rng.randint(1, 3)
                ids = list(range(p, p + npanes))
                p += npanes
                width = cols if rng.random() < 0.7 else rng.choice([200, 499, 81])
                self.windows.append({"id": wid, "s": sid, "idx": k, "active": k == active_w,
                                     "panes": ids, "w": width, "h": rows,
                                     "name": rng.choice(["zsh", "vim", "claude", ""])})
                active_p = rng.randrange(npanes)
                for j, pid in enumerate(ids):
                    self.panes.append({"id": f"%{pid}", "w": wid, "idx": j, "active": j == active_p,
                                       "alt": rng.random() < 0.25, "copy": rng.random() < 0.1,
                                       "cmd": rng.choice(["bash", "vim", "2.1.269", "node", ""])})

    def rules(self):
        attached = {self.sessions[0][0]}
        sess = "\\n".join(f"{sid} {1 if sid in attached else 0} {name}" for sid, name in self.sessions)
        wins = "\\n".join(
            f"{x['s']} {x['id']} {x['idx']} {layout(x['panes'], x['w'], x['h'], x['panes'])} {1 if x['active'] else 0} {len(x['panes'])} {x['name']}"
            for x in self.windows)
        panes = "\\n".join(
            f"{x['id']} {x['w']} {x['idx']} 80 24 {1 if x['active'] else 0} {1 if x['alt'] else 0} {1 if x['copy'] else 0} {x['cmd']}"
            for x in self.panes)
        hooks = "\\n".join(f"{x['id']}|{self.rng.choice(['working', 'done', '', 'x'])}|claude|1758000000|{x['cmd']}|t"
                           for x in self.panes) or "-"
        out = [f"rule ok list-sessions => {sess}", f"rule ok list-windows => {wins or '-'}",
               f"rule ok list-panes -a -F '#{{pane_id}}| => {hooks}",
               f"rule ok list-panes -a -F '#{{pane_id}}\\s => {panes or '-'}"]
        for x in self.windows:
            out.append(f"rule ok display-message -p -t '{x['id']}' => {len(x['panes'])} 0 %{x['panes'][0]} "
                       f"{layout(x['panes'], x['w'], x['h'], x['panes'])}")
        return out

    def window_of(self, pane):
        return next((x for x in self.windows if x["id"] == pane["w"]), None)


def text(rng):
    # Octal escapes are tmux's own (%output spells control bytes \\NNN).
    pieces = ["hello", "\\\\015\\\\012", "\\\\007", "\\\\033]0;title\\\\007", "中文", "\\\\033[31mred\\\\033[0m",
              "\\s", "x" * rng.randint(1, 30)]
    return "".join(rng.choice(pieces) for _ in range(rng.randint(1, 5)))


def script(rng):
    renders = rng.random() < 0.85
    cols, rows = rng.randint(40, 100), rng.randint(20, 60)
    world = World(rng, cols, rows)
    steps = [f"config renders={1 if renders else 0} label=iPhone quiet={rng.choice([0.2, 2, 30])} "
             f"stamp={rng.choice([0.3, 5])} probe={rng.choice([0.2, 1.5])} streaming={rng.choice([0.5, 60])}",
             f"init-size {cols} {rows}"]
    if rng.random() < 0.7:
        steps.append("client-name moshpit-a")
    holder = rng.choice([None, "1757999998", "1757999900"])
    clients = "moshpit-a\\nmoshpit-b" if holder and rng.random() < 0.7 else "moshpit-a"
    steps += [f"rule ok list-clients -F '#{{client_name}}' => {clients}",
              f"rule ok show-options -wqv => {('moshpit-b|iPad|' + holder) if holder else '-'}",
              "rule ok display-message -p '#{client_name}' => moshpit-a",
              f"rule ok display-message -p -t %{rng.randrange(9)} '#{{mouse_any_flag}} => {rng.randint(0, 1)} {rng.randint(0, 1)}",
              "rule ok list-clients -F '#{client_control_mode} => 1|45613|$0|/dev/ttys030\\n0|45533|$0|/dev/ttys026",
              "rule ok show-options -gqv prefix => C-a",
              f"rule ok capture-pane -p -e -S => {rng.choice(['history 1\\nhistory 2', 'W' * 120, '-'])}",
              f"rule ok capture-pane -p -e\\s\\s-t => ALT",
              f"rule ok capture-pane -p -e -t => {rng.choice(['$ ls\\nREADY', 'frame\\n' * 70, 'W' * 150, 'ok'])}",
              f"rule ok display-message -p -t %{rng.randrange(9)} '#{{cursor_x}} => {rng.randint(0, 9)} {rng.randint(0, 9)}",
              *world.rules()]
    if rng.random() < 0.5:
        steps += ["attach", "recv %begin 100 0 0\\n%end 100 0 0\\n\\n"]
    else:
        steps += ["begin", f"recv %begin 100 0 0\\n%end 100 0 0\\n\\n%session-changed {world.sessions[0][0]} x\\n"]
    steps.append("settle")
    for _ in range(rng.randint(10, 40)):
        pane = rng.choice(world.panes) if world.panes else None
        win = rng.choice(world.windows) if world.windows else None
        kind = rng.random()
        if kind < 0.12 and pane:
            steps.append(f"recv %output {pane['id']} {text(rng)}\\n")
        elif kind < 0.2 and win:
            width = rng.choice([cols, cols, 200])
            flag = rng.choice(["", " *", " *Z", " -Z"])
            lay = layout(win["panes"], width, rows, win["panes"])
            steps.append(f"recv %layout-change {win['id']} {lay} {lay}{flag}\\n")
        elif kind < 0.25:
            choice = rng.random()
            if choice < 0.2 and win:
                steps.append(f"recv %window-add {win['id']}\\n")
            elif choice < 0.4 and win and renders:
                steps.append(f"recv %session-window-changed {win['s']} {win['id']}\\n")
            elif choice < 0.6 and pane:
                steps.append(f"recv %window-pane-changed {pane['w']} {pane['id']}\\n")
            elif choice < 0.7 and pane:
                steps.append(f"recv %pause {pane['id']}\\n%continue {pane['id']}\\n")
            elif choice < 0.8:
                steps.append("recv %sessions-changed\\n")
            elif win:
                steps.append(f"recv %window-renamed {win['id']} r{rng.randrange(9)}\\n")
        elif kind < 0.33:
            if renders and rng.random() < 0.5 and win:
                steps.append(f"select-window {win['id']}")
            elif renders and pane:
                steps.append(f"select-pane {pane['id']}")
            else:
                steps.append(f"select-session {rng.choice(world.sessions)[0]}")
        elif kind < 0.38:
            steps.append(f"resize {rng.randint(40, 100)} {rng.randint(20, 60)}")
        elif kind < 0.46 and pane:
            data = rng.choice(["1b5b41", "1b4f42", "61", "0d", "1b", "1b5b313b3543", "e4b8ad", "03"])
            steps.append(f"input {pane['id']} {data}")
        elif kind < 0.5 and pane:
            steps.append(f"paste {pane['id']} {rng.choice(['two\\nlines', '$HOME ~x', 'a\\e[201~b', 'q'])}")
        elif kind < 0.58:
            steps.append(rng.choice(["scroll 3", "scroll -2", "scroll-px 20 16", "scroll-px -7.5 16",
                                     "scroll-end", "scroll-begin", "click 3 4", "mouse"]))
        elif kind < 0.64 and renders:
            steps.append(rng.choice(["release", "repin", "take", "resync 0", "resync 1"]))
        elif kind < 0.68:
            steps.append(rng.choice(["refresh", "poll", "count", "prefix", "cancel-copy"]))
            if pane and rng.random() < 0.3:
                steps.append(f"path {pane['id']}")
        elif kind < 0.72 and pane and renders:
            steps.append(rng.choice([f"view-grid {pane['id']} {cols} {rows}", f"view-defer {pane['id']} {cols - 1} {rows}",
                                     f"view-mode {pane['id']}", f"view-return {pane['id']}", f"view-bell {pane['id']}",
                                     f"view-input {pane['id']} 6a"]))
        elif kind < 0.75 and win:
            steps.append(rng.choice([f"rename-window {win['id']} n{rng.randrange(9)}", "new-pane",
                                     f"rename-session {rng.choice(world.sessions)[0]} s\\s1"]))
        elif kind < 0.78 and len(world.panes) > 1 and pane:
            # A pane exits: tmux drops it from its window's layout.
            owner = world.window_of(pane)
            if owner and len(owner["panes"]) > 1:
                owner["panes"].remove(int(pane["id"][1:]))
                world.panes.remove(pane)
                if pane["active"]:
                    nxt = next(x for x in world.panes if x["w"] == owner["id"])
                    nxt["active"] = True
                steps.append("rules-clear")
                steps += [s for s in script_rules_keep(steps)]
                steps += world.rules()
                lay = layout(owner["panes"], owner["w"], rows, owner["panes"])
                steps.append(f"recv %layout-change {owner['id']} {lay} {lay}\\n")
        elif kind < 0.9:
            steps.append(f"wait {rng.choice([50, 180, 300, 450, 700, 1000, 2100, 3100, 4000])}")
        else:
            steps.append(rng.choice(["respond", "settle", "settle"]))
    steps += ["settle", "wait 3000", "settle", "dump"]
    return steps


def script_rules_keep(steps):
    """The rules that don't describe the tree, re-added after a rules-clear."""
    keep = []
    for step in steps:
        if step.startswith("rule ") and not any(k in step for k in ("list-sessions", "list-windows", "list-panes",
                                                                    "display-message -p -t '@")):
            keep.append(step)
    seen, out = set(), []
    for step in keep:
        if step not in seen:
            seen.add(step)
            out.append(step)
    return out


def main():
    out, seed, count = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    os.makedirs(out, exist_ok=True)
    for f in os.listdir(out):
        if f.endswith(".scn"):
            os.remove(os.path.join(out, f))
    for n in range(count):
        rng = random.Random(seed * 1_000_003 + n)
        with open(os.path.join(out, f"fuzz-{seed}-{n:04d}.scn"), "w") as f:
            f.write("\n".join(script(rng)) + "\n")


main()
