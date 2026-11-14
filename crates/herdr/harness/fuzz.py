"""Seeded fuzz cases for the herdr equivalence harness, and the tagger that
says which documented divergence a case can trigger.

`cases(seed, n)` returns n cases per family. `tags(case)` names the
divergences (harness/divergences.json) whose trigger the case contains:
equivalence.py accepts a difference between the Swift and Rust outputs only
on a tagged case, and counts every one. Untagged cases must be identical.
"""
import base64
import json
import os
import random
import re

HERE = os.path.dirname(os.path.abspath(__file__))

# MARK: alphabet

ASCII = [chr(c) for c in range(0x20, 0x7F)]
STRUCTURAL = list('"\\{}[]\';:.,/= -_')
CONTROL = ["\n", "\r", "\t", "\x00", "\x1b", "\x7f", "\x0b", "\x0c"]
# Non-ASCII that behaves the same under Swift's Character rules and bytes.
SAFE = ["é", "ñ", "ü", "中", "文", "가", "😀", "✳", "—", "“", "”", "\u00a0", "\u2028", "\u3000",
        "\u200b", "\u0085", "\ufeff", "€", "∞", "🚀", "Ω", "ß"]
# Characters on which the two can differ (harness/unicode-hazards.txt).
HAZARDS = ["\u0301", "\ufe0f", "\U0001f3fb", "\u200d", "\u0600", "\u037e", "\u212b", "\U0001f1e6",
           "\u1100", "\u1161", "\u0663", "\u4e94", "\u0d4e", "\u0903"]


def text(rng, low=0, high=12, hazards=0.02, controls=0.03, safe=0.08, structural=0.15):
    out = []
    for _ in range(rng.randint(low, high)):
        roll = rng.random()
        if roll < hazards:
            out.append(rng.choice(HAZARDS))
        elif roll < hazards + controls:
            out.append(rng.choice(CONTROL))
        elif roll < hazards + controls + safe:
            out.append(rng.choice(SAFE))
        elif roll < hazards + controls + safe + structural:
            out.append(rng.choice(STRUCTURAL))
        else:
            out.append(rng.choice(ASCII))
    return "".join(out)


def word(rng, low=1, high=8):
    return "".join(rng.choice("abcdefghijklmnopqrstuvwxyz0123456789_:.-") for _ in range(rng.randint(low, high)))


def x(value):
    if isinstance(value, str):
        value = value.encode("utf-8")
    return "x:" + value.hex()


def opt(value):
    return "-" if value is None else x(value)


# MARK: JSON text

def json_string(rng, value):
    """`value` as a JSON string literal, escapes chosen at random."""
    out = ['"']
    for ch in value:
        code = ord(ch)
        roll = rng.random()
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif code < 0x20:
            short = {"\b": "\\b", "\f": "\\f", "\n": "\\n", "\r": "\\r", "\t": "\\t"}
            if ch in short and roll < 0.6:
                out.append(short[ch])
            else:
                out.append("\\u%04x" % code if roll < 0.8 else "\\u%04X" % code)
        elif ch == "/" and roll < 0.3:
            out.append("\\/")
        elif code > 0xFFFF and roll < 0.2:
            v = code - 0x10000
            out.append("\\u%04x\\u%04x" % (0xD800 + (v >> 10), 0xDC00 + (v & 0x3FF)))
        elif code > 0x7F and roll < 0.15:
            out.append("\\u%04x" % code)
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def number_literal(rng):
    kind = rng.random()
    if kind < 0.35:
        return str(rng.choice([0, 1, -1, 2, 7, 24, 80, 52, 30, 255, 65535, 65536, 70000]))
    if kind < 0.5:
        return str(rng.choice([2**63 - 1, 2**63, -(2**63), -(2**63) - 1, 2**64 - 1, 2**64, 10**25,
                               -10**20, 12345678901234567890]))
    if kind < 0.6:
        return rng.choice(["-0", "0.0", "-0.0", "1.0", "1.5", "2e0", "1E2", "1e-5", "0.5", "100e-2",
                           "1.00e0", "3.0", "1e18", "1e19", "1e20", "-1e19", "9.5e1"])
    if kind < 0.7:
        return rng.choice(["1e308", "1e309", "-1e309", "1.8e308", "-1.8e308", "1e-400", "-1e-400",
                           "4e-320", "1e0001", "1e+0001", "1e-0001", "1e00001", "0e0000", "0e00000",
                           "1E-0", "1e+0", "-9e999", "9e999"])
    if kind < 0.8:
        digits = "".join(rng.choice("0123456789") for _ in range(rng.randint(1, 22)))
        frac = "".join(rng.choice("0123456789") for _ in range(rng.randint(1, 22)))
        lead = digits.lstrip("0") or "0"
        return ("-" if rng.random() < 0.3 else "") + lead + "." + frac
    if kind < 0.9:
        # Malformed on purpose.
        return rng.choice(["01", "-", "1.", ".5", "+1", "1e", "1e+", "--1", "0x10", "NaN", "Infinity",
                           "1.e5", "00", "-01", "1ee5", "Infinity", "1 2"])
    mantissa = str(rng.randint(0, 10**rng.randint(1, 9)))
    exp = rng.randint(-30, 30)
    return mantissa + rng.choice(["e", "E"]) + (str(exp) if exp < 0 or rng.random() < 0.5 else "+" + str(exp))


