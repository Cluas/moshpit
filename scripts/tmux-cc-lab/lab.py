#!/usr/bin/env python3
"""tmux -CC lab: deterministic experiments + fixture recorder for Moshpit's
SSH+tmux (-CC) renderer.

Runs a PRIVATE tmux server (-L moshpit-lab-<pid>, -f lab.conf) — it never
touches the user's default tmux server or sessions. Three actors:

  cc       a real `tmux -CC attach` on its own pty (what the app is),
           recorded byte-for-byte; commands are written to its stdin the
           same way TmuxSessionController sends them
  desktop  a regular `tmux attach` client on a second pty with a different
           window size — the "size war" enemy; "activity" (a keypress)
           makes it win `window-size latest`
  ext      one-shot `tmux -L <sock> <cmd>` invocations (neutral driver)

Every scenario writes into an output dir:
  stream.bin     raw bytes the -CC client received
  stream.jsonl   the same, chunked with monotonic timestamps
  notes.jsonl    timestamped scenario steps (aligns with stream.jsonl)
  captures/*.txt capture-pane ground truths taken along the way
  report.txt     the built-in analysis (same detectors as `analyze`)

Usage:
  lab.py copymode|wheel|sizewar|capturerace [--out DIR]
  lab.py analyze <stream.bin|cctap.bin> ...
"""
import argparse
import base64
import fcntl
import json
import os
import pty
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import termios
import threading
import time
from collections import Counter, defaultdict

TMUX = os.environ.get("LAB_TMUX") or shutil.which("tmux") or "/opt/homebrew/bin/tmux"
HERE = os.path.dirname(os.path.abspath(__file__))
FAKE_TUI = os.path.join(HERE, "fake_tui.py")

LAB_CONF = """
set -g window-size latest
set -g mouse on
set -g history-limit 50000
set -g escape-time 0
set -g status on
"""


def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class PtyClient:
    """A tmux client (control or regular) on its own pty, fully recorded."""

    def __init__(self, lab, args, rows, cols, tag):
        self.lab = lab
        self.tag = tag
        self.master, slave = pty.openpty()
        set_winsize(self.master, rows, cols)
        env = dict(os.environ, TERM="xterm-256color")

        def child_setup():
            os.setsid()
            fcntl.ioctl(0, getattr(termios, "TIOCSCTTY", 0x20007461), 0)

        self.proc = subprocess.Popen(
            [TMUX, "-L", lab.sock] + args,
            stdin=slave, stdout=slave, stderr=slave,
            env=env, preexec_fn=child_setup)
        os.close(slave)
        self.chunks = []  # (t, bytes)
        self._stop = False
        self.thread = threading.Thread(target=self._pump, daemon=True)
        self.thread.start()

    def _pump(self):
        while not self._stop:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                break
            if not data:
                break
            self.chunks.append((self.lab.now(), data))

    def cmd(self, line):
        """Write a command line to the client's stdin (control mode) or
        keystrokes (regular client)."""
        self.lab.note(f"{self.tag}-stdin", line)
        os.write(self.master, line.encode() + b"\n")

    def keys(self, data: bytes):
        self.lab.note(f"{self.tag}-keys", repr(data))
        os.write(self.master, data)

    def resize(self, rows, cols):
        self.lab.note(f"{self.tag}-resize", f"{cols}x{rows}")
        set_winsize(self.master, rows, cols)
        # TIOCSWINSZ on the pty delivers SIGWINCH to the client's foreground
        # process group; nothing else needed.

    def close(self):
        self._stop = True
        try:
            self.proc.terminate()
        except ProcessLookupError:
            pass
        try:
            os.close(self.master)
        except OSError:
            pass


