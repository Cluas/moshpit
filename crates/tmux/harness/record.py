#!/usr/bin/env python3
"""Records raw tmux -CC byte streams for the G2 parser equivalence harness.

Everything runs on a PRIVATE tmux server, `tmux -L moshpit-g2 -f /dev/null`:
never the user's default server, never their sessions or config. The server
is killed when the recording ends, also on failure.

A real `tmux -CC attach` runs on its own pty (what the app is, over SSH), and
every byte it writes is kept in order. The parent keeps the slave side open
until the client has exited and the master has gone quiet, so no trailing
bytes are lost at hangup; nothing sits between tmux and the recorder (a
script(1) pty in between once dropped bytes under load). Commands are written
to the client's stdin the way TmuxSessionController sends them, and each
capture starts with the app's own discovery commands.

    record.py <workload> <out.bin>
    record.py all <dir>          every committed workload into <dir>

Workloads: plain cjk-emoji ansi altscreen layout session-switch error
boot-chain pause burst (burst is seq 1 200000, recorded at harness time and
not committed).
"""
import fcntl
import os
import pty
import select
import shutil
import struct
import subprocess
import sys
import termios
import threading
import time

TMUX = shutil.which("tmux") or "/opt/homebrew/bin/tmux"
SOCKET = "moshpit-g2"
BASE = [TMUX, "-L", SOCKET, "-f", "/dev/null"]
WORKDIR = "/tmp/moshpit-g2-rec"


def child_env():
    env = {k: v for k, v in os.environ.items() if not k.startswith("TMUX")}
    env.update(
        TERM="xterm-256color",
        SHELL="/bin/sh",
        ENV="/dev/null",
        HISTFILE="/dev/null",
        LANG="en_US.UTF-8",
        LC_ALL="en_US.UTF-8",
    )
    return env