def json_value(rng, depth=0, keys=None):
    roll = rng.random()
    if depth > 3 or roll < 0.25:
        return rng.choice([lambda: json_string(rng, text(rng)), lambda: number_literal(rng),
                           lambda: rng.choice(["true", "false", "null", "tru", "nul", "True"])])()
    if roll < 0.6:
        return json_object(rng, depth + 1, keys)
    items = [json_value(rng, depth + 1, keys) for _ in range(rng.randint(0, 4))]
    return "[" + sep(rng).join(items) + trailing(rng, items) + "]"


def sep(rng):
    return rng.choice([",", ", ", " , ", ",\n", ",\t", ",\r\n"])


def trailing(rng, items):
    return "," if items and rng.random() < 0.08 else ""


def json_object(rng, depth=0, keys=None, fields=None):
    entries = []
    for _ in range(rng.randint(0, 5)):
        if keys and rng.random() < 0.7:
            key = rng.choice(keys)
        else:
            key = text(rng, 0, 6)
        entries.append((key, json_value(rng, depth, keys)))
    if fields:
        entries.extend(fields)
        rng.shuffle(entries)
    if entries and rng.random() < 0.1:
        entries.append(rng.choice(entries))  # a duplicate key
    body = sep(rng).join(json_string(rng, k) + rng.choice([":", ": ", " :", " : "]) + v for k, v in entries)
    return "{" + rng.choice(["", " ", "\n"]) + body + trailing(rng, entries) + rng.choice(["", " ", "\r"]) + "}"


def mutate(rng, data):
    data = bytearray(data)
    for _ in range(rng.randint(1, 4)):
        op = rng.random()
        if not data:
            data.append(rng.randrange(256))
        elif op < 0.3:
            data[rng.randrange(len(data))] ^= 1 << rng.randrange(8)
        elif op < 0.5:
            del data[rng.randrange(len(data))]
        elif op < 0.7:
            data.insert(rng.randrange(len(data) + 1), rng.choice(b'{}[]",:\\\x00\n0e.-' + bytes([0xff, 0xc3, 0xef])))
        elif op < 0.85:
            data = data[: rng.randrange(len(data) + 1)]
        else:
            at = rng.randrange(len(data))
            data[at:at] = data[: rng.randrange(min(len(data), 12) + 1)]
    return bytes(data)


# MARK: herdr payloads

FRAME_KEYS = ["type", "seq", "width", "height", "full", "bytes", "encoding", "reason", "error", "message", "code", "id"]
SNAPSHOT_KEYS = ["workspace_id", "label", "focused", "worktree", "is_linked_worktree", "repo_name", "checkout_path",
                 "tab_id", "number", "pane_count", "pane_id", "terminal_id", "cwd", "foreground_cwd", "agent_status",
                 "agent", "display_agent", "terminal_title_stripped", "title", "rect", "width", "height", "zoomed",
                 "panes"]


def scalar_of(rng, good):
    """`good` most of the time, otherwise something of another type."""
    if rng.random() < 0.85:
        return good
    return rng.choice([json_string(rng, text(rng, 0, 4)), number_literal(rng), "true", "false", "null", "[]", "{}"])


def b64_value(rng, payload):
    encoded = base64.b64encode(payload).decode()
    roll = rng.random()
    if roll < 0.85:
        return encoded
    if roll < 0.9:
        return encoded.rstrip("=")
    if roll < 0.93:
        return encoded + "=" * rng.randint(1, 3)
    if roll < 0.95:
        return "=" * rng.randint(1, 8)
    if roll < 0.97 and len(encoded) >= 4:
        return encoded[:-2] + "=" + rng.choice("AQg/")
    return encoded[: rng.randrange(len(encoded) + 1)] + rng.choice(["", " ", "*", "-", "_", "\n"])


