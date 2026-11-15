#!/usr/bin/env python3
r"""B575: herdr's own TUI client forwards a split palette reply into the
focused pane as typing (an upstream herdr issue, not Moshpit's).

This plays the herdr client's host terminal: it answers the client's
colour queries (OSC 10/11 and all 256 `OSC 4;n;?`), cuts one palette reply
right after its ESC, and sends the rest PAUSE seconds later. With a pause
of 0.3 s the focused pane's `cat -v` gets `^[]4;191;rgb:d7d7/ffff/5f5f^[\`
(in Claude Code: an Escape, then `]4;191;rgb:d7d7/ffff/5f5f` typed); 0.05 s
and 0.2 s don't leak.

Run it against an ISOLATED herdr only (never the user's server):

    export HOME=/tmp/h XDG_CONFIG_HOME=/tmp/h/.config HERDR_SOCKET_PATH=/tmp/h/h.sock \
           HERDR_CONFIG_PATH=/tmp/h/config.toml SSH_CONNECTION="127.0.0.1 1 127.0.0.1 22"
    herdr server &                     # SSH_CONNECTION: no launchd job
    herdr workspace create --cwd /tmp/h --focus     # its pane, say w1:p1
    herdr pane run w1:p1 'exec cat -v'
    python3 -I herdr-palette-split.py w1:p1 191 0.3
    herdr server stop

Usage: herdr-palette-split.py PANE INDEX PAUSE
"""
import os, pty, re, select, subprocess, sys, time

env = {k: v for k, v in os.environ.items() if not (k.startswith("HERDR_") and k not in ("HERDR_SOCKET_PATH", "HERDR_CONFIG_PATH", "HERDR_DISABLE_SOUND"))}
env.pop("TMUX", None)
env["TERM"] = "xterm-256color"
PANE = sys.argv[1]
SPLIT_INDEX = int(sys.argv[2]) if len(sys.argv) > 2 else 191
PAUSE = float(sys.argv[3]) if len(sys.argv) > 3 else 0.05

pid, fd = pty.fork()
if pid == 0:
    os.execvpe("herdr", ["herdr"], env)

import fcntl, termios, struct
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))

buf = b""
deadline = time.time() + 6
answered = set()
default_answered = False
while time.time() < deadline:
    r, _, _ = select.select([fd], [], [], 0.1)
    if not r:
        continue
    try:
        data = os.read(fd, 65536)
    except OSError:
        break
    buf += data
    queries = re.findall(rb"\x1b\]4;(\d+);\?(?:\x1b\\|\x07)", buf)
    if not default_answered and (b"\x1b]11;?" in buf or b"\x1b]10;?" in buf):
        os.write(fd, b"\x1b]10;rgb:cccc/cccc/cccc\x1b\\\x1b]11;rgb:0000/0000/0000\x1b\\")
        default_answered = True
    new = [int(q) for q in queries if int(q) not in answered]
    if new:
        out = b""
        for n in new:
            answered.add(n)
            reply = b"\x1b]4;%d;rgb:d7d7/ffff/5f5f\x1b\\" % n
            if n == SPLIT_INDEX:
                # The transport cut it right after its introducer's ESC.
                os.write(fd, out + reply[:1])
                time.sleep(PAUSE)
                out = reply[1:]
            else:
                out += reply
        os.write(fd, out)
# A marker after the replies: typing reaches the pane (the client is in it).
os.write(fd, b"hello")
time.sleep(1)
print("palette queries answered:", len(answered), "default colours:", default_answered)
os.kill(pid, 15)
time.sleep(0.5)
read = subprocess.run(["herdr", "pane", "read", PANE], capture_output=True, text=True, env=env)
print("pane shows:", read.stdout[-400:])