class Lab:
    def __init__(self, outdir):
        self.sock = f"moshpit-lab-{os.getpid()}"
        self.outdir = outdir
        os.makedirs(os.path.join(outdir, "captures"), exist_ok=True)
        self.conf = os.path.join(outdir, "lab.conf")
        with open(self.conf, "w") as f:
            f.write(LAB_CONF)
        self.t0 = time.monotonic()
        self.notes = []
        self.clients = []
        self.cc = None
        self.finished = False

    def now(self):
        return round(time.monotonic() - self.t0, 4)

    def note(self, kind, text):
        entry = {"t": self.now(), "kind": kind, "note": text}
        self.notes.append(entry)
        print(f"  [{entry['t']:8.3f}] {kind}: {text}")

    def ext(self, *cmd, quiet=False):
        if not quiet:
            self.note("ext", " ".join(cmd))
        try:
            r = subprocess.run([TMUX, "-L", self.sock] + list(cmd),
                               capture_output=True, text=True, timeout=5)
        except subprocess.TimeoutExpired:
            self.note("ext-TIMEOUT", " ".join(cmd))
            return ""
        if r.returncode != 0 and not quiet:
            self.note("ext-err", r.stderr.strip())
        return r.stdout

    def start_server(self, cols, rows, command):
        self.ext("-f", self.conf, "new-session", "-d", "-s", "lab",
                 "-x", str(cols), "-y", str(rows), command)

    def attach_cc(self, pin_cols, pin_rows):
        cc = PtyClient(self, ["-CC", "attach", "-t", "lab"], 24, 80, "cc")
        self.clients.append(cc)
        self.cc = cc
        time.sleep(0.6)
        # Pin exactly the way TmuxSessionController does.
        cc.cmd(f"refresh-client -C {pin_cols}x{pin_rows}")
        cc.cmd(f"resize-window -t @0 -x {pin_cols} -y {pin_rows}")
        time.sleep(0.4)
        return cc

    def attach_desktop(self, rows, cols):
        d = PtyClient(self, ["attach", "-t", "lab"], rows, cols, "desktop")
        self.clients.append(d)
        time.sleep(0.5)
        return d

    def capture(self, name, *extra):
        text = self.ext("capture-pane", "-p", "-e", "-t", "%0", *extra)
        path = os.path.join(self.outdir, "captures", name + ".txt")
        with open(path, "w") as f:
            f.write(text)
        self.note("capture", f"{name} ({len(text.splitlines())} lines)")
        return text

    def finish(self, cc):
        if self.finished:
            return
        self.finished = True
        time.sleep(0.5)
        for c in self.clients:
            c.close()
        subprocess.run([TMUX, "-L", self.sock, "kill-server"],
                       capture_output=True)
        raw = b"".join(d for _, d in cc.chunks)
        with open(os.path.join(self.outdir, "stream.bin"), "wb") as f:
            f.write(raw)
        with open(os.path.join(self.outdir, "stream.jsonl"), "w") as f:
            for t, d in cc.chunks:
                f.write(json.dumps({"t": t, "hex": d.hex()}) + "\n")
        with open(os.path.join(self.outdir, "notes.jsonl"), "w") as f:
            for n in self.notes:
                f.write(json.dumps(n) + "\n")
        report = analyze_stream(raw, notes=self.notes, chunks=cc.chunks)
        with open(os.path.join(self.outdir, "report.txt"), "w") as f:
            f.write(report)
        print(f"\n=== {self.outdir}/report.txt ===")
        print(report)


# ---------------------------------------------------------------- scenarios

def wheel_bytes(up: bool, col=35, row=10) -> bytes:
    btn = 64 if up else 65
    return f"\x1b[<{btn};{col};{row}M".encode()


def send_wheel(cc, up: bool, count=1):
    """Mirror sendInput: send-keys -H <hex bytes> straight into the pane."""
    hexes = " ".join(f"{b:02x}" for b in wheel_bytes(up))
    for _ in range(count):
        cc.cmd(f"send-keys -t %0 -H {hexes}")


def scenario_copymode(lab):
    """Q: what does a -CC client see when copy-mode scrolls a shell pane?
    Q: what does capture-pane return while the pane is in copy-mode?"""
    lab.start_server(70, 20, "seq -f 'line-%03g-abcdefghijklmnop' 1 200; exec cat")
    cc = lab.attach_cc(70, 20)
    time.sleep(0.3)
    lab.capture("before")
    lab.note("step", "enter copy-mode via cc stdin (as the app does)")
    cc.cmd("copy-mode -t %0")
    time.sleep(0.3)
    cc.cmd("send-keys -t %0 -N 5 -X scroll-up")
    time.sleep(0.3)
    cc.cmd("send-keys -t %0 -N 5 -X scroll-up")
    time.sleep(0.4)
    lab.note("step", "capture WHILE scrolled up in copy-mode")
    lab.capture("during-copymode")
    print("   pane_in_mode/scroll_position: " +
          lab.ext("display-message", "-p", "-t", "%0",
                  "#{pane_in_mode} #{scroll_position}").strip())
    lab.note("step", "new output arrives while scrolled up")
    lab.ext("send-keys", "-t", "%0", "-l", "NEW-OUTPUT-WHILE-SCROLLED\r")
    time.sleep(0.4)
    lab.note("step", "scroll-down back + cancel")
    cc.cmd("send-keys -t %0 -N 10 -X scroll-down")
    time.sleep(0.3)
    cc.cmd("send-keys -t %0 -X cancel")
    time.sleep(0.4)
    lab.capture("after-cancel")
    lab.finish(cc)


