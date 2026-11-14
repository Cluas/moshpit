#!/usr/bin/env python3
"""Cases for the herdr equivalence harness.

    corpus.py smoke <out>              a quick mixed sample
    corpus.py base <out>               the committed corpus: every Swift test's
                                       inputs, the real captures, edge cases
    corpus.py fuzz <out> <seed> <n>    seeded random cases (n per family)

A case is one line: a kind and its arguments, `x:<hex>` for strings and
bytes, `-` for nil (see src/harness.rs). Fuzz cases are generated from a
seed with Python's `random.Random`, so a seed reproduces its corpus.
"""
import base64
import json
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
APP_FIXTURES = os.path.join(REPO, "Tests", "MoshpitTests", "Fixtures")
FIXTURES = os.path.join(CRATE, "tests", "fixtures")


def x(value):
    """A string or bytes argument."""
    if isinstance(value, str):
        value = value.encode("utf-8")
    return "x:" + value.hex()


def opt(value):
    return "-" if value is None else x(value)


def frame(seq, payload, full=False, width=60, height=20, extra=None):
    body = {"type": "terminal.frame", "seq": seq, "encoding": "ansi", "width": width,
            "height": height, "full": full, "bytes": base64.b64encode(payload).decode()}
    if extra:
        body.update(extra)
    return json.dumps(body, separators=(",", ":"), ensure_ascii=False)


TWO_WORKSPACES = (
    '{"id":"cli:api:snapshot","result":{"snapshot":{'
    '"workspaces":[{"workspace_id":"w1","label":"~","focused":true},'
    '{"workspace_id":"w2","label":"api","focused":false}],'
    '"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":2}],'
    '"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":false},'
    '{"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","focused":true}],'
    '"layouts":[],'
    '"focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p2"}}}'
)

ONE_PANE = (
    '{"id":"cli:api:snapshot","result":{"snapshot":{'
    '"workspaces":[{"workspace_id":"w1","label":"~","focused":true}],'
    '"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],'
    '"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true}],'
    '"layouts":[],'
    '"focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}'
)

STATUS_OLD_CLIENT = """client:
  version: 0.8.0
  channel: stable
  protocol: 19

server:
  status: running
  version: 0.9.5
  protocol: 22
  compatible: no
  socket: /Users/u/.config/herdr/herdr.sock
"""

STATUS_MATCHED = """client:
  version: 0.9.5
  channel: stable
  protocol: 22
  endpoint_protocol_generation: 1

server:
  status: running
  version: 0.9.5
  endpoint_compatible: yes
"""

STATUS_OLD_SERVER = """client:
  version: 0.9.5
  channel: stable
  protocol: 22

server:
  status: running
  version: 0.8.0
  endpoint_compatible: no
"""

UUID = "6F9619FF-8B86-D011-B42D-00C04FC964FF"


def retarget_cases():
    """A follow (plain attach) and the person's pick (`--takeover`), the
    herdr the loop runs named or not: the base corpus's until fix
    retarget-unseen-refusal (harness/divergences.json) made every retarget
    command the port's own."""
    key = UUID + "-abc12345"
    cases = []
    for takeover in (0, 1):
        cases.append(f"launch.retarget {takeover} {x('term_abc123')} {x(key)}")
        cases.append(f"launch.retarget {takeover} {x(chr(97) + chr(39) + '; rm -rf /')} {x(key)}")
        cases.append(f"launch.retarget {takeover} {x('term_abc123')} {x(key)} -")
        cases.append(f"launch.retarget {takeover} {x('term_abc123')} {x(key)} {x('/opt/herdr')}")
        cases.append(f"launch.retarget {takeover} {x('term_abc123')} {x(key)} {x('herdr --session s')}")
    return cases


def app_fixture(name):
    with open(os.path.join(APP_FIXTURES, name), encoding="utf-8") as f:
        return f.read()