def frame_line(rng):
    roll = rng.random()
    if roll < 0.55:
        payload = text(rng, 0, 30, controls=0.15).encode("utf-8", "surrogatepass")
        fields = [("type", json_string(rng, "terminal.frame")),
                  ("seq", scalar_of(rng, str(rng.choice([0, 1, rng.randint(2, 10**6), 2**64 - 1])))),
                  ("width", scalar_of(rng, str(rng.randint(1, 300)))),
                  ("height", scalar_of(rng, str(rng.randint(1, 100)))),
                  ("full", scalar_of(rng, rng.choice(["true", "false"]))),
                  ("bytes", json_string(rng, b64_value(rng, payload)))]
        if rng.random() < 0.5:
            fields.append(("encoding", json_string(rng, "ansi")))
        rng.shuffle(fields)
        fields = [f for f in fields if rng.random() > 0.05]
        line = "{" + ",".join(json_string(rng, k) + ":" + v for k, v in fields) + "}"
    elif roll < 0.65:
        reason = rng.choice([json_string(rng, text(rng, 0, 10)), "null", "3"])
        line = '{"type":"terminal.closed","reason":%s}' % reason if rng.random() < 0.8 else '{"type":"terminal.closed"}'
    elif roll < 0.7:
        line = '{"type":"moshpit.ready"}'
    elif roll < 0.8:
        message = json_string(rng, text(rng, 0, 20, controls=0.1))
        line = '{"id":"cli:x","error":{"code":"protocol_mismatch","message":%s}}' % message
    elif roll < 0.88:
        line = rng.choice(["", " ", "\ufeff", "\u200b", "\u3000"]) + "herdr: " + text(rng, 0, 30)
    elif roll < 0.94:
        line = text(rng, 0, 30)
    else:
        line = json_object(rng, 0, FRAME_KEYS)
    if rng.random() < 0.15:
        line = text(rng, 0, 10, structural=0) + line
    data = line.encode("utf-8", "surrogatepass")
    if rng.random() < 0.15:
        data = mutate(rng, data)
    return data + rng.choice([b"\n", b"\r\n", b"\n", b""])


def snapshot_json(rng):
    workspaces = []
    tabs = []
    panes = []
    layouts = []
    agents = []
    ids = []
    for w in range(1, rng.randint(1, 3) + 1):
        wid = rng.choice([f"w{w}", f"w{w}", word(rng)])
        fields = [("workspace_id", json_string(rng, wid))]
        if rng.random() < 0.8:
            fields.append(("label", scalar_of(rng, json_string(rng, text(rng, 0, 8)))))
        if rng.random() < 0.8:
            fields.append(("focused", scalar_of(rng, rng.choice(["true", "false"]))))
        if rng.random() < 0.2:
            fields.append(("worktree", "{%s}" % ",".join(
                [f'"is_linked_worktree":{scalar_of(rng, rng.choice(["true", "false"]))}'] +
                ([f'"repo_name":{json_string(rng, text(rng, 0, 6))}'] if rng.random() < 0.6 else []) +
                ([f'"checkout_path":{json_string(rng, "/" + text(rng, 0, 8))}'] if rng.random() < 0.6 else []))))
        workspaces.append(fields)
        for t in range(1, rng.randint(0, 2) + 1):
            tid = f"{wid}:t{t}"
            tabs.append([("tab_id", json_string(rng, tid)), ("workspace_id", json_string(rng, wid)),
                         ("label", scalar_of(rng, json_string(rng, text(rng, 0, 6)))),
                         ("number", scalar_of(rng, str(t))), ("focused", scalar_of(rng, rng.choice(["true", "false"]))),
                         ("pane_count", scalar_of(rng, str(rng.randint(1, 3))))])
            lpanes = []
            for p in range(1, rng.randint(0, 3) + 1):
                pid = rng.choice([f"{wid}:p{p}", f"{wid}:p{p}", f"{wid}:p{rng.randint(1, 99)}", word(rng)])
                ids.append(pid)
                fields = [("pane_id", json_string(rng, pid)), ("tab_id", json_string(rng, tid))]
                for key, value in (("terminal_id", json_string(rng, "term_" + word(rng))),
                                   ("cwd", json_string(rng, "/" + text(rng, 0, 10))),
                                   ("foreground_cwd", json_string(rng, rng.choice(["", "/" + text(rng, 0, 10)]))),
                                   ("agent_status", json_string(rng, rng.choice(["working", "blocked", "done", "idle", "unknown", word(rng)]))),
                                   ("agent", json_string(rng, rng.choice(["claude", "codex", "", word(rng)]))),
                                   ("display_agent", json_string(rng, rng.choice(["Claude Code", "", text(rng, 0, 6)]))),
                                   ("terminal_title_stripped", json_string(rng, text(rng, 0, 10))),
                                   ("title", json_string(rng, text(rng, 0, 10))),
                                   ("focused", rng.choice(["true", "false"]))):
                    if rng.random() < 0.4:
                        fields.append((key, scalar_of(rng, value)))
                panes.append(fields)
                if rng.random() < 0.7:
                    lpanes.append('{"pane_id":%s,"rect":{"width":%s,"height":%s}}' % (
                        json_string(rng, pid), scalar_of(rng, str(rng.randint(1, 200))), scalar_of(rng, str(rng.randint(1, 60)))))
                if rng.random() < 0.3:
                    agents.append('{"pane_id":%s,"agent":%s,"agent_status":%s}' % (
                        json_string(rng, pid), json_string(rng, rng.choice(["claude", "codex", ""])),
                        json_string(rng, rng.choice(["working", "blocked", "idle"]))))
            layouts.append('{"tab_id":%s,"zoomed":%s,"panes":[%s]}' % (
                json_string(rng, tid), scalar_of(rng, rng.choice(["true", "false"])), ",".join(lpanes)))

    def objects(rows):
        return "[" + ",".join("{" + ",".join(json_string(rng, k) + ":" + v for k, v in row) + "}" for row in rows) + "]"

    body = [("workspaces", objects(workspaces)), ("tabs", objects(tabs)), ("panes", objects(panes)),
            ("layouts", "[" + ",".join(layouts) + "]")]
    if agents:
        body.append(("agents", "[" + ",".join(agents) + "]"))
    if ids and rng.random() < 0.8:
        body.append(("focused_pane_id", json_string(rng, rng.choice(ids))))
    body.append(("focused_workspace_id", json_string(rng, "w1")))
    body.append(("focused_tab_id", json_string(rng, "w1:t1")))
    for _ in range(rng.randint(0, 1)):
        index = rng.randrange(len(body))
        key, value = body[index]
        body[index] = (key, rng.choice(["null", "{}", "[1]", "[{},2]", value]))
    rng.shuffle(body)
    inner = "{" + ",".join(json_string(rng, k) + ":" + v for k, v in body) + "}"
    shape = rng.random()
    if shape < 0.7:
        raw = '{"id":"cli:api:snapshot","result":{"snapshot":%s}}' % inner
    elif shape < 0.85:
        raw = inner
    else:
        raw = '{"snapshot":%s}' % inner
    if rng.random() < 0.2:
        raw = text(rng, 0, 20) + "\n" + raw
    if rng.random() < 0.1:
        raw = mutate(rng, raw.encode("utf-8", "surrogatepass")).decode("utf-8", "replace")
    return raw


