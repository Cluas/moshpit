#!/usr/bin/env python3
"""Adds tmux's reading of a recording to it: the recorded bytes, up to a
cut, are written raw into a pane of a private tmux server, and that pane's
`capture-pane` and cursor become a reference line in the fixture:

    {"ref": [rows], "cursor": [x, y, shown], "at": N}

N is how many events of the recording come before the cut (the harness
feeds those and compares). tmux is the reference terminal here, as it is
for the live tmux recordings' last screen.

    tmux_ref.py FIXTURE [--before REGEX]

--before: cut before the first chunk whose text (escape sequences left
out) matches REGEX; default: after the whole recording.
"""
import argparse, base64, json, os, re, shutil, subprocess, sys, tempfile, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from drive import plain  # noqa: E402


def main():
    p = argparse.ArgumentParser()
    p.add_argument("fixture")
    p.add_argument("--before", default=None)
    a = p.parse_args()
    lines = open(a.fixture).read().splitlines()
    head = json.loads(lines[0])
    events = [json.loads(l) for l in lines[1:]]
    body = [e for e in events if "o" in e or "k" in e]
    cut = len(body)
    if a.before:
        for i, e in enumerate(body):
            if "o" in e and re.search(a.before.encode(), plain(base64.b64decode(e["o"]))):
                cut = i
                break
    data = b"".join(base64.b64decode(e["o"]) for e in body[:cut] if "o" in e)

    tmp = tempfile.mkdtemp(prefix="clilab-ref-")
    try:
        f = os.path.join(tmp, "bytes")
        open(f, "wb").write(data)
        t = shutil.which("tmux")
        env = {"TMUX_TMPDIR": tmp, "PATH": os.environ["PATH"], "HOME": tmp}
        tm = lambda *args: subprocess.run([t, "-L", "clilab-ref", "-f", "/dev/null", *args], env=env,
                                          capture_output=True, text=True).stdout
        # Raw, no echo: no output processing (the bytes are what a terminal
        # got), and the answers tmux gives to queries in them are not echoed. Then
        # stay: the pane must outlive the capture.
        cmd = f"stty raw -echo; cat {f}; {t} -L clilab-ref -f /dev/null wait-for -S done; exec sleep 600"
        tm("new-session", "-d", "-s", "ref", "-x", str(head["cols"]), "-y", str(head["rows"]), cmd)
        tm("set", "-g", "status", "off")
        tm("wait-for", "done")
        time.sleep(0.2)
        rows = tm("capture-pane", "-p", "-t", "ref").split("\n")[: head["rows"]]
        cur = [int(x) for x in tm("display", "-p", "-t", "ref", "#{cursor_x},#{cursor_y},#{cursor_flag}").split(",")]
        tm("kill-server")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    # One reference per cut: replace an earlier one at the same place.
    keep = [l for l in lines if not ('"ref"' in l and json.loads(l).get("at") == cut)]
    keep.append(json.dumps({"ref": rows, "cursor": cur, "at": cut}, ensure_ascii=False))
    open(a.fixture, "w").write("\n".join(keep) + "\n")
    print(f"{a.fixture}: ref at {cut} of {len(body)} events")


if __name__ == "__main__":
    main()