def base_cases():
    """Every input the Swift tests use, the captures, and the edges found
    while characterising Foundation."""
    cases = []
    add = cases.append
    snapshot = app_fixture("herdr-snapshot.json")
    socket_snapshot = app_fixture("herdr-socket-snapshot.json")
    events = app_fixture("herdr-socket-events.jsonl")

    # Frame channel: HerdrFrameChannelTests.
    stream = ("The default interactive shell is now zsh.\r\n"
              "stty raw -echo; herdr terminal session control w1:p1\r\n"
              + frame(1, b"ok") + "\n")
    for plan in ("whole", "bytes", "rand:1:7", "fixed:3"):
        add(f"frames {plan} {x(stream)}")
    add(f"frames whole {x(frame(1, b'\x1b[2Jhi', True) + chr(10))}")
    add(f"frames whole {x(frame(1, b'a') + chr(10) + frame(2, b'b') + chr(10))}")
    add(f"frames whole {x('{\"type\":\"terminal.closed\",\"reason\":\"pane gone\"}' + chr(10))}")
    add(f"frames whole {x('herdr: server rejected handshake (version 16): client version 19 is newer than server version 16; please upgrade the herdr server' + chr(13) + chr(10))}")
    add(f"frames whole {x('{\"id\":\"cli:api:snapshot\",\"error\":{\"code\":\"protocol_mismatch\",\"message\":\"client protocol 19 is newer than server protocol 16' + chr(92) + 'nRun herdr server stop\"}}' + chr(10))}")
    add(f"frames whole {x('dev@host ~ % ' + frame(3, b'z') + chr(10))}")
    add(f"frames whole {x('{\"type\":\"terminal.fra' + chr(10) + frame(9, b'back') + chr(10))}")
    add(f"frames whole {x('{\"type\":\"terminal.future\",\"x\":1}' + chr(10))}")
    add(f"frames whole {x('herdr: lost connection to server: server closed connection' + chr(13) + chr(10))}")
    add(f"frames whole {x('{\"type\":\"moshpit.ready\"}' + chr(10))}")
    add(f"frames whole {x('{\"type\":\"terminal.closed\",\"reason\":null}' + chr(10) + '{\"type\":\"terminal.closed\"}' + chr(10))}")
    for message in ("lost connection to server: server closed connection",
                    "lost connection to remote Herdr: broken pipe",
                    "terminal session control input ignored: invalid json command"):
        add(f"drop {x(message)}")
    # A follow's plain attach, and the person's pick (`--takeover`).
    for take in ("plain", "take"):
        add(f"start {x('w1:p2')} 52 30 - {take}")
        add(f"start {x('w1:p1')} 40 20 {x('/opt/herdr/bin/herdr')} {take}")
        add(f"boot {x('term_abc')} 52 30 - {take} {x(UUID)}")
        add(f"boot {x('t')} 10 10 - {take} {x(UUID)}")
        add(f"boot {x('t')} 10 10 {x(chr(47) + 'opt/o' + chr(39) + 'clock/herdr')} {take} {x(UUID)}")
        add(f"retarget {x('term_abc')} 52 30 {take}")
        add(f"retarget {x('t')} 0 -3 {take}")
    add(f"input {x(bytes([0x1b, 0x5b, 0x41]) + 'é'.encode())}")
    add(f"input {x(b'')}")
    add(f"input {x(bytes(range(256)))}")
    add("resize 0 -4")
    add("resize 52 30")
    add("scroll 1 3 wheel")
    add("scroll 0 0 page_key")
    add("consts")
    add(f"paste {x('one' + chr(27) + '[201~two')}")
    add(f"paste {x('first line' + chr(10) + 'second line')}")
    add(f"paste {x('/tmp/moshpit-uploads/shot.png')}")
    add(f"paste {x('')}")
    for seq in (b"\x1b[A", b"\x1bOB", b"\x1b[C", b"\x1bOD", b"A", b"\x1b[A\x1b[A", b"\x1b[H", b"\x1b[", b""):
        add(f"arrow {x(seq)}")

    # Snapshot decoding: HerdrSnapshotTests.
    add(f"snapshot {x(snapshot)}")
    add(f"firstobj {x(snapshot)}")
    newer = ('{"id":"x","result":{"snapshot":{\n'
             '  "workspaces":[{"workspace_id":"w1","label":"~","focused":true}],\n'
             '  "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],\n'
             '  "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","focused":true,\n'
             '            "agent_status":"working","agent":"claude","display_agent":"Claude Code",\n'
             '            "terminal_title_stripped":"claude — moshpit"}],\n'
             '  "layouts":[],\n'
             '  "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1"}}}')
    add(f"snapshot {x(newer)}")
    add(f"snapshot {x(chr(123) + '\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"solo\",\"focused\":true}],\"tabs\":[],\"panes\":[],\"layouts\":[]}')}")
    for status in ("working", "blocked", "done", "unknown", "idle"):
        pane = ('{"result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"~","focused":true}],'
                '"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1,"focused":true,"pane_count":1}],'
                '"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"%s"}],'
                '"layouts":[]}}}' % status)
        add(f"snapshot {x(pane)}")
    add(f"snapshot {x('Welcome to Ubuntu 24.04' + chr(10) + chr(123) + '\"result\":{\"snapshot\":{\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"~\"}],\"tabs\":[],\"panes\":[],\"layouts\":[]}}}')}")
    add(f"snapshot {x(chr(123) + '\"result\":{\"snapshot\":{\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"weird{name}\"}],\"tabs\":[],\"panes\":[],\"layouts\":[]}}}')}")
    for raw in ("", "not json at all", "{}", '{"result":{}}'):
        add(f"snapshot {x(raw)}")
    agents08 = ('{"result":{"snapshot":{"workspaces":[{"workspace_id":"wP","label":"sample-app","focused":true}],'
                '"tabs":[{"tab_id":"wP:t1","workspace_id":"wP","label":"claude","number":1,"focused":true}],'
                '"panes":[{"pane_id":"wP:p1","tab_id":"wP:t1","workspace_id":"wP","focused":true,'
                '"agent_status":"idle","cwd":"/Users/u/code/sample-app","terminal_title_stripped":"claude --resume 7d04"}],'
                '"layouts":[],"agents":[{"agent":"claude","agent_status":"idle","pane_id":"wP:p1",'
                '"workspace_id":"wP","tab_id":"wP:t1","focused":true}],'
                '"focused_workspace_id":"wP","focused_tab_id":"wP:t1","focused_pane_id":"wP:p1"}}}')
    add(f"snapshot {x(agents08)}")
    legacy = ('{"result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"x"}],"tabs":[],"layouts":[],'
              '"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","agent":"codex","agent_status":"working"}]}}}')
    add(f"snapshot {x(legacy)}")
    worktree = ('{"result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"~","focused":true},'
                '{"workspace_id":"w4","label":"fix-scroll","worktree":{"is_linked_worktree":true,"repo_name":"moshpit",'
                '"checkout_path":"/Users/x/.herdr/worktrees/moshpit/fix-scroll"}}],"tabs":[],"panes":[],"layouts":[]}}}')
    add(f"snapshot {x(worktree)}")
    add(f"snapshot {x(socket_snapshot)}")
    add(f"snapshot {x(TWO_WORKSPACES)}")
    for pane_id in ("w1:p1", "w1:p3", "w12:p47", "nonsense", "w1:p+5", "w1:p-5", "w1:p", "w1:p2:p3", "w1:p99999999999999999999"):
        add(f"paneindex {x(pane_id)}")
    not_running = '{"error":{"code":"server_not_running","message":"no herdr server is running at /x/herdr.sock"},"id":"cli:api:snapshot"}'
    add(f"notrunning {x(not_running)}")
    add(f"notrunning {x(chr(123) + '\"error\":{\"code\":\"not_found\"}}')}")
    add(f"snapshot {x(not_running)}")
    add(f"mismatch {x('{\"id\":\"cli:api:snapshot\",\"error\":{\"code\":\"protocol_mismatch\",\"message\":\"client protocol 19 is older than server protocol 22; upgrade the Herdr client before using this command\"}}')}")
    add(f"mismatch {x('{\"error\":{\"code\":\"protocol_mismatch\"}}')}")
    for message in ("client protocol 19 is older than server protocol 22",
                    "client protocol 19 is newer than server protocol 16",
                    "server rejected handshake (version 16): …"):
        add(f"skew {x(message)}")

    # Versions: HerdrVersionReportTests.
    for status in (STATUS_OLD_CLIENT, STATUS_MATCHED, STATUS_OLD_SERVER,
                   "client:\n  version: 0.9.5\n\nserver:\n  status: not running\n",
                   "error: unknown command 'status'", "", "Last login: Tue on ttys001\n" + STATUS_OLD_CLIENT):
        add(f"status {x(status)}")
    for a, b in (("0.10.0", "0.9.5"), ("0.8.0", "0.9.5"), ("1.0", "1.0.0"), ("0.9.5-preview", "0.9.5"),
                 ("0.7.4", "0.7.5"), ("", "0.7.5"), ("1..2", "1.2"), (".5", "5")):
        add(f"vcompare {x(a)} {x(b)}")

    # Socket: HerdrSocketClientTests.
    add(f"socket whole 0 {x(events)}")
    add(f"socket bytes 0 {x(events)}")
    event = '{"event":"pane_focused","data":{"pane_id":"w1:p1"}}'
    add(f"socket fixed:17 0 {x(event + chr(10) + 'Last login: Sun Aug 17 on ttys002' + chr(10) + event + chr(10))}")
    reply = socket_snapshot.strip().replace('"id": "s1"', '"id": "mp1"')
    add(f"socket whole 1 {x(reply + chr(10))}")
    add(f"socket whole 1 {x('{\"id\":\"mp1\",\"error\":{\"code\":\"pane_not_found\",\"message\":\"no such pane\"}}' + chr(10))}")
    add(f"socket whole 2 {x('{\"id\":\"\",\"error\":{\"code\":\"invalid_request\",\"message\":\"missing field\"}}' + chr(10))}")
    add(f"socket whole 1 {x('')}")
    add(f"socket whole 2 {x('stty raw -echo; exec python3 -c ' + chr(39) + 'os.write(1,b\"MOSHPIT_PUMP_READY' + chr(92) + 'n\")' + chr(39) + chr(10) + chr(27) + 'kstty' + chr(27) + chr(92) + 'MOSHPIT_PUMP_READY' + chr(13) + chr(10) + '{\"id\":\"mp2\",\"result\":{}}' + chr(10) + '{\"id\":\"mp9\",\"result\":{}}' + chr(10))}")
    add(f"request {x('ping')}")
    add(f"request {x('session.snapshot')}")
    add(f"subscribe {x('tab.created')} {x('pane.agent_status_changed=w1:p1')}")
    add(f"subscribe {x(','.join(['workspace.created', 'pane.updated']))} {x('')}")
    add("pumpboot")
    add("kinds")

    # Launch: HerdrLaunchTests.
    key = UUID + "-abc12345"
    add(f"launch.attach -")
    add(f"launch.attach {x('/opt/herdr')}")
    add(f"launch.attach {x('')}")
    add(f"launch.command {x('api snapshot')} -")
    add(f"launch.daemon -")
    add(f"launch.daemon {x('/opt/herdr')}")
    add(f"launch.key {x(UUID)} {x('abc12345')}")
    add(f"launch.paths {x(key)}")
    add(f"launch.loop {x(key)} -")
    add(f"launch.loop {x(key)} {x('/opt/herdr')}")
    add(f"launch.cleanup {x(UUID)}")
    # A follow (plain attach) and the person's pick (`--takeover`): in the
    # divergence corpus while fix retarget-unseen-refusal is open (every
    # retarget command differs from the app's), RETARGET_CASES.
    for said in ("MOSHPIT_RETARGET_MISSING\n", "", "MOSHPIT_RETARGETED\n"):
        add(f"launch.missing {x(said)}")
    add(f"launch.check {x('term_abc123')} {x(key)}")
    add(f"launch.check {x(chr(97) + chr(39) + '; rm -rf /')} {x(key)}")
    for said in ("MOSHPIT_RETARGETED\n", "", "kill: usage\n", "x MOSHPIT_RETARGETEDMOSHPIT_ATTACHED", "MOSHPIT_RETARGET"):
        add(f"launch.landed {x(said)}")
        add(f"launch.up {x(said)}")
    add(f"launch.up {x('MOSHPIT_ATTACHED' + chr(10))}")
    add(f"launch.watch {x(key)}")
    add(f"launch.loopup {x(key)}")
    for said in ("MOSHPIT_LOOP_UP\n", "", "MOSHPIT_LOOP", "x MOSHPIT_LOOP_UP"):
        add(f"launch.isup {x(said)}")
    # What a watch prints: herdr 0.9.5's last words, measured live.
    nl = chr(10)
    for said in ("herdr: server shut down: terminal attach taken over",
                 "herdr: server shut down: terminal attach ended: terminal term_65ffe2c6b497f2 not found",
                 "herdr: server shut down: terminal attach failed: terminal term_00000000000000 not found",
                 "herdr: server shut down: server is shutting down",
                 "herdr: server shut down: terminal attach failed: terminal term_1 already has an attached client; retry with --takeover",
                 "sh: 1: exec: herdr: not found", ""):
        add(f"launch.stop {x('MOSHPIT_RENDERER_STOPPED' + nl + '1 term_65ffe2a36e7ee1' + nl + said + nl)}")
        for status in (1, 127):
            for listed, up in ((1, 1), (0, 1), (1, 0), (0, 0)):
                add(f"launch.stopkind {status} {x(said)} {listed} {up}")
    add(f"launch.stop {x('MOSHPIT_RENDERER_GONE' + nl)}")
    add(f"launch.stop {x('')}")
    add(f"launch.stop {x('MOSHPIT_RENDERER_STOPPED')}")
    add(f"launch.stop {x('MOSHPIT_RENDERER_STOPPED' + nl + 'x' + nl)}")
    add(f"launch.probe -")
    for value in ("it's", "", "plain", "''", "a'b'c"):
        add(f"quote {x(value)}")

    # Control client: HerdrControlClientTests.
    add(f"ctl.parse {x('payload' + chr(10) + '__moshpit_rc=7' + chr(10))}")
    add(f"ctl.parse {x('payload' + chr(10) + '__moshpit_rc=0')}")
    add(f"ctl.parse {x('payload only')}")
    add(f"ctl.parse {x('a__moshpit_rc=1' + chr(10) + 'b__moshpit_rc=2')}")
    add(f"ctl.parse {x('x__moshpit_rc=')}")
    add(f"ctl.parse {x('x__moshpit_rc=007z')}")
    created = ('{"id":"cli:worktree:create","result":{"type":"worktree_created",'
               '"workspace":{"workspace_id":"w4","label":"fix-scroll","focused":true},'
               '"root_pane":{"pane_id":"w4:p1","cwd":"/Users/dev/.herdr/worktrees/moshpit/fix-scroll"}}}')
    for output, code in (
            ('{"id":"cli:request","error":{"code":"pane_not_found","message":"pane w4:p1 not found"}}', "1"),
            ('{"id":"cli:agent:prompt","error":{"code":"agent_blocked","message":"agent w4:p1 is blocked and requires interactive input"}}', "1"),
            ('{"id":"cli:agent:prompt","error":{"code":"agent_not_found","message":"agent target w4:p1 not found"}}', "1"),
            ('{"id":"cli:agent:prompt","error":{"code":"agent_not_ready","message":"agent w4:p1 is not an active named agent"}}', "1"),
            ('{"id":"cli:pane:run","result":{"type":"ok"}}', "0"),
            ("usage: herdr agent <list|get|read|send|rename|focus|wait|attach|start|explain>", "2"),
            ("error: unrecognized subcommand 'send'", "2"),
            ("", "0"), ("", "1"), ("", "none"),
            ('{"error":{"code":"x"}}', "0"),
            ("   \n  first printed line  \nsecond", "3"),
            (created, "0")):
        add(f"ctl.refusal {x(output)} {code}")
        if code != "none":
            add(f"ctl.notup {x(output)} {code}")
    add(f"ctl.errmsg {x(created)}")
    add(f"ctl.errmsg {x('{\"error\":{\"code\":\"worktree_create_failed\",\"message\":\"branch fix-scroll already exists\"}}')}")
    add(f"ctl.errmsg {x('{\"error\":{\"code\":\"only_code\"}}')}")
    for version, verb in (("0.7.3", 1), ("0.7.4", 1), ("0.7.5", 0), ("0.8.0", 0), ("0.9.5", 0), (None, 0)):
        add(f"ctl.first {x('fix the scroll')} {x('w4:p1')} {opt(version)}")
    add(f"ctl.submit {x('w4:p1')}")
    add("ctl.homescan")
    valid = ("/Users/dev/code/moshpit", "fix-scroll-jump", "claude")
    add(f"ctl.validate {x(valid[0])} {x(valid[1])} {x(valid[2])}")
    add(f"ctl.validate {x('  ')} {x(valid[1])} {x(valid[2])}")
    add(f"ctl.validate {x(valid[0])} {x(valid[1])} {x('')}")
    for branch in ("", "  ", "two words", "-leading-dash", "/leading-slash", "trailing-slash/", "double..dot",
                   "ends.lock", "star*", "question?", "colon:", "tilde~", "caret^", "bracket[", "back\\slash",
                   "fix-scroll-jump", "feature/thing", "v2", "a_b.c", "用户-分支", "  fix-scroll  ", "tab\there",
                   "del\x7f", "nl\nx"):
        add(f"ctl.validate {x(valid[0])} {x(branch)} {x(valid[2])}")
    add(f"ctl.repolines {x('/Users/dev/code/scanned' + chr(10) + '/Users/dev/code/moshpit' + chr(10))}")
    add(f"ctl.repolines {x('noise' + chr(10) + '  /a  ' + chr(10) + '/a' + chr(13) + chr(10) + 'git: fatal' + chr(10))}")
    for action in (f"sel-session {x('w2')}", f"sel-window {x('w1:t1')}", f"kill-session {x('w2')}",
                   f"kill-window {x('w1:t1')}", f"kill-pane {x('w1:p1')}",
                   f"ren-session {x('w1')} {x('api')}", f"ren-window {x('w1:t1')} {x('logs')}",
                   f"new-session {x('build')}", "new-session -", f"new-session {x('')}", "new-window -",
                   f"new-window {x('logs')}", "new-pane", f"sel-pane {x('w1:p1')}", f"sel-pane-imm {x('w1:p1')}",
                   f"ren-session {x('w1')} {x(chr(105) + 't' + chr(39) + 's; rm -rf ~')}",
                   f"new-window-in {x('w2')} -", f"new-window-in {x('w1')} {x('x')}",
                   f"new-pane-in {x('w1:t1')}", f"new-pane-in {x('w9:t9')}",
                   f"keys {x('w1:p1')} {x('up')}", f"keys {x('w1:p1')} {x('up,down,up')}", f"keys {x('w1:p1')} {x('')}",
                   f"remove {x('w4')} 0", f"remove {x('w4')} 1",
                   f"manifests {x('[{\"agent\":\"pi\",\"active_version\":\"1\"},{\"agent\":\"claude\",\"active_version\":\"2\"},{\"agent\":\"codex\"}]')}",
                   f"manifests {x('')}", f"manifests {x('noise [1,2] tail')}"):
        add(f"ctl.cmd {x(TWO_WORKSPACES)} - {action}")
        add(f"ctl.cmd {x(TWO_WORKSPACES)} {x('/opt/herdr/bin/herdr')} {action}")
    add(f"ctl.cmd {x('{}')} - new-pane")
    add(f"ctl.cmd {x(ONE_PANE)} - new-pane")
    changed = TWO_WORKSPACES.replace('"focused_pane_id":"w1:p2"', '"focused_pane_id":"w1:p1"')
    dead = "Error: Os { code: 2, kind: NotFound, message: \"No such file or directory\" }\n__moshpit_rc=1"
    add("ctl.poll - " + " ".join(x(o) for o in [TWO_WORKSPACES] * 6 + [changed]))
    add("ctl.poll - " + " ".join(x(o) for o in [TWO_WORKSPACES, "bash: herdr: command not found",
                                                 "some unrelated chatter\n__moshpit_rc=0", not_running, dead]))
    mismatch = ('{"id":"cli:api:snapshot","error":{"code":"protocol_mismatch","message":"client protocol 19 is older '
                'than server protocol 22; upgrade the Herdr client before using this command"}}\n__moshpit_rc=0')
    add(f"ctl.poll {x(STATUS_OLD_CLIENT + '__moshpit_rc=0')} {x(mismatch)} {x(mismatch)} {x(TWO_WORKSPACES)}")
    add(f"ctl.poll {x('error: unknown command' + chr(10) + '__moshpit_rc=2')} {x(mismatch)} {x(TWO_WORKSPACES)}")
    return cases


def main():
    mode = sys.argv[1]
    if mode in ("smoke", "base"):
        cases = base_cases()
    elif mode == "fuzz":
        import fuzz
        cases = fuzz.cases(int(sys.argv[3]), int(sys.argv[4]))
    else:
        sys.exit(__doc__)
    with open(sys.argv[2], "w", encoding="utf-8") as f:
        f.write("\n".join(cases) + "\n")
    print(f"{len(cases)} cases", file=sys.stderr)


if __name__ == "__main__":
    main()