def tmux(*args, timeout=30, check=True):
    """One-shot command on the private server."""
    done = subprocess.run(BASE + list(args), env=child_env(), capture_output=True,
                          timeout=timeout, text=True)
    if check and done.returncode != 0:
        raise RuntimeError(f"tmux {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout


def server_running():
    return subprocess.run(BASE + ["has-session"], env=child_env(), capture_output=True).returncode == 0


def kill_server():
    subprocess.run(BASE + ["kill-server"], env=child_env(), capture_output=True)


def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class PtyProcess:
    """A process on its own pty; everything it writes is kept in order."""

    def __init__(self, argv, rows=24, cols=80):
        self.master, self.slave = pty.openpty()
        set_winsize(self.master, rows, cols)

        def child_setup():
            os.setsid()
            fcntl.ioctl(0, getattr(termios, "TIOCSCTTY", 0x20007461), 0)

        self.proc = subprocess.Popen(argv, stdin=self.slave, stdout=self.slave,
                                     stderr=self.slave, env=child_env(),
                                     preexec_fn=child_setup, close_fds=True)
        self.data = bytearray()
        self.lock = threading.Lock()
        self.reading = threading.Event()
        self.reading.set()
        self.stopped = False
        self.last_byte_at = time.monotonic()
        self.thread = threading.Thread(target=self._pump, daemon=True)
        self.thread.start()

    def _pump(self):
        while not self.stopped:
            if not self.reading.wait(0.05):
                continue
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if not ready:
                continue
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                break
            if not chunk:
                break
            with self.lock:
                self.data += chunk
                self.last_byte_at = time.monotonic()

    def write(self, data: bytes):
        os.write(self.master, data)

    def size(self):
        with self.lock:
            return len(self.data)

    def wait_for(self, needle: bytes, start=0, timeout=30.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                at = self.data.find(needle, start)
            if at >= 0:
                return at + len(needle)
            time.sleep(0.01)
        raise TimeoutError(f"no {needle!r} within {timeout}s")

    def quiet(self, idle=0.4, timeout=60.0):
        """Wait until nothing has arrived for `idle` seconds."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                since = time.monotonic() - self.last_byte_at
            if since >= idle:
                return
            time.sleep(0.05)
        raise TimeoutError("stream never went quiet")

    def stall(self, seconds):
        """Stop reading for a while: the client's output backs up in tmux."""
        self.reading.clear()
        time.sleep(seconds)
        self.reading.set()

    def finish(self, timeout=10.0):
        """Wait for the process to exit and the master to drain; return bytes."""
        try:
            self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.terminate()
            self.proc.wait(timeout=5)
        self.quiet(idle=0.5, timeout=timeout)
        self.stopped = True
        self.thread.join()
        os.close(self.slave)
        os.close(self.master)
        return bytes(self.data)


class ControlClient(PtyProcess):
    """`tmux -CC` on its own pty, driven the way the app drives it."""

    def __init__(self, argv):
        super().__init__(argv)
        self.syncs = 0

    def send(self, command: str):
        self.write(command.encode() + b"\n")

    def sync(self, timeout=60.0):
        """Round-trip a no-op command: every reply before it has arrived."""
        self.syncs += 1
        marker = f"G2SYNC{self.syncs}"
        self.send(f"display-message -p {marker}")
        self.wait_for(marker.encode() + b"\r\n", timeout=timeout)

    def type_line(self, pane: str, text: str):
        """Type a shell command line into a pane (no single quotes in text)."""
        assert "'" not in text, text
        self.send(f"send-keys -t {pane} -l '{text}'")
        self.send(f"send-keys -t {pane} Enter")

    def run(self, pane: str, text: str, idle=0.4):
        self.type_line(pane, text)
        self.sync()
        self.quiet(idle=idle)

    def run_signalled(self, pane: str, text: str, channel: str, timeout=120.0):
        """Run a long command; the pane signals `channel` when it is done."""
        self.type_line(pane, f"{text}; tmux -L {SOCKET} wait-for -S {channel}")
        tmux("wait-for", channel, timeout=timeout)
        self.quiet(idle=0.6, timeout=timeout)
        self.sync(timeout=timeout)

    def prologue(self):
        """What the app sends right after attaching."""
        self.send("refresh-client -C 80x24")
        self.send("list-sessions -F '#{session_id} #{session_attached} #{session_name}'")
        self.send("list-windows -a -F '#{session_id} #{window_id} #{window_index} "
                  "#{window_layout} #{window_active} #{window_panes} #{window_name}'")
        self.send("list-panes -a -F '#{pane_id} #{window_id} #{pane_index} #{pane_width} "
                  "#{pane_height} #{pane_active} #{alternate_on} #{pane_in_mode} "
                  "#{pane_current_command}'")
        self.send("show-options -gqv prefix")
        self.send("list-clients -F '#{client_control_mode}|#{client_session}|#{client_name}'")
        self.sync()

    def backfill(self, pane: str):
        """The app's resume path: scrollback above the screen, then the screen."""
        self.send(f"display-message -p -t {pane} '#{{alternate_on}} #{{pane_in_mode}}'")
        self.send(f"capture-pane -p -e -S -2000 -E -1 -t {pane}")
        self.send(f"capture-pane -p -e -t {pane}")
        self.send(f"display-message -p -t {pane} '#{{cursor_x}} #{{cursor_y}}'")
        self.sync()

    def detach(self):
        self.send("detach-client")
        return self.finish()


def write_script(name: str, body: str) -> str:
    path = os.path.join(WORKDIR, name)
    with open(path, "w", encoding="utf-8") as f:
        f.write(body)
    return path


def start_server():
    if server_running():
        raise RuntimeError(f"a tmux server already runs on -L {SOCKET}; kill it first")
    shutil.rmtree(WORKDIR, ignore_errors=True)
    os.makedirs(WORKDIR)
    # Panes run a non-login /bin/sh: no /etc/profile, so no host or user name
    # in the prompt (bash's default "sh-3.2$ ").
    tmux("new-session", "-d", "-s", "g2", "-x", "80", "-y", "24", "-c", WORKDIR, "/bin/sh")
    tmux("set-option", "-g", "default-shell", "/bin/sh")
    tmux("set-option", "-g", "default-command", "/bin/sh")
    tmux("set-option", "-g", "history-limit", "5000")
    time.sleep(0.3)


def attach():
    cc = ControlClient(BASE + ["-CC", "attach-session", "-t", "g2"])
    cc.wait_for(b"%end ")  # the attach command's own reply block
    cc.prologue()
    return cc


# MARK: - Workloads

def w_plain():
    cc = attach()
    cc.run("%0", 'printf "hello world\\n"')
    cc.run("%0", "echo plain text: the quick brown fox; echo second line")
    cc.run("%0", 'printf "%s\\n" one two three four five')
    cc.run("%0", 'echo $((6 * 7)); printf "tab:[\\t] backslash:[\\\\] percent:[100%%]\\n"')
    # Pane text shaped like protocol lines: content, never control.
    cc.run("%0", "echo %begin 1 2 0 and %end 1 2 0 as pane text")
    cc.run("%0", 'printf "%%end 1 2 0\\n%%exit\\n%%output %%9 fake\\n"')
    cc.backfill("%0")
    return cc.detach()


CJK_LINES = [
    "测试中文 你好世界 ┌─┐ │x│ └─┘",
    "かなカナ 한국어 ไทย العربية हिन्दी",
    "emoji: 😀 👍🏽 👨‍👩‍👧 🇨🇳 ❤️ ✔︎ e\u0301 n\u0303",
]


def w_cjk_emoji():
    cc = attach()
    # One big write: the pane's pty hands tmux ~1 KB pieces, and with 3- and
    # 4-byte characters most piece boundaries fall inside a character, so
    # many %output lines end (and the next begin) mid-sequence.
    body = "".join(
        f"{i:03d} 汉字かなカナ한국어 😀👍🏽👨‍👩‍👧🇨🇳 ┌─┬─┐ é ñ ü 終わり\n" for i in range(120))
    text = write_script("cjk.txt", "\n".join(CJK_LINES) + "\n" + body)
    cc.run("%0", "echo 你好 世界 😀")
    cc.run("%0", f"cat {text}", idle=0.8)
    cc.send("rename-window 汉字窗口")
    cc.send("rename-session 会话")
    cc.sync()
    cc.prologue()
    cc.backfill("%0")
    return cc.detach()


def w_ansi():
    cc = attach()
    script = write_script("ansi.sh", ANSI_SCRIPT)
    cc.run("%0", f"sh {script}", idle=0.6)
    cc.backfill("%0")
    return cc.detach()


def w_altscreen():
    cc = attach()
    lines = "".join(f"line {i:04d} of the alternate-screen file\n" for i in range(1, 201))
    path = write_script("alt.txt", lines)
    cc.run("%0", f"less {path}", idle=0.6)
    cc.send("display-message -p -t %0 '#{alternate_on} #{pane_in_mode} #{mouse_any_flag}'")
    cc.sync()
    cc.backfill("%0")
    cc.send("send-keys -t %0 Space")
    cc.sync()
    cc.quiet()
    cc.send("send-keys -t %0 q")
    cc.sync()
    cc.quiet()
    cc.run("%0", f"vim -u NONE -i NONE -N -n {path}", idle=0.8)
    for keys in ["G", "g g", "-l /0150", "Enter", "o", "-l 'typed in insert mode'", "Escape"]:
        cc.send(f"send-keys -t %0 {keys}")
        cc.sync()
        cc.quiet()
    cc.send("send-keys -t %0 -l ':q!'")
    cc.send("send-keys -t %0 Enter")
    cc.sync()
    cc.quiet(idle=0.6)
    cc.backfill("%0")
    return cc.detach()


def w_layout():
    cc = attach()
    steps = [
        "split-window -h -t %0 -c " + WORKDIR,
        "split-window -v -t %1 -c " + WORKDIR,
        "select-layout -t @0 tiled",
        "select-layout -t @0 even-horizontal",
        "resize-pane -Z -t %1",
        "select-pane -t %0",
        "resize-pane -Z -t %0",
        "new-window -n second -c " + WORKDIR,
        "split-window -h -c " + WORKDIR,
        "select-window -t @0",
        "resize-pane -Z -t %4",
        "select-pane -m -t %0",
        "resize-pane -Z -t %0",
        "resize-pane -Z -t %0",
        "select-pane -M",
        "refresh-client -C 100x30",
        "swap-pane -s %0 -t %1",
        "rotate-window -t @0",
        "rename-window -t @1 renamed-second",
        "set-option -w -t @0 @moshpit_size_owner g2-phone",
        "resize-window -t @0 -x 90 -y 28",
        "kill-pane -t %2",
        "break-pane -d -s %1",
        "join-pane -h -s %1 -t %0",
        "kill-window -t @1",
        "refresh-client -C 80x24",
    ]
    for step in steps:
        cc.send(step)
        cc.sync()
        cc.quiet(idle=0.3)
    cc.run("%0", "echo after the layout dance")
    cc.prologue()
    return cc.detach()


def w_session_switch():
    cc = attach()
    cc.send("new-session -d -s other -c " + WORKDIR)
    cc.sync()
    cc.send("switch-client -t other")
    cc.sync()
    cc.quiet()
    cc.prologue()
    cc.run("%1", "echo in the other session")
    cc.send("switch-client -t g2")
    cc.sync()
    cc.quiet()
    cc.send("rename-session -t other renamed")
    cc.sync()
    # A second, regular client: it attaches, switches session, detaches.
    desk = PtyProcess(BASE + ["attach-session", "-t", "g2"], rows=30, cols=100)
    time.sleep(0.8)
    cc.quiet()
    names = [line.split()[0] for line in
             tmux("list-clients", "-F", "#{client_name} #{client_control_mode}").splitlines()
             if line.endswith(" 0")]
    if names:
        tmux("switch-client", "-c", names[0], "-t", "renamed")
        time.sleep(0.5)
        tmux("detach-client", "-t", names[0])
    desk.finish()
    cc.quiet()
    cc.send("kill-session -t renamed")
    cc.sync()
    cc.quiet()
    cc.prologue()
    return cc.detach()


def w_error():
    cc = attach()
    for command in [
        "bogus-command",
        "select-window -t @999",
        "send-keys -t %999 x",
        "display-message -p -t %0 '#{pane_id}'",
        "list-panes -t no-such-session",
        "set-option -g",
        "capture-pane -p -t %0 -S -2000 -E -1",
        "run-shell 'exit 3'",
        "list-windows -F '#{window_id}'",
    ]:
        cc.send(command)
    cc.sync()
    cc.quiet()
    return cc.detach()


def w_boot_chain():
    # The app's preferred-session boot line: a failed attach falls through.
    line = (f"{TMUX} -L {SOCKET} -f /dev/null -CC attach-session -t nope || "
            f"{TMUX} -L {SOCKET} -f /dev/null -CC attach-session -t g2")
    cc = ControlClient(["/bin/sh", "-c", line])
    after_exit = cc.wait_for(b"%exit")
    cc.wait_for(b"%end ", start=after_exit, timeout=30)
    cc.quiet()
    cc.prologue()
    cc.run("%0", "echo attached through the fallback")
    return cc.detach()


def w_pause():
    cc = attach()
    cc.send("refresh-client -f pause-after=1")
    cc.sync()
    cc.type_line("%0", f"seq 1 30000; tmux -L {SOCKET} wait-for -S g2pause")
    cc.stall(3.0)
    tmux("wait-for", "g2pause", timeout=60)
    cc.quiet(idle=0.6)
    cc.send("refresh-client -A '%0:continue'")
    cc.sync()
    cc.quiet()
    cc.send("refresh-client -f '!pause-after'")
    cc.sync()
    cc.run("%0", "echo resumed")
    return cc.detach()


def w_burst():
    cc = attach()
    cc.run_signalled("%0", "seq 1 200000", "g2burst")
    return cc.detach()


WORKLOADS = {
    "plain": w_plain,
    "cjk-emoji": w_cjk_emoji,
    "ansi": w_ansi,
    "altscreen": w_altscreen,
    "layout": w_layout,
    "session-switch": w_session_switch,
    "error": w_error,
    "boot-chain": w_boot_chain,
    "pause": w_pause,
    "burst": w_burst,
}
COMMITTED = [name for name in WORKLOADS if name != "burst"]


def record(name: str, out: str):
    start_server()
    try:
        data = WORKLOADS[name]()
    finally:
        kill_server()
        shutil.rmtree(WORKDIR, ignore_errors=True)
    with open(out, "wb") as f:
        f.write(data)
    print(f"{name}: {len(data)} bytes -> {out}")


def main(argv):
    if len(argv) != 3 or (argv[1] != "all" and argv[1] not in WORKLOADS):
        sys.exit(__doc__)
    if argv[1] == "all":
        os.makedirs(argv[2], exist_ok=True)
        for name in COMMITTED:
            record(name, os.path.join(argv[2], f"{name}.bin"))
    else:
        record(argv[1], argv[2])


if __name__ == "__main__":
    main(sys.argv)