def status_text(rng):
    lines = []
    if rng.random() < 0.2:
        lines.append(text(rng, 0, 20, controls=0))
    for section in rng.sample(["client", "server", "other"], rng.randint(1, 3)):
        lines.append(rng.choice(["", " ", "\u3000"]) + section + rng.choice([":", ":", "", ": "]))
        for _ in range(rng.randint(0, 4)):
            field = rng.choice(["version", "status", "protocol", "channel", "version"])
            value = rng.choice([version(rng), "running", "19", "", text(rng, 0, 6, controls=0)])
            lines.append(rng.choice(["  ", "\t", " ", "", "\u00a0"]) + field + rng.choice([": ", ":", " : "]) + value)
    return rng.choice(["\n", "\r\n", "\n"]).join(lines) + rng.choice(["", "\n"])


def version(rng):
    roll = rng.random()
    if roll < 0.7:
        parts = [str(rng.choice([0, 0, 1, 7, 8, 9, 10, 5, 3])) for _ in range(rng.randint(1, 4))]
        out = ".".join(parts)
        if rng.random() < 0.15:
            out += rng.choice(["-preview", "-rc1", "+build", "beta"])
        return out
    if roll < 0.8:
        return ".".join(str(rng.randint(0, 10**rng.randint(1, 22))) for _ in range(rng.randint(1, 3)))
    return text(rng, 0, 8, controls=0)


def socket_stream(rng, pending):
    lines = []
    for _ in range(rng.randint(0, 8)):
        roll = rng.random()
        if roll < 0.3:
            lines.append('{"data":{"type":%s},"event":%s}' % (json_string(rng, word(rng)), json_string(rng, rng.choice(
                ["pane_updated", "tab_created", "workspace_focused", word(rng), text(rng, 0, 6)]))))
        elif roll < 0.55 and pending:
            rid = rng.choice([f"mp{rng.randint(1, pending)}", f"mp{rng.randint(1, pending + 2)}", "mpx"])
            if rng.random() < 0.6:
                lines.append('{"id":%s,"result":%s}' % (json_string(rng, rid), json_object(rng, 0, ["type", "snapshot"])))
            else:
                lines.append('{"id":%s,"error":{%s}}' % (json_string(rng, rid), ",".join(
                    ([f'"code":{scalar_of(rng, json_string(rng, word(rng)))}'] if rng.random() < 0.8 else []) +
                    ([f'"message":{json_string(rng, text(rng, 0, 10))}'] if rng.random() < 0.8 else []))))
        elif roll < 0.6:
            lines.append('{"id":"","error":{"code":"invalid_request","message":%s}}' % json_string(rng, text(rng, 0, 8)))
        elif roll < 0.7:
            lines.append(rng.choice(["", "\x1bkstty\x1b\\", " ", "echo "]) + "MOSHPIT_PUMP_READY" + rng.choice(["", " ", "\r", "x"]))
        elif roll < 0.8:
            lines.append(text(rng, 0, 20))
        else:
            lines.append(json_object(rng, 0, ["id", "event", "error", "result", "code", "message"]))
    data = "\n".join(lines).encode("utf-8", "surrogatepass") + rng.choice([b"\n", b"", b"\r\n"])
    if rng.random() < 0.1:
        data = mutate(rng, data)
    return data


