# mosh-live herdr-retarget's `gone` (run.sh herdr): another client takes a
# terminal over — `herdr --session <private> terminal attach <tid>
# --takeover` on a PTY of its own, 80x24 — for a while, then goes away.
# Refuses a herdr without a private session.
#   python3 -I herdr-evict.py <seconds> <herdr argv...> <terminal id>
import os, pty, sys, time, select
seconds = float(sys.argv[1])
argv = sys.argv[2:-1] + ["terminal", "attach", sys.argv[-1], "--takeover"]
assert "--session" in argv, "refused: not a private session"
pid, fd = pty.fork()
if pid == 0:
    import fcntl, termios, struct
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    os.execv(argv[0], argv)
end = time.time() + seconds
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.05)
    if r:
        try:
            os.read(fd, 65536)
        except OSError:
            break
os.kill(pid, 9)
os.waitpid(pid, 0)
print("evicted")