def scenario_wheel(lab):
    """Q: what do wheel-driven TUI diff repaints look like in %output?"""
    lab.start_server(70, 20, f"{sys.executable} {FAKE_TUI}")
    cc = lab.attach_cc(70, 20)
    time.sleep(0.5)
    lab.capture("initial")
    for i in range(6):
        send_wheel(cc, up=False)
        time.sleep(0.25)
    lab.capture("after-wheel-down")
    for i in range(3):
        send_wheel(cc, up=True)
        time.sleep(0.25)
    time.sleep(0.3)
    lab.capture("final")
    lab.finish(cc)


def scenario_sizewar(lab):
    """The money shot: wheel-scroll a TUI while a desktop client flips the
    window size — record exact byte ordering of %layout-change vs %output."""
    lab.start_server(70, 20, f"{sys.executable} {FAKE_TUI}")
    cc = lab.attach_cc(70, 20)
    desktop = lab.attach_desktop(45, 180)
    time.sleep(0.5)
    lab.note("phase", "desktop attached; it may already have won the size")
    lab.capture("attach-settled")

    for round_no in range(3):
        lab.note("phase", f"round {round_no}: phone scrolls, then backgrounds")
        send_wheel(cc, up=False)
        time.sleep(0.15)
        # The app backgrounds: releaseWindowPin hands the window back
        # (`resize-window -x/-y` pins MANUAL size; only unsetting the
        # per-window option re-arms `window-size latest`).
        cc.cmd("set-option -u -w -t @0 window-size")
        time.sleep(0.1)
        # Desktop activity (typing into the pane app) wins `latest`,
        # and the keystrokes make the TUI repaint at the desktop grid.
        desktop.keys(b"xxx")
        time.sleep(0.3)
        lab.note("phase", f"round {round_no}: phone foregrounds, re-pins")
        cc.cmd("refresh-client -C 70x20")
        cc.cmd("resize-window -t @0 -x 70 -y 20")
        time.sleep(0.15)
        send_wheel(cc, up=False)
        time.sleep(0.35)

    lab.note("phase", "settle at phone size and capture ground truth")
    cc.cmd("resize-window -t @0 -x 70 -y 20")
    time.sleep(0.8)
    lab.capture("final-70x20")
    lab.finish(cc)


def scenario_capturerace(lab):
    """Q: how stale is a capture taken immediately at return-to-our-width
    (the settling problem) vs one taken 300ms later?"""
    lab.start_server(70, 20, f"{sys.executable} {FAKE_TUI}")
    cc = lab.attach_cc(70, 20)
    desktop = lab.attach_desktop(45, 180)
    time.sleep(0.5)
    cc.cmd("set-option -u -w -t @0 window-size")  # phone backgrounds
    time.sleep(0.1)
    desktop.keys(b"x")  # desktop wins `latest`; TUI repaints at 180
    time.sleep(0.5)
    lab.note("step", "re-pin then capture IMMEDIATELY, then at +300ms")
    cc.cmd("resize-window -t @0 -x 70 -y 20")
    lab.capture("immediate")
    time.sleep(0.3)
    lab.capture("plus300ms")
    time.sleep(0.5)
    lab.capture("plus800ms")
    lab.finish(cc)


# ----------------------------------------------------------------- analysis

OCT = re.compile(rb"\\([0-7]{3})")
CSI = re.compile(rb"\x1b\[([0-9;?]*)([A-Za-z@`])")
CHA = re.compile(rb"\x1b\[(\d+)G")
CUP = re.compile(rb"\x1b\[(\d+);(\d+)H")
CUF = re.compile(rb"\x1b\[(\d+)C")


def decode_cc(data: bytes) -> bytes:
    return OCT.sub(lambda m: bytes([int(m.group(1), 8)]), data)