def plan(rng, size):
    return rng.choice(["whole", "bytes" if size < 400 else "whole", f"rand:{rng.randint(1, 10**6)}:{rng.choice([3, 17, 64, 1000])}",
                       f"fixed:{rng.randint(1, 64)}"])


def herdr_path(rng):
    return rng.choice([None, None, "/opt/herdr/bin/herdr", "", "/opt/o'clock/herdr", "/" + text(rng, 0, 10, controls=0)])


def command_result(rng):
    roll = rng.random()
    if roll < 0.3:
        output = '{"id":"cli:x","error":{%s}}' % ",".join(
            ([f'"code":{json_string(rng, rng.choice(["agent_not_found", "agent_not_ready", "agent_blocked", "pane_not_found", word(rng)]))}'] if rng.random() < 0.85 else []) +
            ([f'"message":{json_string(rng, text(rng, 0, 30))}'] if rng.random() < 0.8 else []))
    elif roll < 0.5:
        output = '{"id":"cli:x","result":{"type":"ok"}}'
    elif roll < 0.8:
        output = "\n".join(text(rng, 0, rng.choice([10, 40, 260])) for _ in range(rng.randint(0, 3)))
    else:
        output = ""
    exit_code = rng.choice(["0", "1", "2", "127", "-", "none", "0", "1"])
    return output, exit_code


def snapshot_for_commands(rng):
    if rng.random() < 0.5:
        return snapshot_json(rng)
    from corpus import TWO_WORKSPACES
    return TWO_WORKSPACES


# MARK: families

def family_json(rng):
    data = json_value(rng) if rng.random() < 0.5 else json_object(rng)
    data = data.encode("utf-8", "surrogatepass")
    if rng.random() < 0.3:
        data = mutate(rng, data)
    if rng.random() < 0.03:
        depth = rng.choice([511, 512, 513, 514])
        data = (b"[" * depth + b"]" * depth) if rng.random() < 0.5 else (b'{"a":' * depth + b"1" + b"}" * depth)
    return f"json {x(data)}"


def family_firstobj(rng):
    raw = text(rng, 0, 10) + (json_object(rng, 0, SNAPSHOT_KEYS) if rng.random() < 0.8 else json_value(rng)) + text(rng, 0, 10)
    if rng.random() < 0.2:
        raw = mutate(rng, raw.encode("utf-8", "surrogatepass")).decode("utf-8", "replace")
    return f"firstobj {x(raw)}"


def family_frames(rng):
    stream = b"".join(frame_line(rng) for _ in range(rng.randint(1, 8)))
    return f"frames {plan(rng, len(stream))} {x(stream)}"


def family_line(rng):
    return f"line {x(frame_line(rng).rstrip(b'\n'))}"


def family_snapshot(rng):
    return f"snapshot {x(snapshot_json(rng))}"


def family_socket(rng):
    pending = rng.randint(0, 4)
    stream = socket_stream(rng, pending)
    return f"socket {plan(rng, len(stream))} {pending} {x(stream)}"


def family_versions(rng):
    if rng.random() < 0.5:
        return f"status {x(status_text(rng))}"
    return f"vcompare {x(version(rng))} {x(version(rng))}"


