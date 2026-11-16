#!/usr/bin/env python3
"""Write the hand-made controller scenarios (tests/fixtures/controller/*.scn).

    scenarios.py [<dir>]

Each scenario drives one behaviour of the Swift test suites through a
script both controllers run (see crates/tmux/src/scenario.rs). They carry no
assertions: the harness compares the two logs, and tests/controller_logs.rs
holds the Rust log to the committed golden.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DIR = os.path.join(os.path.dirname(os.path.dirname(HERE)), "tests", "fixtures", "controller")

BOOT = "recv %begin 100 0 0\\n%end 100 0 0\\n\\n"


def world(sessions, windows, panes, hooks=None):
    """Rules answering the listings from a fixed tree."""
    rules = [
        "rule ok list-sessions => " + "\\n".join(sessions),
        "rule ok list-windows => " + "\\n".join(windows),
    ]
    if hooks is not None:
        rules.append("rule ok list-panes -a -F '#{pane_id}| => " + ("\\n".join(hooks) or "-"))
    rules.append("rule ok list-panes -a -F '#{pane_id}\\s => " + "\\n".join(panes))
    return rules


ONE = world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 main"], ["%0 @0 0 80 24 1 0 0 bash"],
            hooks=["%0|working|claude|1758000000|claude|fixing"])
TWO_WINDOWS = world(["$0 1 main"],
                    ["$0 @0 0 b25d,80x24,0,0,0 1 1 main", "$0 @1 1 b25e,80x24,0,0,1 0 1 work"],
                    ["%0 @0 0 80 24 1 0 0 bash", "%1 @1 0 80 24 1 0 0 vim"], hooks=[])
TWO_SESSIONS = world(["$0 1 main", "$1 0 dev"],
                     ["$0 @0 0 b25d,80x24,0,0,0 1 1 main", "$1 @1 0 b25e,80x24,0,0,1 1 1 work"],
                     ["%0 @0 0 80 24 1 0 0 bash", "%1 @1 0 80 24 1 0 0 vim"], hooks=[])
SPLIT = "4b9d,203x60,0,0[203x30,0,0,0,203x29,0,31{101x29,0,31,1,101x29,102,31,2}]"
THREE_PANES = world(["$0 1 main"], [f"$0 @0 0 {SPLIT} 1 3 main"],
                    ["%0 @0 0 203 30 1 0 0 bash", "%1 @0 1 101 29 0 0 0 bash", "%2 @0 2 101 29 0 0 0 bash"],
                    hooks=[])
CAPTURES = [
    "rule ok capture-pane -p -e -S => history 1\\nhistory 2\\n\\s\\s",
    "rule ok capture-pane -p -e\\s\\s-t => ALT SCREEN",
    "rule ok capture-pane -p -e -t => $ ls\\nREADY\\n",
    "rule ok display-message -p -t %0 '#{cursor_x} => 2 1",
    "rule ok display-message -p -t %1 '#{cursor_x} => 0 0",
]


def ssh(*rules, size=(70, 35), client="moshpit-a"):
    steps = ["config label=iPhone quiet=30", f"init-size {size[0]} {size[1]}"]
    if client:
        steps.append(f"client-name {client}")
    steps += ["attach", BOOT, *rules]
    return steps


S = {}

S["attach-basic"] = ssh(*ONE, *CAPTURES) + ["settle", "dump", "wait 800", "settle", "dump",
                                            "recv %output %0 hello\\r\\n", "input %0 1b5b41",
                                            "input %0 616263", "paste %0 two\\nlines", "dump"]

S["attach-session-changed"] = [
    "config label=iPhone", "init-size 69 60", "begin",
    "recv %begin 100 0 0\\n%end 100 0 0\\n\\n%session-changed $0 main\\n",
    "rule ok display-message -p '#{client_name}' => /dev/ttys003",
    "rule ok list-clients -F '#{client_name}' => /dev/ttys003",
    "rule ok show-options -wqv => -", *ONE, *CAPTURES,
    "settle", "dump", "wait 800", "settle", "poll", "settle", "dump",
]

S["attach-alternate-copy-mode"] = ssh(
    *world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 main"], ["%0 @0 0 80 24 1 1 1 vim"], hooks=[]),
    *CAPTURES) + ["settle", "wait 800", "settle", "dump"]

S["late-boot-banner"] = ["attach", BOOT, *ONE, "settle", "dump"]

S["resize-storm"] = ssh(*ONE, *CAPTURES) + [
    "settle", "resize 90 40", "resize 80 37", "resize 70 35", "wait 100", "resize 70 35",
    "wait 200", "settle", "wait 700", "settle", "resize 70 35", "wait 400", "settle", "dump"]

S["first-report-equals-guess"] = ssh(*ONE, *CAPTURES, size=(69, 60)) + [
    "settle", "resize 69 60", "wait 200", "settle", "resize 69 60", "wait 400", "settle"]

S["frame-rejected-retried"] = ssh(*ONE, *CAPTURES, size=(69, 60)) + [
    "settle", "rules-clear", *ONE,
    "rule ok capture-pane -p -e -t => " + "\\n".join(f"flap-row-{i}" for i in range(70)),
    "rule ok display-message => 0 0",
    "recv %layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\\n",
    "recv %layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\\n",
    "settle", "wait 350", "settle", "rules-clear", *ONE,
    "rule ok capture-pane -p -e -t => REPAIRED-AFTER-RETRY", "rule ok display-message => 0 0",
    "waitsettle 3000 100", "dump"]

S["frame-foreign-width-lines"] = ssh(*ONE, *CAPTURES, size=(69, 60)) + [
    "settle", "rules-clear", *ONE,
    "rule ok capture-pane -p -e -t => " + "\\n".join("W" * 200 for _ in range(30)),
    "rule ok display-message => 0 0", "resync 1", "settle", "wait 350", "settle"]

S["backfill-foreign-width-recaptured"] = ssh(
    *ONE, "rule ok capture-pane -p -e -S => " + "\\n".join("W" * 80 for _ in range(12)),
    "rule ok display-message => 0 0", size=(69, 60)) + [
    "settle", "rules-clear", *ONE,
    "rule ok capture-pane -p -e -S => " + "\\n".join(f"history {i}" for i in range(12)),
    "rule ok capture-pane -p -e -t => LIVE", "rule ok display-message => 3 0",
    "wait 800", "settle", "wait 800", "settle", "dump"]

S["resync-coalesce"] = ssh(*ONE, *CAPTURES) + [
    "settle", "resync 1", "resync 1", "resync 0", "resync 1", "resync 1", "settle",
    "resync 0", "respond", "resync 1", "settle"]

S["pause-continue"] = ssh(*ONE, *CAPTURES) + ["settle", "recv %pause %0\\n", "settle",
                                              "recv %continue %0\\n", "settle"]

S["glued-recovery"] = ssh(*ONE, *CAPTURES, size=(69, 60)) + [
    "settle", "recv %output %0 visible%begin 900 2000 1\\n%end 900 2000 1\\n",
    "recv %output %0 quote:%begin 100 5 1\\n", "resync 1",
    "recv %begin 400 400 1\\nROW-A%end 400 400 1\\n", "settle", "recv %output %0 -after\\n"]

S["pane-before-discovery"] = [
    "attach", BOOT, "recv %output %5 streaming\\n", "select-pane %5", "dump",
    *world(["$0 1 0"], ["$0 @4 5 81x24,0,0,4 1 1 ref", "$0 @5 6 81x24,0,0,5 0 1 islander"],
           ["%4 @4 0 80 24 1 0 0 claude", "%5 @5 0 80 24 1 0 0 claude"], hooks=[]),
    "settle", "dump"]

S["restore-into-split-window"] = [
    "restore $0 @1 %4", "attach", BOOT + "%session-changed $0 0\\n",
    *world(["$0 1 0"], ["$0 @1 1 ed9b,499x62,0,0{215x62,0,0,0,283x62,216,0[283x11,216,0,1,283x10,216,12,2,283x9,216,23,3,283x9,216,33,4,283x9,216,43,5,283x9,216,53,6]} 1 7 team"],
           [f"%{i} @1 {i} 80 9 {1 if i == 6 else 0} 0 0 claude" for i in range(7)], hooks=[]),
    "rule ok display-message -p -t '@1' => 7 0 %6 ed9b,499x62,0,0{215x62,0,0,0,283x62,216,0[283x11,216,0,1,283x10,216,12,2,283x9,216,23,3,283x9,216,33,4,283x9,216,43,5,283x9,216,53,6]}",
    "rule ok select-pane -Z -t %4 => - => %window-pane-changed @1 %4\\n",
    "settle", "wait 800", "settle", "dump"]

S["new-window-lands"] = ssh(*ONE, *CAPTURES) + [
    "settle", "new-window logs", "dump",
    "rule ok new-window => @1", "rules-clear", "rule ok new-window => @1",
    *world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 0 1 main", "$0 @1 1 b25e,80x24,0,0,1 1 1 logs"],
           ["%0 @0 0 80 24 1 0 0 bash", "%1 @1 0 80 24 1 0 0 bash"], hooks=[]),
    *CAPTURES, "settle", "dump", "new-window \\s\\t", "rules-clear",
    "rule err new-window => duplicate window: logs",
    *world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 0 1 main", "$0 @1 1 b25e,80x24,0,0,1 1 1 logs"],
           ["%0 @0 0 80 24 1 0 0 bash", "%1 @1 0 80 24 1 0 0 bash"], hooks=[]),
    *CAPTURES, "new-window x", "settle", "dump", "wait 3100", "dump"]

S["new-window-in-session-new-pane"] = ssh(*TWO_SESSIONS, *CAPTURES) + [
    "settle", "new-window-in $1 api", "new-window-in $0", "settle", "new-pane-in @1", "settle",
    "new-pane", "settle", "dump"]

S["window-close-fallback"] = ssh(*TWO_WINDOWS, *CAPTURES) + [
    "settle", "recv %window-close @0\\n", "dump", "recv %session-window-changed $0 @1\\n", "settle",
    "dump"]

S["unlinked-close-lands-where-tmux-did"] = ssh(
    *world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 0 1 main", "$0 @2 1 b25e,80x24,0,0,2 0 1 logs",
                           "$0 @1 2 b25f,80x24,0,0,1 1 1 vim"],
           ["%0 @0 0 80 24 1 0 0 bash", "%2 @2 0 80 24 1 0 0 tail", "%1 @1 0 80 24 1 0 0 vim"], hooks=[]),
    *CAPTURES) + ["settle", "recv %session-window-changed $0 @2\\n%unlinked-window-close @1\\n",
                  "settle", "dump", "recv %unlinked-window-close @0\\n", "dump"]

S["notifications"] = ssh(*TWO_WINDOWS, *CAPTURES) + [
    "settle", "recv %window-renamed @0 edge\\n", "recv %layout-change @99 81x24,0,0,77\\n",
    "recv %window-pane-changed @0 %0\\n", "recv %window-pane-changed @77 %99\\n",
    "recv %session-window-changed $0 @1\\n", "recv %session-window-changed $5 @42\\n",
    "recv %session-renamed $0 x|y\\n", "recv %session-renamed $9 nope\\n",
    "recv %window-add @9\\n", "settle", "recv %client-detached client-7\\n",
    "recv %sessions-changed\\n%unlinked-window-add @7\\n%unlinked-window-renamed @0 r2\\n", "settle",
    "recv %output %bogus x\\n%layout-change bad 1x1,0,0,0\\n%window-close x\\n", "dump",
    "recv %session-changed $42 newone\\n", "settle", "dump"]

S["select-window-switch"] = ssh(*TWO_WINDOWS, *CAPTURES) + [
    "settle", "resize 70 35", "select-window @1", "settle", "wait 800", "settle", "dump",
    "select-window @1", "settle", "select-pane %0", "settle", "switch next", "settle",
    "switch prev", "settle", "dump"]

S["select-session-hop"] = ssh(*TWO_SESSIONS, *CAPTURES) + [
    "settle", "select-pane %1", "settle", "dump", "select-session $0", "settle", "dump"]

S["refused-listings-keep-tree"] = ssh(*ONE, *CAPTURES) + [
    "settle", "poll", "settle", "dump", "rules-clear",
    "rule err list- => server exited unexpectedly", "refresh", "rename-session $0 work2",
    "recv %window-add @0\\n", "poll", "settle", "dump", "wait 400", "dump"]

S["kill-and-rename"] = ssh(*world(["$10 0 ten", "$2 0 two", "$3 1 three"], ["$3 @0 0 b25d,80x24,0,0,0 1 1 main"],
                                  ["%0 @0 0 80 24 1 0 0 bash"], hooks=[]), *CAPTURES) + [
    "settle", "dump", "kill-session $3", "dump", "rule err kill- => can't find", "kill-window @0",
    "kill-pane %0", "rename-window @0 \\s", "rename-window @0 it's", "rename-session $2 my\\sproj",
    "settle", "dump", "run display-message hi", "settle"]

S["hooks-poll"] = ssh(*world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 main"], ["%0 @0 0 80 24 1 0 0 bash"],
                             hooks=["%0|working|claude|1758000000|2.1.269|fix | it", "%1||||zsh|",
                                    "%2|bogus|codex|soon|node|", "|working|x|1|y|", "%3|done"]),
                       *CAPTURES) + ["settle", "poll", "settle", "dump"]

S["questions"] = ssh(*ONE, *CAPTURES,
                     "rule ok display-message -p -t %0 '#{pane_current_path}' => /home/me/proj\\s",
                     "rule ok show-options -gqv prefix => C-a",
                     "rule ok display-message -p -t %0 '#{pane_in_mode}' => 1") + [
    "settle", "path %0", "prefix", "cancel-copy", "count", "settle", "path %0", "wait 4100", "dump"]

S["exit-then-new-session"] = ssh(*ONE, *CAPTURES) + [
    "settle", "count", "recv %exit\\n", "skip", "dump",
    "recv %begin 300 300 0\\n%end 300 300 0\\n%session-changed $7 again\\n",
    "rules-clear", *world(["$7 1 again", "$8 0 other"], ["$7 @3 0 b25d,80x24,0,0,3 1 1 sh"],
                          ["%3 @3 0 80 24 1 0 0 sh"], hooks=[]),
    "rule ok display-message -p '#{client_name}' => client-9", *CAPTURES, "settle", "dump"]

S["detach-answers-waiters"] = ssh(*ONE, *CAPTURES) + ["settle", "count", "detach", "dump"]

S["close-answers-waiters"] = ssh(*ONE, *CAPTURES) + ["settle", "count", "close", "dump"]

S["watchdog-silent"] = ssh() + ["wait 4000", "wait 4000", "wait 4000", "wait 4000", "wait 4000"]

S["watchdog-answering"] = ssh(*ONE, *CAPTURES) + ["settle"] + ["wait 4000", "settle"] * 5

S["first-frame-once"] = ssh(*world(["$0 1 main"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 main"],
                                   ["%0 @0 0 80 24 1 0 0 bash", "%1 @0 1 80 24 0 0 0 vim"], hooks=[])) + [
    "respond", "mint %0", "mint %1", "view-reveal %0", "view-reveal %0", "view-reveal %1", "dump"]

S["view-reports"] = ssh(*ONE, *CAPTURES) + [
    "settle", "view-defer %0 70 30", "view-defer %0 70 30", "wait 200", "settle",
    "view-grid %0 72 30", "wait 200", "settle", "view-size %0 72 31", "wait 200", "settle",
    "live 0", "view-input %0 61", "live 1", "view-input %0 62", "view-mode %0", "settle",
    "view-return %0", "wait 500", "settle", "view-bell %0", "view-defer %0 60 20", "wait 1600",
    "settle", "dump"]

# The size lease (TmuxSizeLeaseTests): the iPad is "moshpit-b".
LEASE_WORLD = world(["$0 1 main"], ["$0 @0 0 200x50,0,0,0 1 1 main"], ["%0 @0 0 200 50 1 0 0 bash"], hooks=[])
CLIENTS_AB = "rule ok list-clients -F '#{client_name}' => moshpit-a\\nmoshpit-b"
CLIENTS_A = "rule ok list-clients -F '#{client_name}' => moshpit-a"
DRIFT = "recv %layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\\n"
# The holder's window changing again: tmux reports a layout only when it
# changes (the same one again is another session's copy of it, B771).
DRIFT_AGAIN = "recv %layout-change @0 c71e,201x50,0,0,0 c71e,201x50,0,0,0 *\\n"


def lease_rig(clients, lease):
    owner = f"rule ok show-options -wqv => {lease}" if lease else "rule ok show-options -wqv => -"
    return ssh(clients, owner, *LEASE_WORLD, "rule ok capture-pane => READY",
               "rule ok display-message => 0 0") + ["settle"]


S["lease-free-window"] = lease_rig(CLIENTS_A, None) + ["dump", "lease-release", "lease-release"]
S["lease-live-holder"] = lease_rig(CLIENTS_AB, "moshpit-b|iPad|1757999998") + [
    "dump", DRIFT, "settle", "recv %output %0 IPAD-SIZED\\n", "dump", "lease-release"]
S["lease-idle-holder"] = lease_rig(CLIENTS_AB, "moshpit-b|iPad|1757999940") + ["dump"]
S["lease-ghost"] = lease_rig(CLIENTS_A, "moshpit-b|iPad|1757999999") + ["dump"]
S["lease-recheck-never-steals"] = lease_rig(CLIENTS_AB, "moshpit-b|iPad|1757999998") + [
    DRIFT, "settle", "config quiet=0.1", DRIFT_AGAIN, "waitsettle 600 30", "dump"]
S["lease-recheck-takes-freed"] = lease_rig(CLIENTS_AB, "moshpit-b|iPad|1757999998") + [
    DRIFT, "settle", "rules-clear", CLIENTS_AB, "rule ok show-options -wqv => -", *LEASE_WORLD,
    "rule ok capture-pane => READY", "rule ok display-message => 0 0", "config quiet=0.1", DRIFT_AGAIN,
    "waitsettle 600 30", "dump"]
S["lease-tap-and-keystroke"] = lease_rig(CLIENTS_AB, "moshpit-b|iPad|1757999998") + [
    DRIFT, "take", "settle", "recv %output %0 MINE-AGAIN\\n", DRIFT_AGAIN, "input %0 78", "settle", "dump"]
S["lease-stamp-throttle"] = lease_rig(CLIENTS_A, None) + [
    "config stamp=0.3", "input %0 61", "input %0 62", "wait 100", "input %0 63", "wait 300",
    "input %0 64", "paste %0 x", "dump"]
S["lease-background-foreground"] = lease_rig(CLIENTS_A, None) + [
    "release", "settle", "recv %output %0 hidden\\n", "recv %output %0 \\\\033]0;t\\\\007\\n",
    "recv %output %0 \\\\007\\n", "repin", "settle", "wait 400", "settle", "dump"]
S["lease-foreground-defers"] = lease_rig(CLIENTS_A, None) + [
    "release", "settle", "rules-clear", CLIENTS_AB, "rule ok show-options -wqv => moshpit-b|iPad|1757999999",
    *LEASE_WORLD, "rule ok capture-pane => READY", "rule ok display-message => 0 0",
    "config probe=30", "repin", "settle", DRIFT, "dump"]
S["lease-foreground-probe-timeout"] = lease_rig(CLIENTS_A, None) + [
    "release", "settle", "config probe=0.1", "repin", "wait 150", "settle", "dump"]

S["war-backoff"] = ssh(*ONE, *CAPTURES) + [
    "settle", "resize 70 35", "wait 200", "settle",
    # The desktop's width and our pin's landing in turn (a war); the third
    # drift trips the backoff, so no pin lands after it.
    "recv %layout-change @0 200x24,0,0,2\\n%layout-change @0 70x35,0,0,2\\n%layout-change @0 200x24,0,0,2\\n%layout-change @0 70x35,0,0,2\\n%layout-change @0 200x24,0,0,2\\n",
    "settle", "wait 5100", "recv %layout-change @0 201x24,0,0,2\\n", "settle"]
S["war-quiet-repin"] = ssh(*ONE, "rule ok capture-pane => QUIET-REPAIRED", "rule ok display-message => 0 0",
                           size=(69, 60)) + [
    "config quiet=0.15", "settle"] + ["recv %layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\\n",
                                      "wait 30",
                                      "recv %layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\\n",
                                      "wait 30"] * 2 + [
    "recv %layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\\n", "wait 30",
    "settle", "waitsettle 400 50", "recv %layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\\n",
    "waitsettle 1500 50", "recv %output %0 live-after-quiet\\n", "dump"]
S["war-foreign-output-paints"] = ssh(*ONE, *CAPTURES, size=(69, 60)) + [
    "settle", "recv %layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\\n",
    "recv %output %0 defghij\\n", "settle", "recv %layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\\n",
    "settle", "wait 800", "settle"]

THREE = "3992,47x40,0,0{45x40,0,0,0,1x40,46,0[1x20,46,0,1,1x19,46,21,2]}"
FOUR = "1c2d,47x40,0,0{45x40,0,0[45x20,0,0,0,45x19,0,21,3],1x40,46,0[1x20,46,0,1,1x19,46,21,2]}"
S["split-rezoom"] = ssh(*world(["$0 1 main"], [f"$0 @0 0 {THREE} 1 3 claude"],
                               ["%0 @0 0 45 40 0 0 0 claude", "%1 @0 1 1 20 1 0 0 claude",
                                "%2 @0 2 1 19 0 0 0 claude"], hooks=[]),
                        f"rule ok display-message -p -t '@0' => 4 0 %1 {FOUR}", size=(47, 40)) + [
    "settle", f"recv %layout-change @0 {THREE} 3992,47x40,0,0,1 *Z\\n", "settle",
    f"recv %layout-change @0 {FOUR} {FOUR}\\n", "settle", f"recv %layout-change @0 {FOUR} 1c2d,47x40,0,0,1 *Z\\n",
    "settle", f"recv %layout-change @0 {FOUR} {FOUR}\\n", "settle", "dump"]


def pinned_split(capture):
    return ssh(*THREE_PANES, f"rule ok display-message -p -t '@0' => {capture}", *CAPTURES, client=None) + [
        "resize 70 35", "wait 200", "settle"]


S["layout-restore-on-release"] = pinned_split(f"3 0 %0 {SPLIT}") + ["release", "settle", "dump"]
S["layout-restore-user-zoom"] = pinned_split(f"3 1 %2 {SPLIT}") + ["release", "settle"]
S["layout-single-pane"] = pinned_split(f"1 0 %0 {SPLIT}") + ["release", "settle"]
S["layout-stale-record"] = pinned_split(f"3 0 %0 {SPLIT}") + [
    "rules-clear", *world(["$0 1 main"], [f"$0 @0 0 {SPLIT} 1 2 main"],
                          ["%0 @0 0 203 30 1 0 0 bash", "%1 @0 1 101 29 0 0 0 bash"], hooks=[]),
    "refresh", "settle", "release", "settle", "dump"]
S["layout-teardown-commands"] = pinned_split(f"3 1 %1 {SPLIT}") + ["layout-restore", "layout-restore", "ledger"]
S["layout-repin-recaptures"] = pinned_split(f"3 0 %0 {SPLIT}") + ["release", "settle", "repin", "settle",
                                                                 "wait 400", "settle", "dump"]
S["pin-ledger"] = ["inherit-pins @4,@6"] + ssh(*ONE, *CAPTURES) + [
    "settle", "resize 70 35", "wait 200", "settle", "dump", "ledger", "dump"]

# Scroll routing.
SCROLL = ssh(*ONE, *CAPTURES)


def flags(mouse, alt):
    return f"rule ok display-message -p -t %0 '#{{mouse_any_flag}} => {mouse} {alt}"


S["scroll-shell"] = SCROLL + ["settle", "scroll 3", "scroll-px 56.1234 16", "scroll-end",
                              "recv %output %0 x\\n", "scroll -2", "click 4 2", "settle"]
S["scroll-mouse-idle"] = SCROLL + [flags(1, 0), "settle", "mouse", "settle", "scroll 3", "scroll -9",
                                   "scroll-px 10 16", "scroll-px 10 16", "scroll-px 10 16", "scroll-begin",
                                   "settle", "scroll-px 10 16", "click 4 2", "settle", "dump"]
S["scroll-mouse-streaming"] = SCROLL + [flags(1, 0), "config streaming=60", "settle", "mouse", "settle",
                                        "recv %output %0 stream\\n", "scroll 3", "scroll-end",
                                        "input %0 6b", "settle"]
S["scroll-alt-mouse"] = SCROLL + [flags(1, 1), "config streaming=60", "settle", "mouse", "settle",
                                  "recv %output %0 repaint\\n", "scroll -2", "scroll-px -40 16", "settle"]
S["div-scroll-alt-no-mouse"] = SCROLL + [flags(0, 1), "settle", "mouse", "settle", "scroll 3",
                                         "scroll-px 30 16", "scroll-end", "dump"]

# The mosh sidecar.
SIDECAR = ["config renders=0", "attach", BOOT]


def clients(*rows):
    return "rule ok list-clients -F '#{client_control_mode} => " + "\\n".join(rows)


AAA = world(["$0 1 aaa", "$1 1 zzz"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 zsh", "$1 @1 0 b25e,200x50,0,0,1 1 1 vim"], [], hooks=[])
# B84: the renderer's attach fails and a desktop attaches in the same second;
# counted, the desktop is the one new client, and no count names the renderer.
# The renderer comes after, known by the pid mosh-server's program recorded,
# and the sidecar follows it.
S["sidecar-follow-renderer"] = SIDECAR + [
    *AAA, clients("1|45613|$0|/dev/ttys030", "0|41001|$1|/dev/ttys009"), "settle",
    "recv %session-changed $0 aaa\\n", "settle", "count", "settle", "rules-clear", *AAA,
    clients("1|45613|$0|/dev/ttys030", "0|41001|$1|/dev/ttys009", "0|46100|$0|/dev/ttys031"),
    "count", "settle", "refresh", "settle", "dump", "rules-clear", *AAA,
    clients("1|45613|$0|/dev/ttys030", "0|41001|$1|/dev/ttys009", "0|46100|$0|/dev/ttys031",
            "0|45533|$0|/dev/ttys026"),
    "renderer-pid 45533", "settle", "dump", "sync-mosh 40 20", "settle", "repin", "settle",
    "select-session $1", "settle", "dump", "recv %client-session-changed /dev/ttys026 $0 aaa\\n", "settle",
    "recv %client-detached /dev/ttys026\\n", "sync-mosh 40 20", "settle", "dump"]
S["sidecar-unknown-renderer"] = SIDECAR + [
    *AAA, clients("1|45613|$0|/dev/ttys030", "0|41001|$1|/dev/ttys009", "0|45533|$0|/dev/ttys026"),
    "settle", "recv %session-changed $0 aaa\\n", "settle", "refresh", "settle", "sync-mosh 40 20",
    "repin", "select-session $1", "settle", "dump"]
S["sidecar-status-line"] = SIDECAR + [
    # A tmux that refuses the phone's group (B773): the status line alone.
    *world(["$1 2 work"], [], [], hooks=[]), clients("1|45613|$1|/dev/ttys030"),
    "rule err new-session -t => sessions should be nested with care", "rule ok new-session => $5",
    "settle", "recv %session-changed $1 work\\n", "settle", "new-session mine", "settle",
    "recv %session-changed $5 mine\\n", "settle", "select-session $1", "settle",
    "recv %session-changed $1 work\\n", "settle", "new-session \\s", "settle", "dump", "restore-status", "settle"]
S["div-sidecar-window-close"] = SIDECAR + [
    *world(["$0 1 aaa"], ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 0 1 htop",
                          "$0 @1 2 b25f,80x24,0,0,1 1 1 vim"], [], hooks=[]),
    clients("1|45613|$0|/dev/ttys030"), "settle", "recv %session-changed $0 aaa\\n", "settle", "dump",
    "rules-clear", *world(["$0 1 aaa"], ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 1 1 htop"], [], hooks=[]),
    clients("1|45613|$0|/dev/ttys030"),
    "recv %session-window-changed $0 @2\\n%unlinked-window-close @1\\n", "settle", "dump"]
S["sidecar-burst"] = SIDECAR + [
    *world(["$0 1 aaa"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 zsh"], [], hooks=[]), clients("1|45613|$0|/dev/ttys030"),
    "settle", "recv %session-changed $0 aaa\\n", "settle",
    "recv %sessions-changed\\n%client-session-changed /dev/ttys009 $0 aaa\\n%sessions-changed\\n%unlinked-window-add @7\\n%unlinked-window-add @8\\n%window-renamed @0 marker\\n",
    "settle", "dump"]

# Divergences: one per fix the port made (harness/controller/divergences.json).
S["div-refused-capture"] = ssh(*ONE, *CAPTURES) + [
    "settle", "rules-clear", *ONE, "rule err capture-pane => can't find pane: %0",
    "rule err display-message => can't find pane: %0", "resync 1", "settle"]
S["div-cursor-overflow"] = ssh(*ONE, *CAPTURES) + ["settle", "resync 1",
                                                   "recv %begin 7 7 1\\nok\\n%end 7 7 1\\n%begin 8 8 1\\n0 9223372036854775807\\n%end 8 8 1\\n"]
S["div-dead-pane-terminals"] = ssh(*world(["$0 1 main"], ["$0 @0 0 81x24,0,0{40x24,0,0,0,40x24,41,0,1} 1 2 main"],
                                        ["%0 @0 0 40 24 1 0 0 bash", "%1 @0 1 40 24 0 0 0 vim"], hooks=[]),
                                  *CAPTURES) + [
    "settle", "rules-clear", *ONE, *CAPTURES,
    "recv %layout-change @0 b25d,80x24,0,0,0 b25d,80x24,0,0,0 *\\n", "refresh", "settle",
    "select-window @0", "settle", "dump"]
S["div-sidecar-current-window"] = SIDECAR + [
    *world(["$0 1 aaa"], ["$0 @0 0 b25d,80x24,0,0,0 1 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 0 1 htop"], [], hooks=[]),
    clients("1|45613|$0|/dev/ttys030"), "settle", "recv %session-changed $0 aaa\\n", "settle",
    "recv %session-window-changed $0 @2\\n", "settle", "dump"]
S["div-probe-stuck-after-exit"] = lease_rig(CLIENTS_A, None) + [
    "rules-clear", "resize 60 30", "wait 180", "recv %exit\\n", "skip",
    "recv %begin 300 300 0\\n%end 300 300 0\\n%session-changed $0 main\\n", CLIENTS_A,
    "rule ok display-message -p '#{client_name}' => moshpit-a",
    "rule ok show-options -wqv => -", *LEASE_WORLD, "rule ok capture-pane => READY",
    "rule ok display-message => 0 0", "settle", "recv %layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\\n",
    "settle", "dump"]
S["div-flags-refused"] = SCROLL + [flags(1, 1), "settle", "mouse", "settle", "rules-clear", *ONE,
                                   "rule err display-message -p -t %0 '#{mouse_any_flag} => can't find pane: %0",
                                   "mouse", "settle", "dump"]
S["div-path-target"] = ssh(*ONE, *CAPTURES) + ["settle", "path not-a-pane", "settle"]
S["div-prefix-unanswered"] = ["config renders=1", "prefix", "attach", BOOT, *ONE, "prefix", "close"]
S["div-lease-clients-refused"] = ssh("rule err list-clients -F '#{client_name}' => no clients",
                                     "rule ok show-options -wqv => moshpit-b|iPad|1757999998", *LEASE_WORLD,
                                     "rule ok capture-pane => READY", "rule ok display-message => 0 0") + [
    "settle", "dump"]

# B273: blocks that answer no command of ours, and commands that answer
# more than once.
S["div-hook-blocks"] = ssh(*TWO_WINDOWS, *CAPTURES,
                           "rule ok select-window => - => %begin 9000 9000 0\\n%end 9000 9000 0\\n") + [
    "settle", "select-window @1", "settle", "refresh", "settle", "dump"]
S["div-multi-reply-run"] = ssh(*ONE, *CAPTURES,
                               "rule ok if-shell => - => %begin 9100 9100 1\\ninner\\n%end 9100 9100 1\\n") + [
    "settle", "run if-shell -F 1 'display-message -p inner'", "refresh", "settle", "dump"]
# B87: a window linked into two sessions, unlinked from the other one.
LINKED = world(["$0 1 main", "$1 0 other"],
               ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @1 1 b25e,80x24,0,0,1 1 1 vim",
                "$1 @1 5 b25e,80x24,0,0,1 0 1 vim", "$1 @2 0 b25f,80x24,0,0,2 1 1 htop"],
               ["%0 @0 0 80 24 1 0 0 zsh", "%1 @1 0 80 24 1 0 0 vim", "%2 @2 0 80 24 1 0 0 htop"], hooks=[])
UNLINKED = world(["$0 1 main", "$1 0 other"],
                 ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @1 1 b25e,80x24,0,0,1 1 1 vim",
                  "$1 @2 0 b25f,80x24,0,0,2 1 1 htop"],
                 ["%0 @0 0 80 24 1 0 0 zsh", "%1 @1 0 80 24 1 0 0 vim", "%2 @2 0 80 24 1 0 0 htop"], hooks=[])
S["div-linked-window-close"] = ssh(*LINKED, *CAPTURES) + [
    "settle", "dump", "rules-clear", *UNLINKED, *CAPTURES, "recv %window-close @1\\n", "dump", "settle", "dump"]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else DIR
    os.makedirs(out, exist_ok=True)
    for name, steps in S.items():
        with open(os.path.join(out, name + ".scn"), "w") as f:
            f.write("# Generated by harness/controller/scenarios.py.\n")
            f.write("\n".join(steps) + "\n")
    print(f"{len(S)} scenarios")


main()
