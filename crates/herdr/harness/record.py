#!/usr/bin/env python3
"""Records real herdr output for the harness, from a PRIVATE herdr server.

    record.py <out-dir>

Isolation: every herdr command runs with HOME set to a fresh temporary
directory AND `--session moshpit-w2`, so the server, its socket
(<tmp>/.config/herdr/sessions/moshpit-w2/herdr.sock), its config and its
worktrees all live there; the user's own herdr is never addressed. Pane
shells are zsh with an empty rc (a plain `% ` prompt). The server is stopped
and the directory removed at the end, whatever happens.

Writes into <out-dir>:
  status-*.txt          `herdr status`: stopped, running
  replies.jsonl         one {"command","output","exit"} per CLI call: api
                        snapshot (none, empty, full), agent manifests,
                        agent focus / prompt on a shell pane, pane run,
                        send-keys, worktree create / dirty remove / forced
                        remove, a refused attach
  frames.jsonl          raw stdout of `terminal session control` while the
                        script types, resizes, scrolls, clicks (mouse
                        reporting on), pastes (bracketed paste on) and
                        releases; frames.stderr its stderr
  frames-log.jsonl      what was written into it, and when
  socket.jsonl          raw bytes of an events.subscribe conversation on
                        the API socket: the bootstrap replay, live mutations,
                        a session.snapshot reply, and an invalid request
"""
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

SESSION = "moshpit-w2"


class Herdr:
    def __init__(self, home):
        self.home = home
        self.env = {
            "PATH": os.environ["PATH"],
            "HOME": home,
            "SHELL": "/bin/zsh",
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
            "LC_ALL": "en_US.UTF-8",
        }
        self.replies = []

    def argv(self, *args):
        return ["herdr", "--session", SESSION, *args]

    def run(self, *args, record=True, timeout=20):
        done = subprocess.run(self.argv(*args), env=self.env, capture_output=True, timeout=timeout)
        output = done.stdout.decode("utf-8", "replace") + done.stderr.decode("utf-8", "replace")
        if record:
            self.replies.append({"command": " ".join(args), "output": output, "exit": done.returncode})
        return output, done.returncode

    def json(self, *args):
        output, _ = self.run(*args)
        start = output.find("{")
        return json.loads(output[start:output.rfind("}") + 1]) if start != -1 else {}

    @property
    def socket_path(self):
        return os.path.join(self.home, ".config", "herdr", "sessions", SESSION, "herdr.sock")