def family_commands(rng):
    kind = rng.choice(["start", "boot", "retarget", "input", "resize", "scroll", "paste", "arrow", "quote",
                       "drop", "skew", "paneindex", "mismatch", "notrunning", "launch"])
    size = lambda: rng.choice([1, 2, 24, 80, 52, 30, 0, -1, -4, 65535, 65536, 70000, rng.randint(-10, 400)])
    target = rng.choice(["w1:p1", "term_" + word(rng), word(rng), text(rng, 0, 8)])
    take = rng.choice(["take", "plain"])
    if kind == "start":
        return f"start {x(target)} {size()} {size()} {opt(herdr_path(rng))} {take}"
    if kind == "boot":
        uuid = "%08X-%04X-%04X-%04X-%012X" % (rng.getrandbits(32), rng.getrandbits(16), rng.getrandbits(16),
                                              rng.getrandbits(16), rng.getrandbits(48))
        return f"boot {x(target)} {size()} {size()} {opt(herdr_path(rng))} {take} {x(uuid)}"
    if kind == "retarget":
        return f"retarget {x(target)} {size()} {size()} {take}"
    if kind == "input":
        return f"input {x(text(rng, 0, 20, controls=0.2).encode('utf-8', 'surrogatepass'))}"
    if kind == "resize":
        return f"resize {size()} {size()}"
    if kind == "scroll":
        return f"scroll {rng.randint(0, 1)} {size()} {rng.choice(['wheel', 'page_key'])}"
    if kind == "paste":
        pieces = [text(rng, 0, 10), "\x1b[201~", "\x1b[200~", "\x1b[20", "1~", "\x1b[201", "~"]
        return f"paste {x(''.join(rng.choice(pieces) for _ in range(rng.randint(0, 6))))}"
    if kind == "arrow":
        return f"arrow {x(bytes(rng.choice([0x1b, ord('['), ord('O'), ord('A'), ord('B'), ord('C'), ord('D'), ord('H'), 0x41]) for _ in range(rng.randint(0, 4))))}"
    if kind == "quote":
        return f"quote {x(text(rng, 0, 12, structural=0.4))}"
    if kind == "drop":
        return f"drop {x(rng.choice(['', 'lost connection to server', 'lost connection to serve']) + text(rng, 0, 8))}"
    if kind == "skew":
        pieces = ["rejected handshake", "protocol", "newer", "older than server", " ", text(rng, 0, 6)]
        return f"skew {x(''.join(rng.choice(pieces) for _ in range(rng.randint(0, 5))))}"
    if kind == "paneindex":
        return f"paneindex {x(rng.choice(['w1', word(rng), '']) + rng.choice([':p', ':p', ':', 'p', '']) + rng.choice([str(rng.randint(-5, 10**rng.randint(1, 20))), '+3', '-0', text(rng, 0, 4), '']))}"
    if kind in ("mismatch", "notrunning"):
        code = rng.choice(["protocol_mismatch", "server_not_running", word(rng)])
        message = rng.choice([None, text(rng, 0, 30, structural=0.3)])
        body = '{"error":{"code":%s%s}}' % (json_string(rng, code), "" if message is None else ',"message":' + json_string(rng, message))
        return f"{kind} {x(text(rng, 0, 6) + body)}"
    which = rng.choice(["attach", "command", "daemon", "key", "paths", "loop", "cleanup", "retarget", "probe",
                        "check", "landed", "watch", "stop"])
    uuid = "%08X-%04X-%04X-%04X-%012X" % (rng.getrandbits(32), rng.getrandbits(16), rng.getrandbits(16), rng.getrandbits(16), rng.getrandbits(48))
    key = uuid + "-" + word(rng)
    if which in ("attach", "daemon", "probe"):
        return f"launch.{which} {opt(herdr_path(rng))}"
    if which == "command":
        return f"launch.command {x(text(rng, 0, 10))} {opt(herdr_path(rng))}"
    if which == "key":
        return f"launch.key {x(uuid)} {x(word(rng))}"
    if which == "paths":
        return f"launch.paths {x(key)}"
    if which == "loop":
        return f"launch.loop {x(key)} {opt(herdr_path(rng))}"
    if which == "cleanup":
        return f"launch.cleanup {x(uuid)}"
    if which == "check":
        return f"launch.check {x(text(rng, 0, 10, structural=0.4))} {x(key)}"
    if which == "watch":
        return f"launch.{rng.choice(['watch', 'loopup'])} {x(key)}"
    if which == "stop":
        said = rng.choice(["taken over", "terminal attach ended: terminal t not found", "server is shutting down",
                           "herdr: not found", "terminal t already has an attached client; retry with --takeover",
                           text(rng, 0, 12)])
        if rng.random() < 0.5:
            return f"launch.stopkind {rng.choice([0, 1, 126, 127, 137])} {x(said)} {rng.randint(0, 1)} {rng.randint(0, 1)}"
        record = rng.choice(["1 term_ab", "137 t", "x", "", "1"])
        return f"launch.stop {x(rng.choice(['', text(rng, 0, 3)]) + 'MOSHPIT_RENDERER_STOPPED' + chr(10) + record + chr(10) + said + chr(10))}"
    if which == "landed":
        # What echo prints: the marker and a newline (a combining mark right
        # after it is no output of ours, and Swift's String search would
        # read the marker's last letter as part of its own character).
        said = rng.choice(["MOSHPIT_RETARGETED", "MOSHPIT_ATTACHED", "MOSHPIT_", ""]) + rng.choice(["", "\n", " x\n"])
        return f"launch.{rng.choice(['landed', 'up'])} {x(rng.choice(['', text(rng, 0, 4)]) + said)}"
    takeover = rng.randint(0, 1)
    if rng.random() < 0.5:
        return f"launch.retarget {takeover} {x(text(rng, 0, 10, structural=0.4))} {x(key)} {opt(herdr_path(rng))}"
    return f"launch.retarget {takeover} {x(text(rng, 0, 10, structural=0.4))} {x(key)}"


