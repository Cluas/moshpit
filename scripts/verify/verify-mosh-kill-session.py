#!/usr/bin/env python3
"""B583 on a real mosh: does killing the renderer's tmux session end mosh?

The renderer is mosh-server's program, `exec tmux attach` under `sh -c`, as
crates/session/src/attempt.rs starts it. This rig runs a real mosh-server
and mosh-client on 127.0.0.1 (the client on a pty with a size: a 0x0 pty
takes mosh-server down) against a private tmux socket, then kills the
renderer's session the way the sidecar's `kill_session` does.

  python3 -I scripts/verify/verify-mosh-kill-session.py               # today's order
  python3 -I scripts/verify/verify-mosh-kill-session.py --move-first  # B583's order

Before B583 (kill only): "[exited][mosh is exiting.]", mosh-server gone.
With the renderer moved first: mosh keeps running, on the other session.
Exits 0 when mosh survived. Needs mosh-server, mosh-client and tmux.
"""
import fcntl
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import time

SOCKET = f"moshpit-b583-{os.getpid()}"
ENV = dict(os.environ, LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8")
ENV.pop("TMUX", None)
ENV.pop("TMUX_PANE", None)


def tmux(*args):
    return subprocess.run(
        ["tmux", "-L", SOCKET, "-f", "/dev/null", *args],
        capture_output=True, text=True, env=ENV,
    ).stdout.strip()


def alive(pid):
    return subprocess.run(["ps", "-p", str(pid)], capture_output=True).returncode == 0


def drain(fd, seconds):
    end, buf = time.time() + seconds, b""
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if ready:
            try:
                buf += os.read(fd, 65536)
            except OSError:
                break
    return buf


def main():
    move_first = "--move-first" in sys.argv
    tmux("new-session", "-d", "-s", "main", "-x", "80", "-y", "24")
    tmux("new-session", "-d", "-s", "dev", "-x", "80", "-y", "24")
    started = subprocess.run(
        ["mosh-server", "new", "-i", "127.0.0.1", "-p", "61200:61299", "--",
         "/bin/sh", "-c", f"exec tmux -L {SOCKET} -f /dev/null attach -t '=main'"],
        capture_output=True, text=True, env=ENV,
    )
    said = started.stdout + started.stderr
    port, key = re.search(r"MOSH CONNECT (\d+) (\S+)", said).groups()
    server = int(re.search(r"pid = (\d+)", said).group(1))
    client, fd = pty.fork()
    if client == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        os.environ.update(ENV, MOSH_KEY=key, TERM="xterm-256color")
        os.execvp("mosh-client", ["mosh-client", "127.0.0.1", port])
    reaped = False
    try:
        drain(fd, 5)
        rows = tmux("list-clients", "-F", "#{client_control_mode}|#{client_name}|#{session_name}")
        renderer = [r.split("|")[1] for r in rows.splitlines() if r.startswith("0|")]
        if not renderer:
            print("the renderer never attached:", rows or "(no clients)")
            return 2
        print("renderer:", renderer[0], "on main")
        if move_first:
            tmux("switch-client", "-c", renderer[0], "-t", "=dev")
        tmux("kill-session", "-t", "=main")
        tail = drain(fd, 4)
        ended = os.waitpid(client, os.WNOHANG)[0] != 0
        reaped = ended
        text = re.sub(rb"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\r", b"", tail)
        print("mosh-client ended:", ended, "| mosh-server alive:", alive(server))
        print("clients now:", tmux("list-clients", "-F", "#{client_name} #{session_name}") or "(none)")
        if b"mosh is exiting" in text:
            print("the client said:", text.decode(errors="replace").strip()[-80:])
        survived = not ended and alive(server)
        print("verdict:", "mosh survived" if survived else "mosh ended")
        return 0 if survived else 1
    finally:
        tmux("kill-server")
        if not reaped:
            os.kill(client, 9)
            os.waitpid(client, 0)
        if alive(server):
            os.kill(server, 15)


if __name__ == "__main__":
    sys.exit(main())
