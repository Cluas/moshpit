#!/usr/bin/env python3
"""CLI lab driver: runs one coding CLI against fake_model.py in an isolated
HOME, types into it char by char, and records what a Moshpit pane would
receive.

    drive.py <cli> <path> [--out DIR] [--text "hello world"] [--cols 80 --rows 24]

<path> is one of
  tmux       a private tmux server (`tmux -L clilab-<cli>`, no config); a
             control-mode client records `%output`; keys go in through
             `send-keys` on that client, as Moshpit's do.
  ssh        the CLI on a bare pty with SSH_CONNECTION/SSH_TTY set (what an
             SSH session without tmux looks like to the CLI); raw pty bytes.
  mosh       mosh-server on 127.0.0.1 running the CLI, a real mosh-client
             (prediction display off) on a pty; its output bytes are mosh's
             frame diffs (Display::new_frame), the same generator that makes
             the host bytes Moshpit's own mosh transport feeds the terminal.
  mosh-tmux  as mosh, with the CLI inside a private tmux in mosh-server.

Fixture: <out>/<cli>-<path>.jsonl, one event per line:
  {"o": base64}       bytes the pane received
  {"k": "h"}          a key was sent (after this line, before the next "o")
  {"ref": [...], "cursor": [x, y]}   tmux's capture-pane at the end (tmux)
Never uses the user's HOME, tmux server or credentials.
"""
import argparse, base64, json, os, pty, re, select, shutil, signal, socket, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
CLISRC = "/tmp/clisrc"
# Its real path, /private/tmp/L, is as long as /Users/dev/lab, the path the
# fixtures show instead: a CLI pads and positions by the real path's width,
# so a scrub that changed the width would move the text after it.
LAB = "/tmp/L"
# Values written into fixtures in place of the lab's real paths.
SCRUB = [("/private" + LAB, "/Users/dev/lab"), (LAB, "/Users/dev/lab")]


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def clis(port, home, work):
    base = f"http://127.0.0.1:{port}"
    # Any key will do: fake_model.py checks none. Never a real one.
    key = os.environ.get("CLILAB_KEY") or "lab-dummy-" + "0" * 32
    node = shutil.which("node")
    # Never set CI (even empty): Ink-based CLIs then render as in a CI log,
    # static output only, and never draw their input box.
    common = {"NO_UPDATE_NOTIFIER": "1", "DISABLE_AUTOUPDATER": "1"}

    def setup_claude():
        os.makedirs(home, exist_ok=True)
        json.dump({"hasCompletedOnboarding": True, "theme": "dark",
                   "customApiKeyResponses": {"approved": [key[-20:]], "rejected": []},
                   "projects": {work: {"hasTrustDialogAccepted": True, "allowedTools": []}}},
                  open(os.path.join(home, ".claude.json"), "w"))

    def setup_codex():
        d = os.path.join(home, ".codex")
        os.makedirs(d, exist_ok=True)
        open(os.path.join(d, "config.toml"), "w").write(
            f'model = "lab-model"\nmodel_provider = "lab"\n'
            f'[model_providers.lab]\nname = "lab"\nbase_url = "{base}/v1"\nenv_key = "LAB_KEY"\nwire_api = "responses"\n'
            f'[projects."{work}"]\ntrust_level = "trusted"\n')

    def setup_gemini():
        d = os.path.join(home, ".gemini")
        os.makedirs(d, exist_ok=True)
        json.dump({"security": {"auth": {"selectedType": "gemini-api-key"}}, "selectedAuthType": "gemini-api-key",
                   "general": {"disableAutoUpdate": True}, "ui": {"hideTips": True}},
                  open(os.path.join(d, "settings.json"), "w"))
        json.dump({work: "TRUST_FOLDER"}, open(os.path.join(d, "trustedFolders.json"), "w"))

    def setup_qwen():
        d = os.path.join(home, ".qwen")
        os.makedirs(d, exist_ok=True)
        json.dump({"security": {"auth": {"selectedType": "openai", "apiKey": key, "baseUrl": base + "/v1"}},
                   "selectedAuthType": "openai", "model": {"name": "lab-model"}},
                  open(os.path.join(d, "settings.json"), "w"))

    def setup_iflow():
        d = os.path.join(home, ".iflow")
        os.makedirs(d, exist_ok=True)
        json.dump({"selectedAuthType": "openai-compatible", "apiKey": key, "baseUrl": base + "/v1",
                   "modelName": "lab-model"}, open(os.path.join(d, "settings.json"), "w"))

    def setup_qoder():
        # useExternal: Qoder's own switch for "auth is handled elsewhere";
        # it starts unauthenticated and skips the sign-in screen, so the
        # input box comes up with no account and no network.
        d = os.path.join(home, ".qoder")
        os.makedirs(d, exist_ok=True)
        json.dump({"security": {"auth": {"useExternal": True, "selectedType": "openai"},
                                "folderTrust": {"enabled": False}},
                   "providers": {"lab": {"protocol": "openai", "baseUrl": base + "/v1", "apiKey": key,
                                         "model": "lab-model"}},
                   "model": {"name": "lab-model"}, "general": {"disableAutoUpdate": True}},
                  open(os.path.join(d, "settings.json"), "w"))

    def setup_codebuddy():
        # A trusted folder skips the trust dialog.
        d = os.path.join(home, ".codebuddy")
        os.makedirs(d, exist_ok=True)
        json.dump({"trustedDirectories": [work]}, open(os.path.join(d, "settings.json"), "w"))

    return {
        "qoder": ([node, f"{CLISRC}/qoder-ai-qodercli-1.1.92/package/bundle/qodercli.js"],
                  {"OPENAI_API_KEY": key, "OPENAI_BASE_URL": base + "/v1", "OPENAI_MODEL": "lab-model"}, setup_qoder),
        "claude": ([shutil.which("claude") or "claude"],
                   {"ANTHROPIC_BASE_URL": base, "ANTHROPIC_API_KEY": key, "ANTHROPIC_MODEL": "lab-model",
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"}, setup_claude),
        "codex": ([shutil.which("codex") or "codex", "--no-daemon"], {"LAB_KEY": key, "CODEX_HOME": f"{home}/.codex"}, setup_codex),
        "gemini": ([shutil.which("gemini") or "gemini"],
                   {"GEMINI_API_KEY": key, "GOOGLE_GEMINI_BASE_URL": base, "GEMINI_MODEL": "lab-model"},
                   setup_gemini),
        "qwen": ([node, f"{CLISRC}/qwen-code-qwen-code-0.28.0/package/cli-entry.js"],
                 {"OPENAI_API_KEY": key, "OPENAI_BASE_URL": base + "/v1", "OPENAI_MODEL": "lab-model"}, setup_qwen),
        "iflow": ([node, f"{CLISRC}/iflow-ai-iflow-cli-0.5.19/package/bundle/entry.js"],
                  {"IFLOW_API_KEY": key, "IFLOW_BASE_URL": base + "/v1", "IFLOW_MODEL_NAME": "lab-model"},
                  setup_iflow),
        "codebuddy": ([node, f"{CLISRC}/tencent-ai-codebuddy-code-2.185.0/package/bin/codebuddy"],
                      {"CODEBUDDY_BASE_URL": base + "/v1", "CODEBUDDY_API_KEY": key, "CODEBUDDY_MODEL": "lab-model"},
                      setup_codebuddy),
        "cursor": ([shutil.which("cursor-agent") or "cursor-agent"], {}, None),
        "copilot": ([shutil.which("copilot") or "copilot"], {}, None),
        # A plain shell: the lab's control (readline echo, no redraws).
        "shell": (["/bin/sh"], {"PS1": "$ "}, None),
    }, common


def scrub(b):
    for real, fake in SCRUB:
        b = b.replace(real.encode(), fake.encode())
    return b


class Rec:
    def __init__(self, path):
        self.f = open(path, "w")
        self.last = time.monotonic()
        self.t0 = time.monotonic()
        self.seen = b""

    def ms(self):
        return int((time.monotonic() - self.t0) * 1000)

    def out(self, b):
        if isinstance(b, list):
            for c in b:
                self.out(c)
            return
        if b:
            self.f.write(json.dumps({"t": self.ms(), "o": base64.b64encode(scrub(b)).decode()}) + "\n")
            self.seen = (self.seen + b)[-200000:]
            self.last = time.monotonic()

    def key(self, k):
        self.f.write(json.dumps({"t": self.ms(), "k": k}) + "\n")

    def meta(self, **kw):
        self.f.write(json.dumps(kw, ensure_ascii=False) + "\n")


def read_until_quiet(read, rec, quiet, cap):
    """Reads until nothing has come for `quiet` s (at most `cap` s)."""
    start = time.monotonic()
    last = time.monotonic()
    while time.monotonic() - start < cap:
        b = read(0.05)
        if b:
            rec.out(b)
            last = time.monotonic()
        elif time.monotonic() - last > quiet:
            return


def plain(b):
    return re.sub(rb"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\\\)|\x1b.", b"", b)


def keys_for(text):
    if text == "-":
        return []
    return list(text) + ["Enter"]


# --- pty helpers -------------------------------------------------------------

def spawn_pty(argv, env, cols, rows, cwd):
    pid, fd = pty.fork()
    if pid == 0:
        import fcntl, struct, termios
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.chdir(cwd)
        os.execvpe(argv[0], argv, env)
    import fcntl, struct, termios
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    return pid, fd


def pty_reader(fd):
    def read(t):
        r, _, _ = select.select([fd], [], [], t)
        if not r:
            return b""
        try:
            return os.read(fd, 65536)
        except OSError:
            return b""
    return read


def answer_queries(fd, data, cols, rows):
    """A real terminal answers what the CLI asks on start; a silent pty makes
    some CLIs wait seconds for each answer. Answers DA1, DSR-CPR, XTVERSION is
    left unanswered (most terminals' tmux says nothing either)."""
    if b"\x1b[c" in data or b"\x1b[0c" in data:
        os.write(fd, b"\x1b[?62;22c")
    if b"\x1b[6n" in data:
        os.write(fd, f"\x1b[{rows};1R".encode())
    if b"\x1b[14t" in data:
        os.write(fd, f"\x1b[4;{rows*20};{cols*10}t".encode())
    if b"\x1b]11;?" in data:
        os.write(fd, b"\x1b]11;rgb:0000/0000/0000\x1b\\")
    if b"\x1b]10;?" in data:
        os.write(fd, b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\")


def key_bytes(k):
    return b"\r" if k == "Enter" else k.encode()


# --- paths -------------------------------------------------------------------

def run_ssh(argv, env, a, rec, work):
    env = dict(env, SSH_CONNECTION="192.0.2.1 50000 192.0.2.2 22", SSH_CLIENT="192.0.2.1 50000 22",
               SSH_TTY="/dev/ttys099")
    pid, fd = spawn_pty(argv, env, a.cols, a.rows, work)
    read0 = pty_reader(fd)

    def read(t):
        b = read0(t)
        if b:
            answer_queries(fd, b, a.cols, a.rows)
        return b
    try:
        drive_keys(read, lambda k: os.write(fd, key_bytes(k)), a, rec)
    finally:
        kill(pid)


def drive_keys(read, send, a, rec):
    if a.ready:
        # Type once the CLI shows this (its input box) and has been quiet a
        # second. Matched on the text without escape sequences: mosh's diffs
        # move the cursor over blanks, so a pattern is best one word.
        start = time.monotonic()
        frozen = []
        while not re.search(a.ready.encode(), plain(rec.seen)) and time.monotonic() - start < a.boot:
            if a.freeze and not frozen and rec.seen:
                frozen = cli_pids(a.freeze)
            rec.out(read(0.01 if a.freeze else 0.05))
        if a.freeze:
            # Stop the CLI where it stands (Qoder exits ~70 ms after its
            # input box shows when there is no account): the recording ends
            # at its first screen, and tmux's capture is of that screen.
            signal_pids(frozen or cli_pids(a.freeze), signal.SIGSTOP)
            read_until_quiet(read, rec, 0.5, 2)
            return
        read_until_quiet(read, rec, 1.0, a.boot)
    else:
        read_until_quiet(read, rec, a.boot_quiet or a.settle, a.boot)
    for k in a.pre:
        # Not the typing under test (a dialog's answer): no key event.
        send(k)
        read_until_quiet(read, rec, a.boot_quiet or a.settle, a.boot)
    for k in keys_for(a.text):
        rec.key(k)
        send(k)
        t0 = time.monotonic()
        while time.monotonic() - t0 < a.delay:
            b = read(0.02)
            rec.out(b)
    read_until_quiet(read, rec, a.settle, a.reply)


def cli_pids(pattern):
    """The node processes whose command line matches `pattern` (not the tmux
    server or shell that carry the same command line)."""
    out = []
    for pid in subprocess.run(["pgrep", "-f", pattern], capture_output=True, text=True).stdout.split():
        comm = subprocess.run(["ps", "-o", "comm=", "-p", pid], capture_output=True, text=True).stdout.strip()
        if os.path.basename(comm) == "node":
            out.append(int(pid))
    return out


def signal_pids(pids, sig):
    for pid in pids:
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            pass


def kill(pid):
    # The CLI leads its own session (pty.fork): signal the whole group, and
    # never block on a child that will not die.
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for target in (-pid, pid):
            try:
                os.kill(target, sig)
            except (ProcessLookupError, PermissionError):
                pass
        for _ in range(20):
            try:
                if os.waitpid(pid, os.WNOHANG)[0]:
                    return
            except ChildProcessError:
                return
            time.sleep(0.05)


def tmux_cmd(sock, tmpdir):
    t = shutil.which("tmux")
    return lambda *args: subprocess.run([t, "-L", sock, "-f", "/dev/null", *args], capture_output=True,
                                        env={"TMUX_TMPDIR": tmpdir, "PATH": os.environ["PATH"], "HOME": tmpdir},
                                        text=True).stdout


def env_prefix(env):
    return "env -i " + " ".join(f"{k}='{v}'" for k, v in env.items()) + " "


def run_tmux(argv, env, a, rec, work, tmpdir):
    sock = f"clilab-{a.cli}"
    tm = tmux_cmd(sock, tmpdir)
    tm("kill-server")
    # The pane waits for the recorder to attach, so the recording holds the
    # CLI's first byte (Moshpit seeds a pane from capture-pane; here there
    # is nothing before the CLI to seed).
    cmd = (f"{shutil.which('tmux')} -L {sock} -f /dev/null wait-for go; exec " + env_prefix(env)
           + " ".join(f"'{x}'" for x in argv))
    tm("new-session", "-d", "-s", "lab", "-x", str(a.cols), "-y", str(a.rows), "-c", work, cmd)
    tm("set", "-g", "status", "off")
    tm("set", "-g", "remain-on-exit", "on")
    pane = tm("display", "-p", "-t", "lab", "#{pane_id}").strip()
    cc = subprocess.Popen([shutil.which("tmux"), "-L", sock, "-f", "/dev/null", "-C", "attach", "-t", "lab"],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          env={"TMUX_TMPDIR": tmpdir, "PATH": os.environ["PATH"], "HOME": tmpdir})
    cc.stdin.write(f"refresh-client -C {a.cols}x{a.rows}\n".encode())
    cc.stdin.flush()
    time.sleep(0.3)
    tm("wait-for", "-S", "go")
    fd = cc.stdout.fileno()
    os.set_blocking(fd, False)
    buf = bytearray()
    prefix = f"%output {pane} ".encode()

    def read(t):
        r, _, _ = select.select([fd], [], [], t)
        if not r:
            return b""
        try:
            buf.extend(os.read(fd, 65536))
        except BlockingIOError:
            return b""
        # One chunk per `%output` line: what Moshpit's tmux client feeds
        # its pane at a time.
        out = []
        while b"\n" in buf:
            i = buf.index(b"\n")
            line = bytes(buf[:i])
            del buf[:i + 1]
            if line.startswith(prefix):
                out.append(unescape(line[len(prefix):]))
        return out

    def send(k):
        if k == "Enter":
            c = f"send-keys -t {pane} Enter\n"
        else:
            c = f"send-keys -t {pane} -l '{k}'\n"
        cc.stdin.write(c.encode())
        cc.stdin.flush()

    try:
        drive_keys(read, send, a, rec)
        ref = tm("capture-pane", "-p", "-t", pane).split("\n")
        cur = tm("display", "-p", "-t", pane, "#{cursor_x},#{cursor_y},#{cursor_flag}").strip().split(",")
        rec.meta(ref=[scrub(l.encode()).decode() for l in ref[:a.rows]], cursor=[int(x) for x in cur])
    finally:
        cc.kill()
        tm("kill-server")


def unescape(s):
    return re.sub(rb"\\([0-7]{3})", lambda m: bytes([int(m.group(1), 8)]), s)


def run_mosh(argv, env, a, rec, work, tmpdir, in_tmux):
    if in_tmux:
        sock = f"clilab-m{a.cli}"
        inner = env_prefix(env) + " ".join(f"'{x}'" for x in argv)
        t = shutil.which("tmux")
        argv = [t, "-L", sock, "-f", "/dev/null", "new-session", "-s", "lab", inner]
        env = dict(env, TMUX_TMPDIR=tmpdir)
    # mosh-server starts its command in $HOME; the CLI must start in work
    # (its trusted project).
    argv = ["/bin/sh", "-c", 'cd "$0" && exec "$@"', work, *argv]
    srv_env = dict(env, LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8")
    out = subprocess.run([shutil.which("mosh-server"), "new", "-s", "-i", "127.0.0.1", "-c", "256",
                          "-l", "LANG=en_US.UTF-8", "--", *argv],
                         capture_output=True, text=True, env=srv_env, cwd=work)
    m = re.search(r"MOSH CONNECT (\d+) (\S+)", out.stdout + out.stderr)
    if not m:
        raise SystemExit("mosh-server did not start: " + out.stdout + out.stderr)
    port, key = m.group(1), m.group(2)
    cenv = {"MOSH_KEY": key, "MOSH_PREDICTION_DISPLAY": a.predict, "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8", "PATH": os.environ["PATH"], "HOME": tmpdir}
    relay = None
    if a.rtt:
        # A slow link: mosh-client talks to a relay that delays each
        # datagram by half the round trip, both ways.
        front = free_port()
        relay = subprocess.Popen([sys.executable, "-I", os.path.join(HERE, "udp_delay.py"),
                                  str(front), port, str(a.rtt // 2)])
        port = str(front)
        time.sleep(0.2)
    if a.moshpit:
        try:
            run_moshpit_client(port, key, a, rec)
        finally:
            if relay:
                relay.kill()
            subprocess.run(["pkill", "-f", f"mosh-server new -s -i 127.0.0.1 -c 256 -l LANG=en_US.UTF-8 -- /bin/sh -c cd \"\\$0\" && exec \"\\$@\" {work}"])
            if in_tmux:
                tmux_cmd(f"clilab-m{a.cli}", tmpdir)("kill-server")
        return
    pid, fd = spawn_pty([shutil.which("mosh-client"), "127.0.0.1", port], cenv, a.cols, a.rows, work)
    try:
        drive_keys(pty_reader(fd), lambda k: os.write(fd, key_bytes(k)), a, rec)
    finally:
        kill(pid)
        if relay:
            relay.kill()
        subprocess.run(["pkill", "-f", f"mosh-server new -s -i 127.0.0.1 -c 256 -l LANG=en_US.UTF-8 -- /bin/sh -c cd \"\\$0\" && exec \"\\$@\" {work}"])
        if in_tmux:
            tmux_cmd(f"clilab-m{a.cli}", tmpdir)("kill-server")


def run_moshpit_client(port, key, a, rec):
    """Moshpit's own mosh transport (crates/mosh, `mosh_harness live`) in
    place of mosh-client: the bytes recorded are the host bytes of each state
    as the transport hands them to the terminal, with the echo ack it
    carried. Keys go on a fixed schedule (the harness takes no input): the
    boot wait, then one key every --delay s, then --reply s."""
    steps = [f"wait:{int(a.boot * 1000)}"]
    for k in keys_for(a.text):
        steps += [f"type:{key_bytes(k).hex()}", f"wait:{int(a.delay * 1000)}"]
    steps.append(f"wait:{int(a.reply * 1000)}")
    proc = subprocess.Popen([a.moshpit, "live", "127.0.0.1", port, key, str(10**9), *steps],
                            stdout=subprocess.PIPE, text=True)
    sent = {}
    for line in proc.stdout:
        f = line.split()
        if not f:
            continue
        if f[0] == "frame":
            b = bytes.fromhex(f[3]) if len(f) > 3 else b""
            rec.f.write(json.dumps({"t": rec.ms(), "o": base64.b64encode(scrub(b)).decode(),
                                    "a": int(f[2])}) + "\n")
            rec.seen = (rec.seen + b)[-200000:]
        elif f[0] == "diag":
            print(line.strip(), file=sys.stderr)
        elif f[0] == "sent":
            k = bytes.fromhex(f[3])
            rec.f.write(json.dumps({"t": rec.ms(), "k": "Enter" if k == b"\r" else k.decode(),
                                    "n": int(f[2])}) + "\n")
    proc.wait()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("cli")
    p.add_argument("path", choices=["tmux", "ssh", "mosh", "mosh-tmux"])
    p.add_argument("--out", default=os.path.join(HERE, "out"))
    p.add_argument("--name", default=None)
    p.add_argument("--text", default="hello world", help='what to type ("-": nothing, not even Enter)')
    p.add_argument("--cols", type=int, default=80)
    p.add_argument("--rows", type=int, default=24)
    p.add_argument("--delay", type=float, default=0.25, help="seconds between keys")
    p.add_argument("--settle", type=float, default=2.0, help="quiet seconds that end a wait")
    p.add_argument("--boot", type=float, default=25.0)
    p.add_argument("--boot-quiet", type=float, default=None, help="quiet seconds that end the boot wait")
    p.add_argument("--reply", type=float, default=15.0)
    p.add_argument("--ready", default=None, help="regex in the output that starts the typing")
    p.add_argument("--pre", action="append", default=[], help="a key sent once the CLI is up, before the typing")
    p.add_argument("--freeze", default=None,
                   help="once --ready shows, SIGSTOP processes matching this and type nothing")
    p.add_argument("--predict", default="never", help="mosh-client MOSH_PREDICTION_DISPLAY")
    p.add_argument("--rtt", type=int, default=0,
                   help="mosh paths: round trip in ms through a delay relay (udp_delay.py)")
    p.add_argument("--moshpit", default=None,
                   help="mosh paths: Moshpit's mosh transport (path to the mosh_harness example) "
                        "in place of mosh-client; the boot wait is --boot s, fixed")
    a = p.parse_args()

    name = a.name or f"{a.cli}-{a.path}" + (f"-pred-{a.predict}" if a.path.startswith("mosh") and
                                            a.predict != "never" else "") + \
        (f"-rtt{a.rtt}" if a.rtt else "") + ("-rs" if a.moshpit else "")
    root = os.path.join(os.path.realpath(LAB), name)
    shutil.rmtree(root, ignore_errors=True)
    home, work, tmpdir = f"{root}/home", f"{root}/work", f"{root}/t"
    for d in (home, work, tmpdir):
        os.makedirs(d)
    home, work, tmpdir = (os.path.realpath(d) for d in (home, work, tmpdir))
    port = free_port()
    srv = subprocess.Popen([sys.executable, "-I", os.path.join(HERE, "fake_model.py"), str(port),
                            f"{root}/model.log"])
    try:
        table, common = clis(port, home, work)
        argv, extra, setup = table[a.cli]
        if setup:
            setup()
        env = {"HOME": home, "PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:" +
               os.path.dirname(argv[0]), "TERM": "xterm-256color", "COLORTERM": "truecolor",
               "LANG": "en_US.UTF-8", "USER": "dev", "SHELL": "/bin/sh", "TMPDIR": tmpdir,
               "XDG_CONFIG_HOME": f"{home}/.config", **common, **extra}
        os.makedirs(a.out, exist_ok=True)
        rec = Rec(os.path.join(a.out, name + ".jsonl"))
        rec.meta(cli=a.cli, path=a.path, cols=a.cols, rows=a.rows, text=a.text, predict=a.predict)
        if a.path == "tmux":
            run_tmux(argv, env, a, rec, work, tmpdir)
        elif a.path == "ssh":
            run_ssh(argv, env, a, rec, work)
        else:
            run_mosh(argv, env, a, rec, work, tmpdir, a.path == "mosh-tmux")
        rec.f.close()
        print(os.path.join(a.out, name + ".jsonl"))
        print(open(f"{root}/model.log").read() if os.path.exists(f"{root}/model.log") else "(no model calls)",
              file=sys.stderr)
    finally:
        srv.kill()
        if a.freeze:
            signal_pids(cli_pids(a.freeze), signal.SIGKILL)
        subprocess.run(["pkill", "-f", f"{home}/"])


if __name__ == "__main__":
    main()
