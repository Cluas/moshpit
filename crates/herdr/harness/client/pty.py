#!/usr/bin/env python3
"""A shell on a terminal, for the live runs (examples/live_herdr.rs).

    pty.py <program> [<arg>...]

Runs the program on a fresh 80x24 PTY. What arrives on stdin is typed into
the terminal, what the terminal prints comes out on stdout: the shape of the
SSH terminal channel the app's frame channel (and push pump) run on, line
discipline and echo included. Exits when the program does; EOF on stdin
hangs the terminal up.
"""
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios


def main():
    argv = sys.argv[1:] or ["/bin/sh"]
    pid, fd = pty.fork()
    if pid == 0:
        os.execvp(argv[0], argv)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    reading_stdin = True
    while True:
        watched = [fd] + ([0] if reading_stdin else [])
        ready, _, _ = select.select(watched, [], [])
        if fd in ready:
            try:
                data = os.read(fd, 65536)
            except OSError:
                data = b""
            if not data:
                break
            os.write(1, data)
        if reading_stdin and 0 in ready:
            data = os.read(0, 65536)
            if not data:
                reading_stdin = False
                os.kill(pid, signal.SIGHUP)
                continue
            while data:
                written = os.write(fd, data)
                data = data[written:]
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass


if __name__ == "__main__":
    main()