def family_control(rng):
    kind = rng.choice(["parse", "refusal", "notup", "errmsg", "first", "validate", "repolines", "cmd", "cmd", "poll"])
    if kind == "parse":
        raw = text(rng, 0, 20) + rng.choice(["", "__moshpit_rc=", "__moshpit_rc=0", "__moshpit_rc=1\n", "__moshpit_rc=x"]) \
            + rng.choice(["", str(rng.randint(0, 300)), text(rng, 0, 5)]) + rng.choice(["", "\n", "__moshpit_rc=2"])
        return f"ctl.parse {x(raw)}"
    if kind in ("refusal", "notup"):
        output, code = command_result(rng)
        if kind == "notup" and code == "none":
            code = "1"
        return f"ctl.{kind} {x(output)} {code}"
    if kind == "errmsg":
        return f"ctl.errmsg {x(command_result(rng)[0])}"
    if kind == "first":
        return f"ctl.first {x(text(rng, 0, 20, structural=0.3))} {x(rng.choice(['w4:p1', word(rng)]))} {opt(rng.choice([None, version(rng)]))}"
    if kind == "validate":
        branch = rng.choice([text(rng, 0, 12, structural=0.3), word(rng), " " + word(rng) + " ", word(rng) + ".lock",
                             "-" + word(rng), word(rng) + "/", word(rng) + ".." + word(rng)])
        return f"ctl.validate {x(rng.choice(['/r', '', '  ', '/' + word(rng)]))} {x(branch)} {x(rng.choice(['claude', '', ' ']))}"
    if kind == "repolines":
        lines = [rng.choice(["/" + text(rng, 0, 12, controls=0), text(rng, 0, 12), "  /" + word(rng) + "  ", ""]) for _ in range(rng.randint(0, 6))]
        return f"ctl.repolines {x(rng.choice(['\n', '\r\n']).join(lines))}"
    if kind == "poll":
        outputs = []
        for _ in range(rng.randint(1, 6)):
            roll = rng.random()
            if roll < 0.5:
                out = snapshot_json(rng)
            elif roll < 0.6:
                out = '{"error":{"code":"protocol_mismatch","message":%s}}' % json_string(rng, text(rng, 0, 30))
            elif roll < 0.7:
                out = '{"error":{"code":"server_not_running"}}'
            else:
                out = text(rng, 0, 20)
            out += rng.choice(["", "\n__moshpit_rc=0", "\n__moshpit_rc=1", "__moshpit_rc=2"])
            outputs.append(out)
        if rng.random() < 0.3:
            outputs = [outputs[0]] * rng.randint(3, 6) + outputs
        status = rng.choice([None, status_text(rng) + "__moshpit_rc=0"])
        return f"ctl.poll {opt(status)} " + " ".join(x(o) for o in outputs)
    tree = snapshot_for_commands(rng)
    path = opt(herdr_path(rng))
    ident = lambda: rng.choice(["w1", "w2", "w1:t1", "w1:p1", "w1:p2", word(rng), text(rng, 0, 8, structural=0.4)])
    name = lambda: rng.choice([None, "", text(rng, 0, 10, structural=0.4)])
    action = rng.choice([
        f"sel-session {x(ident())}", f"sel-window {x(ident())}", f"sel-pane {x(ident())}", f"sel-pane-imm {x(ident())}",
        f"kill-session {x(ident())}", f"kill-window {x(ident())}", f"kill-pane {x(ident())}",
        f"ren-session {x(ident())} {x(text(rng, 0, 10, structural=0.4))}", f"ren-window {x(ident())} {x(text(rng, 0, 10))}",
        f"new-session {opt(name())}", f"new-window {opt(name())}", "new-pane", f"new-window-in {x(ident())} {opt(name())}",
        f"new-pane-in {x(ident())}", f"keys {x(ident())} {x(','.join(rng.choice(['up', 'down', 'left', 'right', 'Enter', word(rng)]) for _ in range(rng.randint(0, 3))))}",
        f"remove {x(ident())} {rng.randint(0, 1)}",
        f"manifests {x(manifest(rng))}",
    ])
    return f"ctl.cmd {x(tree)} {path} {action}"


def manifest(rng):
    names = [json_string(rng, rng.choice(["claude", "codex", "pi", "gemini", word(rng), text(rng, 0, 6)])) for _ in range(rng.randint(0, 4))]
    items = ",".join('{"agent":%s}' % n if rng.random() < 0.9 else '{"name":%s}' % n for n in names)
    raw = text(rng, 0, 6, structural=0) + "[" + items + "]" + text(rng, 0, 6, structural=0)
    # A `]` before the first `[` traps the Swift client (divergence
    # manifest-range-trap): never in a batch, see its own fixture.
    start, end = raw.find("["), raw.rfind("]")
    if start != -1 and end != -1 and end < start:
        raw = raw.replace("]", "")
    return raw


FAMILIES = [family_json, family_firstobj, family_frames, family_line, family_snapshot, family_socket,
            family_versions, family_commands, family_control]


def cases(seed, n):
    rng = random.Random(seed)
    out = []
    for family in FAMILIES:
        for _ in range(n):
            out.append(family(rng))
    return out