def analyze_stream(raw: bytes, notes=None, chunks=None) -> str:
    lines = raw.split(b"\n")
    out = []
    layout = None  # (w, h)
    n_out = Counter()
    bytes_out = Counter()
    events = []
    in_block = None
    block_lines = 0
    for i, ln in enumerate(lines):
        ln = ln.rstrip(b"\r")
        if in_block is not None:
            if ln.startswith(b"%end") or ln.startswith(b"%error"):
                if ln.startswith(b"%error"):
                    events.append((i, f"ERROR num={in_block}: "
                                      f"{first_block_line[:80].decode(errors='replace')}"))
                elif block_lines > 3:
                    events.append((i, f"block num={in_block} lines={block_lines}"))
                in_block = None
            else:
                if block_lines == 0:
                    first_block_line = ln
                block_lines += 1
            continue
        if ln.startswith(b"%begin"):
            parts = ln.split()
            in_block = parts[2].decode() if len(parts) > 2 else "?"
            block_lines = 0
            first_block_line = b""
            continue
        if ln.startswith(b"%layout-change"):
            m = re.search(rb"(\d+)x(\d+),\d+,\d+", ln)
            if m:
                layout = (int(m.group(1)), int(m.group(2)))
                events.append((i, f"layout-change {layout[0]}x{layout[1]}"))
            continue
        if ln.startswith(b"%output "):
            parts = ln.split(b" ", 2)
            pane = parts[1].decode()
            payload = decode_cc(parts[2]) if len(parts) > 2 else b""
            n_out[pane] += 1
            bytes_out[pane] += len(payload)
            # %output event boundaries falling MID escape sequence: the tell
            # that any per-event gate (drop while foreign/backgrounded) can
            # paint half a sequence or swallow the resync that follows.
            if re.match(rb"^[0-9;:]+[A-Za-z@`~]", payload) or \
                    re.match(rb"^\[", payload):
                events.append((i, f"CUT-START pane={pane}: {payload[:40]!r}"))
            if re.search(rb"\x1b\[?[0-9;:]*$", payload):
                events.append((i, f"CUT-END pane={pane}: …{payload[-30:]!r}"))
            if layout:
                w, h = layout
                viol = []
                viol += [f"CHA{v}" for v in
                         (int(m.group(1)) for m in CHA.finditer(payload))
                         if v > w]
                viol += [f"CUF{v}" for v in
                         (int(m.group(1)) for m in CUF.finditer(payload))
                         if v > w]
                viol += [f"CUP{r};{c}" for r, c in
                         ((int(m.group(1)), int(m.group(2)))
                          for m in CUP.finditer(payload))
                         if c > w or r > h]
                if viol:
                    events.append(
                        (i, f"FOREIGN-COORD pane={pane} under {w}x{h}: "
                            f"{viol[:5]} payload[:80]={payload[:80]!r}"))
            continue
        if ln.startswith(b"%"):
            tag = ln.split()[0].decode()
            if tag not in ("%output",):
                events.append((i, ln[:110].decode(errors="replace")))
    out.append(f"outputs per pane: {dict(n_out)}  bytes: {dict(bytes_out)}")
    out.append("event timeline (protocol line numbers):")
    for i, e in events:
        out.append(f"  [{i:6}] {e}")
    return "\n".join(out) + "\n"


SCENARIOS = {
    "copymode": scenario_copymode,
    "wheel": scenario_wheel,
    "sizewar": scenario_sizewar,
    "capturerace": scenario_capturerace,
}


# ------------------------------------------------------------------- serve
#
# Full-stack e2e bridge: the SIMULATOR test runs the real
# TmuxSessionController + SwiftTerm against THIS real tmux server.
#
#   data port  — raw byte bridge to a `tmux -CC attach` pty. The Swift test
#                implements TmuxTransport over this socket; the controller
#                does everything it does in production (pin, gate, resync).
#   ctl port   — line-JSON driver for the parts the app can't reach:
#                {"cmd":"desktop-keys","keys":"xxx"}  desktop-client activity
#                {"cmd":"capture"}                    ground-truth grid (b64)
#                {"cmd":"layout"}                     current window size
#                {"cmd":"quit"}
#
# Started by scripts/tmux-cc-lab/run-e2e.sh, which passes the ports into the
# test process via TEST_RUNNER_MOSHPIT_TMUX_LAB_*.