class Attach:
    """`terminal session control` with its stdin and stdout on pipes."""

    def __init__(self, herdr, target, cols=80, rows=24):
        self.process = subprocess.Popen(
            herdr.argv("terminal", "session", "control", target, "--cols", str(cols), "--rows", str(rows),
                       "--takeover"),
            env=herdr.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.out = bytearray()
        self.err = bytearray()
        self.log = []
        self.started = time.time()
        threading.Thread(target=self.pump, args=(self.process.stdout, self.out), daemon=True).start()
        threading.Thread(target=self.pump, args=(self.process.stderr, self.err), daemon=True).start()

    @staticmethod
    def pump(stream, sink):
        while True:
            chunk = stream.read1(65536)
            if not chunk:
                return
            sink.extend(chunk)

    def send(self, line, settle=0.4):
        self.log.append({"at": round(time.time() - self.started, 3), "line": line})
        try:
            self.process.stdin.write(line.encode() + b"\n")
            self.process.stdin.flush()
        except BrokenPipeError:
            pass
        time.sleep(settle)

    def type(self, text, settle=0.6):
        self.send(json.dumps({"type": "terminal.input", "bytes": base64.b64encode(text.encode()).decode()},
                             separators=(",", ":")), settle)

    def type_bytes(self, data, settle=0.6):
        self.send(json.dumps({"type": "terminal.input", "bytes": base64.b64encode(data).decode()},
                             separators=(",", ":")), settle)

    def finish(self):
        try:
            self.process.stdin.close()
        except BrokenPipeError:
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        time.sleep(0.2)


def record_frames(herdr, pane, out):
    attach = Attach(herdr, pane)
    time.sleep(1.0)
    attach.type("printf 'hello \\033[1;31mred\\033[0m 中文 😀 é\\n'\r")
    attach.type("seq 1 40\r")
    attach.send('{"type":"terminal.resize","cols":100,"rows":30}')
    attach.send('{"type":"terminal.resize","cols":60,"rows":20}')
    attach.send('{"type":"terminal.scroll","direction":"up","lines":5,"source":"wheel"}')
    attach.send('{"type":"terminal.scroll","direction":"down","lines":5,"source":"wheel"}')
    attach.send('{"type":"terminal.scroll","direction":"up","lines":1,"source":"page_key"}')
    attach.type("\x03")
    # Mouse reporting on (SGR), then a click at column 12, row 4 (0-based):
    # the program reads ESC[<0;13;5M / m.
    attach.type("printf '\\033[?1000h\\033[?1006h'; cat -v\r")
    attach.send('{"type":"terminal.mouse","action":"down","button":"left","column":12,"row":4,"modifiers":0}')
    attach.send('{"type":"terminal.mouse","action":"up","button":"left","column":12,"row":4,"modifiers":0}')
    attach.type("\x03")
    attach.type("printf '\\033[?1000l\\033[?1006l'\r")
    # Bracketed paste on: what the program receives for the port's paste and
    # for the two Swift payloads that end it early (paste-marker-unicode,
    # paste-end-marker).
    attach.type("printf '\\033[?2004h'; cat -v\r")
    attach.type_bytes(b"\x1b[200~ls\xcc\x81\necho SAFE\n\x1b[201~")
    attach.type_bytes(b"\x1b[200~ls\x1b[201~\xcc\x81\necho INJECTED\n\x1b[201~")
    attach.type_bytes(b"\x1b[200~\x1b[201~\necho INJECTED\n\x1b[201~")
    attach.type("\x03")
    attach.type("printf '\\033[?2004l'\r")
    # The payloads at zsh's own prompt, which turns bracketed paste on itself.
    # Each pasted command prints $((40+2)), so `…42` on screen means it RAN:
    # the port's paste (SAFE) must not run, Swift's two (INJ, NEST) do.
    for payload in (b"\x1b[200~ls\xcc\x81\necho SAFE$((40+2))\n\x1b[201~",
                    b"\x1b[200~ls\x1b[201~\xcc\x81\necho INJ$((40+2))\n\x1b[201~",
                    b"\x1b[200~\x1b[201~\necho NEST$((40+2))\n\x1b[201~"):
        attach.type_bytes(payload, settle=1.0)
        attach.type("\x03", settle=0.6)
    attach.type("clear\r")
    # A command herdr rejects: `input ignored`, on stderr.
    attach.send('{"type":"terminal.resize","cols":70000,"rows":24}')
    attach.send('{"type":"terminal.release"}', settle=1.0)
    attach.finish()
    with open(os.path.join(out, "frames.jsonl"), "wb") as f:
        f.write(bytes(attach.out))
    with open(os.path.join(out, "frames.stderr"), "wb") as f:
        f.write(bytes(attach.err))
    with open(os.path.join(out, "frames-log.jsonl"), "w") as f:
        for entry in attach.log:
            f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def record_socket(herdr, pane, out):
    received = bytearray()
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(herdr.socket_path)
    sock.settimeout(0.3)

    def send(data):
        try:
            sock.sendall(data)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def drain(seconds):
        end = time.time() + seconds
        while time.time() < end:
            try:
                chunk = sock.recv(65536)
            except socket.timeout:
                continue
            except OSError:
                return
            if not chunk:
                return
            received.extend(chunk)

    kinds = ["workspace.created", "workspace.updated", "workspace.metadata_updated", "workspace.renamed",
             "workspace.moved", "workspace.reordered", "workspace.closed", "workspace.focused",
             "worktree.created", "worktree.opened", "worktree.removed", "tab.created", "tab.closed",
             "tab.renamed", "tab.moved", "tab.focused", "pane.created", "pane.closed", "pane.updated",
             "pane.focused", "pane.moved", "pane.exited", "pane.agent_detected", "layout.updated"]
    request = {"id": "mp1", "method": "events.subscribe",
               "params": {"subscriptions": [{"type": kind} for kind in kinds]}}
    send(json.dumps(request).encode() + b"\n")
    drain(1.5)
    herdr.run("tab", "create", "--focus", "--label", "socket-tab", record=False)
    drain(0.8)
    herdr.run("workspace", "rename", "w1", "renamed", record=False)
    drain(0.8)
    snapshot = herdr.json("api", "snapshot")
    tabs = [t["tab_id"] for t in snapshot.get("result", {}).get("snapshot", {}).get("tabs", [])
            if t.get("label") == "socket-tab"]
    if tabs:
        herdr.run("tab", "close", tabs[0], record=False)
    drain(0.8)
    send(b'{"id":"mp2","method":"session.snapshot","params":{}}\n')
    drain(0.8)
    send(b'{"id":"mp3","method":"pane.get","params":{"pane_id":"' + pane.encode() + b'"}}\n')
    drain(0.6)
    send(b'{"id":"mp4","method":"pane.get","params":{"pane_id":"w9:p9"}}\n')
    drain(0.6)
    send(b"not json at all\n")
    drain(0.8)
    sock.close()
    with open(os.path.join(out, "socket.jsonl"), "wb") as f:
        f.write(bytes(received))


def main():
    out = os.path.abspath(sys.argv[1])
    os.makedirs(out, exist_ok=True)
    home = tempfile.mkdtemp(prefix="mpw2", dir="/tmp")
    herdr = Herdr(home)
    server = None
    try:
        with open(os.path.join(home, ".zshrc"), "w") as f:
            f.write("PROMPT='%% '\nunsetopt BEEP\n")
        with open(os.path.join(out, "status-stopped.txt"), "w") as f:
            f.write(herdr.run("status", record=False)[0])
        herdr.run("api", "snapshot")
        server = subprocess.Popen(herdr.argv("server"), env=herdr.env, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(100):
            if herdr.run("api", "snapshot", record=False)[1] == 0:
                break
            time.sleep(0.1)
        with open(os.path.join(out, "status-running.txt"), "w") as f:
            f.write(herdr.run("status", record=False)[0])
        herdr.run("api", "snapshot")
        herdr.json("workspace", "create", "--focus", "--label", "rec")
        snapshot = herdr.json("api", "snapshot")
        first = snapshot["result"]["snapshot"]["focused_pane_id"]
        herdr.run("tab", "create", "--focus", "--label", "logs")
        herdr.run("pane", "split", first, "--direction", "right", "--focus")
        herdr.run("workspace", "create", "--label", "second")
        time.sleep(1.0)
        snapshot = herdr.json("api", "snapshot")
        herdr.run("server", "agent-manifests", "--json")
        herdr.run("status")
        herdr.run("agent", "focus", first)
        herdr.run("agent", "prompt", first, "hello")
        herdr.run("pane", "send-keys", first, "Enter")
        herdr.run("pane", "run", first, "true")
        herdr.run("pane", "zoom", "--pane", first, "--on")
        herdr.run("pane", "zoom", "--pane", first, "--off")
        herdr.run("pane", "close", "w9:p9")
        herdr.run("workspace", "focus", "w1")
        # A worktree task, in a throwaway repository inside the private HOME.
        repo = os.path.join(home, "repo")
        os.makedirs(repo)
        git = {"GIT_AUTHOR_NAME": "rec", "GIT_AUTHOR_EMAIL": "rec@example.com",
               "GIT_COMMITTER_NAME": "rec", "GIT_COMMITTER_EMAIL": "rec@example.com", **herdr.env}
        for args in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "init"]):
            subprocess.run(["git", *args], cwd=repo, env=git, check=True)
        created = herdr.json("worktree", "create", "--cwd", repo, "--branch", "rec-task", "--label", "rec-task",
                             "--focus", "--json")
        workspace = created.get("result", {}).get("workspace", {}).get("workspace_id")
        checkout = created.get("result", {}).get("root_pane", {}).get("cwd")
        herdr.run("api", "snapshot")
        if workspace and checkout:
            with open(os.path.join(checkout, "dirty.txt"), "w") as f:
                f.write("uncommitted\n")
            herdr.run("worktree", "remove", "--workspace", workspace)
            herdr.run("worktree", "remove", "--workspace", workspace, "--force")
        herdr.run("workspace", "focus", "w1")
        snapshot = herdr.json("api", "snapshot")
        pane = snapshot["result"]["snapshot"]["focused_pane_id"]
        record_frames(herdr, pane, out)
        attach = Attach(herdr, "w9:p9")
        time.sleep(1.0)
        attach.finish()
        herdr.replies.append({"command": "terminal session control w9:p9 --cols 80 --rows 24 --takeover",
                              "output": bytes(attach.out + attach.err).decode("utf-8", "replace"),
                              "exit": attach.process.returncode})
        record_socket(herdr, pane, out)
        herdr.run("api", "snapshot")
    finally:
        herdr.run("server", "stop", record=False)
        if server:
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
        with open(os.path.join(out, "replies.jsonl"), "w") as f:
            for reply in herdr.replies:
                f.write(json.dumps(reply, ensure_ascii=False) + "\n")
        stopped, _ = herdr.run("status", record=False)
        shutil.rmtree(home, ignore_errors=True)
        print(f"private server stopped: {'not running' in stopped}; {home} removed", file=sys.stderr)


if __name__ == "__main__":
    main()