# MARK: tags

def _hazards():
    ranges = []
    with open(os.path.join(HERE, "unicode-hazards.txt"), encoding="utf-8") as f:
        for line in f:
            if line.startswith("#"):
                continue
            for token in line.split():
                low, _, high = token.partition("-")
                ranges.append((int(low, 16), int(high or low, 16)))
    return ranges


HAZARD_RANGES = _hazards()


def is_hazard(ch):
    code = ord(ch)
    return any(low <= code <= high for low, high in HAZARD_RANGES)


NUMBER = re.compile(rb"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?")
JSON_STRING = re.compile(rb'"(?:[^"\\]|\\.)*"')
BASE64 = re.compile(r"[A-Za-z0-9+/=]*")
# A base64 value however the quotes around it pair up: the alphabet and `=`,
# with JSON's escaped slash.
BASE64_RUN = re.compile(rb"(?:[A-Za-z0-9+/=]|\\/)+")


def decimal_literal(token):
    """Whether NSJSONSerialization reads this number as an NSDecimalNumber."""
    text_ = token.decode()
    body = re.match(r"-?([0-9]+)(?:\.([0-9]+))?(?:[eE]([+-]?[0-9]+))?$", text_)
    if not body:
        return False
    integer, fraction, exp = body.group(1), body.group(2), body.group(3)
    if fraction is None and exp is None:
        value = int(text_)
        return not (-(2**63) <= value < 2**64)
    counted = (0 if integer == "0" else len(integer)) + len(fraction or "")
    return counted > 17


def foundation_b64_differs(value):
    """Whether Foundation's decoder accepts `value` where ours refuses it."""
    body = value.rstrip(b"=")
    if not body:
        return len(value) >= 4
    if b"=" in body and len(body) % 4 == 0:
        tail = body[-4:]
        return tail[2:3] == b"=" and b"=" not in tail[:2] + tail[3:] and b"=" not in body[:-4]
    return False


ESCAPE = re.compile(rb"\\u([0-9a-fA-F]{4})(?:\\u([0-9a-fA-F]{4}))?")


def escaped_scalars(raw):
    """The scalars JSON `\\uXXXX` escapes in `raw` stand for (pairs joined),
    wherever they are: a hazard hidden behind an escape is still one."""
    for match in ESCAPE.finditer(raw):
        first = int(match.group(1), 16)
        second = int(match.group(2), 16) if match.group(2) else None
        if 0xD800 <= first < 0xDC00 and second is not None and 0xDC00 <= second < 0xE000:
            yield chr(0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00))
            continue
        if not 0xD800 <= first < 0xE000:
            yield chr(first)
        if second is not None and not 0xD800 <= second < 0xE000:
            yield chr(second)


def json_strings(raw):
    """The JSON string literals in `raw`, decoded (those that decode)."""
    for match in JSON_STRING.finditer(raw):
        try:
            value = json.loads(match.group(0).decode("utf-8"))
        except ValueError:
            continue
        if isinstance(value, str):
            yield value


def tags(case):
    kind, *args = case.split(" ")
    raw = b"".join(bytes.fromhex(a[2:]) for a in args if a.startswith("x:"))
    decoded = raw.decode("utf-8", "replace")
    found = set()
    if any(is_hazard(ch) for ch in decoded) or any(is_hazard(ch) for ch in escaped_scalars(raw)):
        found.add("unicode")
    if "\ufeff" in decoded or b"\\ufeff" in raw.lower():
        found.add("json-bom")
    if b"{\x00" in raw or b"[\x00" in raw:
        found.add("json-encoding-sniff")
    if any(decimal_literal(m.group(0)) for m in NUMBER.finditer(raw)):
        found.add("json-decimal")
    runs = [m.group(0).replace(b"\\/", b"/") for m in BASE64_RUN.finditer(raw)]
    if any(foundation_b64_differs(run) for run in runs) or any(
            foundation_b64_differs(value.encode()) for value in json_strings(raw) if BASE64.fullmatch(value)):
        found.add("base64-quirks")
    if kind in ("start", "boot", "retarget", "resize", "scroll"):
        numbers = [int(a) for a in args if re.fullmatch(r"-?[0-9]+", a)]
        if kind == "scroll":
            numbers = numbers[1:]
        if any(not 1 <= n <= 65535 for n in numbers):
            found.add("size-clamp")
    if kind == "launch.retarget":
        found.add("retarget-unseen-refusal")
    if kind == "paste" and "\x1b[201~" in decoded.replace("\x1b[201~", ""):
        found.add("paste-end-marker")
    if kind == "ctl.refusal" and args[-1] == "-":
        found.add("B90")
    if re.search(rb"[0-9]{19,}", raw) and kind in ("vcompare", "status", "ctl.first", "ctl.poll"):
        found.add("version-digits")
    return sorted(found)