def serve(data_port: int, ctl_port: int):
    outdir = os.path.join(
        os.environ.get("LAB_RUNS", os.path.join(HERE, "runs")),
        f"serve-{time.strftime('%m%d-%H%M%S')}")
    os.makedirs(os.path.join(outdir, "captures"), exist_ok=True)
    lab = Lab(outdir)
    lab.start_server(70, 20, f"{sys.executable} {FAKE_TUI}")
    desktop = lab.attach_desktop(45, 180)
    time.sleep(0.3)

    data_srv = socket.create_server(("127.0.0.1", data_port))
    ctl_srv = socket.create_server(("127.0.0.1", ctl_port))
    print(f"serve: data={data_port} ctl={ctl_port} sock={lab.sock}", flush=True)

    stop = threading.Event()

    def ctl_loop():
        while not stop.is_set():
            try:
                conn, _ = ctl_srv.accept()
            except OSError:
                return
            f = conn.makefile("rwb")
            for line in f:
                try:
                    req = json.loads(line)
                except json.JSONDecodeError:
                    continue
                cmd = req.get("cmd")
                if cmd == "desktop-keys":
                    desktop.keys(req.get("keys", "x").encode())
                    resp = {"ok": True}
                elif cmd == "capture":
                    text = lab.ext("capture-pane", "-p", "-t", "%0", quiet=True)
                    resp = {"ok": True,
                            "b64": base64.b64encode(text.encode()).decode()}
                elif cmd == "layout":
                    out = lab.ext("display-message", "-p", "-t", "%0",
                                  "#{window_width}x#{window_height}", quiet=True)
                    resp = {"ok": True, "layout": out.strip()}
                elif cmd == "quit":
                    resp = {"ok": True}
                    f.write((json.dumps(resp) + "\n").encode())
                    f.flush()
                    stop.set()
                    data_srv.close()
                    ctl_srv.close()
                    return
                else:
                    resp = {"ok": False, "err": f"unknown cmd {cmd!r}"}
                f.write((json.dumps(resp) + "\n").encode())
                f.flush()
            conn.close()

    threading.Thread(target=ctl_loop, daemon=True).start()

    try:
        while not stop.is_set():
            try:
                conn, _ = data_srv.accept()
            except OSError:
                break
            print("serve: cc client connected", flush=True)
            cc = PtyClient(lab, ["-CC", "attach", "-t", "lab"], 24, 80, "cc")
            tag = time.strftime("%H%M%S")
            to_pty_log = open(os.path.join(outdir, f"to-pty-{tag}.bin"), "wb")

            def sock_to_pty():
                while True:
                    try:
                        data = conn.recv(65536)
                    except OSError:
                        data = b""
                    if not data:
                        break
                    to_pty_log.write(data)
                    to_pty_log.flush()
                    # pty master buffers are small; never drop the tail of a
                    # command on a partial write.
                    view = memoryview(data)
                    while view:
                        try:
                            n = os.write(cc.master, view)
                        except OSError:
                            return
                        view = view[n:]

            t = threading.Thread(target=sock_to_pty, daemon=True)
            t.start()
            # pty → socket: reuse the recorder thread's chunks as they land.
            sent = 0
            alive = True
            while alive and t.is_alive() and not stop.is_set():
                chunks = cc.chunks
                if sent < len(chunks):
                    try:
                        conn.sendall(chunks[sent][1])
                        sent += 1
                    except OSError:
                        alive = False
                else:
                    time.sleep(0.005)
            with open(os.path.join(outdir, f"from-pty-{tag}.bin"), "wb") as f:
                f.write(b"".join(d for _, d in cc.chunks))
            to_pty_log.close()
            cc.close()
            conn.close()
            print("serve: cc client disconnected", flush=True)
    finally:
        subprocess.run([TMUX, "-L", lab.sock, "kill-server"], capture_output=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scenario", choices=list(SCENARIOS) + ["analyze", "serve"])
    ap.add_argument("paths", nargs="*")
    ap.add_argument("--out", default=None)
    ap.add_argument("--data-port", type=int, default=8765)
    ap.add_argument("--ctl-port", type=int, default=8766)
    args = ap.parse_args()
    if args.scenario == "analyze":
        for p in args.paths:
            print(f"=== {p} ===")
            print(analyze_stream(open(p, "rb").read()))
        return
    if args.scenario == "serve":
        serve(args.data_port, args.ctl_port)
        return
    outdir = args.out or os.path.join(
        os.environ.get("LAB_RUNS", os.path.join(HERE, "runs")),
        f"{args.scenario}-{time.strftime('%m%d-%H%M%S')}")
    os.makedirs(outdir, exist_ok=True)
    print(f"== scenario {args.scenario} -> {outdir}")
    lab = Lab(outdir)
    try:
        SCENARIOS[args.scenario](lab)
    finally:
        if lab.cc is not None and not lab.finished:
            try:
                lab.finish(lab.cc)
            except Exception as e:
                print(f"finish failed: {e}")
        subprocess.run([TMUX, "-L", lab.sock, "kill-server"],
                       capture_output=True)


if __name__ == "__main__":
    main()
