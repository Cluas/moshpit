#!/usr/bin/env python3
"""Turns the live herdr 0.9.5 recordings (tests/fixtures/recorded/, made by
harness/record.py) into harness cases: tests/fixtures/corpus/recorded.txt.

    recorded_cases.py <out>

The frame stream under several cuts and line by line (with herdr's stderr
line, which the app's PTY merges in), every CLI reply read as a snapshot,
a refusal, an agent-not-up check and an error message, the status reports,
the manifests through the real control client, and the socket conversation
with its subscribe pending.
"""
import json
import os
import sys

from corpus import TWO_WORKSPACES, x

HERE = os.path.dirname(os.path.abspath(__file__))
RECORDED = os.path.join(os.path.dirname(HERE), "tests", "fixtures", "recorded")


def read(name, mode="r"):
    with open(os.path.join(RECORDED, name), mode) as f:
        return f.read()


def main():
    cases = []
    frames = read("frames.jsonl", "rb")
    stderr = read("frames.stderr", "rb")
    for plan in ("whole", "rand:2:64", "rand:3:7"):
        cases.append(f"frames {plan} {x(frames)}")
    # The PTY merges stderr in: herdr's `input ignored` lands between frames.
    lines = frames.split(b"\n")
    merged = b"\n".join(lines[:20] + [stderr.rstrip(b"\n")] + lines[20:])
    cases.append(f"frames rand:4:300 {x(merged)}")
    for line in lines + [stderr.rstrip(b"\n")]:
        if line:
            cases.append(f"line {x(line)}")
    for raw in read("replies.jsonl").splitlines():
        reply = json.loads(raw)
        output, code = reply["output"], str(reply["exit"])
        trailer = f"{output}\n__moshpit_rc={code}"
        if reply["command"] == "api snapshot":
            cases.append(f"snapshot {x(output)}")
            cases.append(f"ctl.poll - {x(trailer)}")
            continue
        cases.append(f"ctl.parse {x(trailer)}")
        cases.append(f"ctl.refusal {x(output)} {code}")
        cases.append(f"ctl.notup {x(output)} {code}")
        cases.append(f"firstobj {x(output)}")
        cases.append(f"ctl.errmsg {x(output)}")
        if "{" in output:
            # A lost exit status with herdr's JSON there reads the same on
            # both sides; without it the port refuses (fix B90).
            cases.append(f"ctl.refusal {x(output)} -")
        if "error" in output:
            for kind in ("notrunning", "mismatch"):
                cases.append(f"{kind} {x(output)}")
        if "agent-manifests" in reply["command"]:
            cases.append(f"ctl.cmd {x(TWO_WORKSPACES)} - manifests {x(output)}")
        if reply["command"] == "status":
            cases.append(f"status {x(output)}")
    for name in ("status-running.txt", "status-stopped.txt"):
        cases.append(f"status {x(read(name))}")
    socket = read("socket.jsonl", "rb")
    for plan in ("whole", "rand:5:50", "bytes"):
        cases.append(f"socket {plan} 1 {x(socket)}")
        cases.append(f"socket {plan} 3 {x(socket)}")
    with open(sys.argv[1], "w", encoding="utf-8") as f:
        f.write("\n".join(cases) + "\n")
    print(f"{len(cases)} recorded cases", file=sys.stderr)


if __name__ == "__main__":
    main()
