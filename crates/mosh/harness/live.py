#!/usr/bin/env python3
"""Live experiments: the Rust mosh client (crates/mosh) against a real
mosh-server on this Mac, through a UDP proxy that can drop either direction
and records every datagram. (They compared the app's Swift client too,
until it was deleted (E2); divergences.json keeps what they found.)

    live.py throwaway [--count N] [--interval MS]
    live.py ttl
    live.py redraw
    live.py idle      [--client rust|mosh|all]
    live.py paste     [--client rust|mosh|all] [--sizes N,N]
                      [--pattern periodic|random] [--loss P] [--delay MS]
    live.py typing    [--client rust|mosh|all] [--count N]
                      [--interval MS] [--loss P] [--delay MS] [--seed N]
    live.py exit      [--client rust|mosh|all]
    live.py refused   [--client rust|mosh|all]
    live.py rebuild
    live.py capture   [--out DIR]        record a session's wire datagrams
    live.py [...] --no-build

MOSH_HARNESS_RUST runs another build of the client (a copy of the binary
from before a change, say) instead of this tree's.

The server runs `mosh-server new -i 127.0.0.1` with a small Python program
as its command: it logs every byte it reads (raw tty, no echo) with a
timestamp, so a report can say which keystrokes reached the remote side,
when, and in what order. MOSH_SERVER_NETWORK_TMOUT bounds its life; the
script kills it afterwards anyway. Everything runs niced, one client at a
time. `--client mosh` (idle, paste, typing) runs mosh's own mosh-client, on
a pty, as the reference. `--loss` drops that fraction of the datagrams each
way at random (seeded), `--delay` holds each one that long each way.
"""
import argparse
import heapq
import os
import random
import selectors
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
WORK = os.path.join(REPO, "target", "mosh-harness")
BUILT_RUST = os.path.join(REPO, "target", "release", "examples", "mosh_harness")
RUST = os.environ.get("MOSH_HARNESS_RUST") or BUILT_RUST
NICE = ["nice", "-n", "10"]

REMOTE = r'''
import os, sys, time, tty
mode = sys.argv[2] if len(sys.argv) > 2 else ""
if mode == "raw":
    # Every byte read, as read; per read "ms total" in <log>.t.
    out, timing, total = open(sys.argv[1], "ab", 0), open(sys.argv[1] + ".t", "a"), 0
    tty.setraw(0)
    while True:
        data = os.read(0, 65536)
        if not data:
            break
        out.write(data)
        total += len(data)
        timing.write("%d %d\n" % (int(time.time() * 1000), total))
        timing.flush()
    sys.exit(0)
log = open(sys.argv[1], "ab", 0)
if mode == "lines":
    for i in range(1, 23):
        os.write(1, ("line %02d " % i + "abcdefghij" * 6 + "\r\n").encode())
tty.setraw(0)
if mode.startswith("tick:"):
    # A line every N ms, for as long as it runs.
    n = 0
    while True:
        n += 1
        os.write(1, b"tick %d\r\n" % n)
        time.sleep(int(mode[5:]) / 1000)
if mode.startswith("exit:"):
    # The command exits after a delay (`exit` typed into a shell, as far as
    # mosh-server is concerned); when, in <log>.ev.
    time.sleep(int(mode[5:]) / 1000)
    os.write(1, b"bye\r\n")
    with open(sys.argv[1] + ".ev", "a") as events:
        events.write("exit %d\n" % int(time.time() * 1000))
    sys.exit(0)
if mode.startswith("ping:"):
    # Started (released by the first user state), then PING after a delay;
    # both times in <log>.ev.
    events = open(sys.argv[1] + ".ev", "a")
    start = int(time.time() * 1000)
    events.write("start %d\n" % start)
    events.flush()
    time.sleep(max(0, start + int(mode[5:]) - time.time() * 1000) / 1000)
    os.write(1, b"PING\r\n")
    events.write("ping %d\n" % int(time.time() * 1000))
    events.flush()
while True:
    data = os.read(0, 4096)
    if not data:
        break
    now = int(time.time() * 1000)
    log.write(b"".join(b"%d %02x\n" % (now, b) for b in data))
'''


def build():
    env = dict(os.environ, CARGO_BUILD_JOBS="2")
    subprocess.run(NICE + ["cargo", "build", "--quiet", "--release", "-p", "moshpit-mosh",
                           "--example", "mosh_harness"], check=True, cwd=REPO, env=env)


class Server:
    """A detached mosh-server whose command logs what it reads. `screen` is
    the remote program's mode (see REMOTE); `tmout` its
    MOSH_SERVER_NETWORK_TMOUT; `verbose` keeps mosh-server's own log (-v:
    every state it receives and sends, its idle exit, refused input) in
    `server_log`."""

    def __init__(self, screen="", tmout=120, verbose=False):
        self.dir = tempfile.mkdtemp(prefix="mosh-live-")
        self.log = os.path.join(self.dir, "remote.log")
        self.server_log = os.path.join(self.dir, "server.log")
        mosh_server = shutil.which("mosh-server")
        if not mosh_server:
            sys.exit("mosh-server not on PATH")
        env = dict(os.environ, LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8",
                   MOSH_SERVER_NETWORK_TMOUT=str(tmout))
        # Files, not pipes: with -v the detached server keeps its stdout and
        # stderr open.
        out_path = os.path.join(self.dir, "out.txt")
        with open(out_path, "w") as out, open(self.server_log, "w") as err:
            subprocess.run(
                NICE + [mosh_server, "new", "-i", "127.0.0.1", "-c", "256", "-l", "LANG=en_US.UTF-8"]
                + (["-v"] if verbose else [])
                + ["--", sys.executable, "-I", "-c", REMOTE, self.log, screen],
                stdout=out, stderr=err, env=env, stdin=subprocess.DEVNULL, timeout=30)
        with open(out_path) as f:
            text = f.read()
        with open(self.server_log, errors="replace") as f:
            text += f.read()
        self.port = self.key = self.pid = None
        for line in text.splitlines():
            if line.startswith("MOSH CONNECT "):
                _, _, port, key = line.split()[:4]
                self.port, self.key = int(port), key
            if "mosh-server detached, pid = " in line:
                self.pid = int(line.split("pid = ")[1].rstrip("]"))
        if not (self.port and self.key and self.pid):
            sys.exit("mosh-server did not start:\n" + text)

    def alive(self):
        try:
            os.kill(self.pid, 0)
            return True
        except OSError:
            return False

    def received(self):
        """[(ms, byte)] the remote command read, in order."""
        if not os.path.exists(self.log):
            return []
        out = []
        with open(self.log) as f:
            for line in f:
                ms, byte = line.split()
                out.append((int(ms), int(byte, 16)))
        return out

    def server_lines(self):
        with open(self.server_log, errors="replace") as f:
            return f.read().splitlines()

    def rss_kb(self):
        done = subprocess.run(["ps", "-o", "rss=", "-p", str(self.pid)], capture_output=True, text=True)
        return int(done.stdout.strip() or 0)

    def events(self):
        """{name: ms} from the remote program's <log>.ev."""
        path = self.log + ".ev"
        if not os.path.exists(path):
            return {}
        with open(path) as f:
            return {name: int(ms) for name, ms in (line.split() for line in f if line.strip())}

    def stop(self):
        for sig in (signal.SIGTERM, signal.SIGKILL):
            if not self.alive():
                break
            os.kill(self.pid, sig)
            for _ in range(50):
                if not self.alive():
                    break
                time.sleep(0.02)
        shutil.rmtree(self.dir, ignore_errors=True)


class Proxy(threading.Thread):
    """client <-> 127.0.0.1:<port> <-> server, with switchable drops, random
    loss (`loss`, a fraction each way, seeded), a delay each way
    (`delay_ms`), a port that can go away for a while (`port_gone`: the
    client's datagrams then get ICMP port unreachable), and a record of
    every datagram: (ms, 'up'|'down', bytes, dropped)."""

    def __init__(self, server_port, loss=0.0, delay_ms=0, seed=1):
        super().__init__(daemon=True)
        self.front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.front.bind(("127.0.0.1", 0))
        self.port = self.front.getsockname()[1]
        self.back = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.back.connect(("127.0.0.1", server_port))
        self.client = None
        self.drop_up = False
        self.drop_down = False
        self.port_gone = False
        self.loss = loss
        self.delay = delay_ms / 1000.0
        self.rng = {"up": random.Random(seed), "down": random.Random(seed + 1)}
        self.queue = []
        self.queued = 0
        self.record = []
        self.stopping = False

    def lost(self, direction):
        return self.loss > 0 and self.rng[direction].random() < self.loss

    def forward(self, direction, data):
        try:
            if direction == "up":
                self.back.send(data)
            elif self.client and self.front is not None:
                self.front.sendto(data, self.client)
        except OSError:        # the server is gone
            pass

    def run(self):
        # Receives don't wait: once the server is gone, a send can take the
        # ICMP error that made the socket readable, and a blocking recv then
        # waits forever. Sends still block, as before.
        sel = selectors.DefaultSelector()
        sel.register(self.front, selectors.EVENT_READ)
        sel.register(self.back, selectors.EVENT_READ)
        while not self.stopping:
            if self.port_gone and self.front is not None:
                sel.unregister(self.front)
                self.front.close()
                self.front = None
            elif not self.port_gone and self.front is None:
                self.front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                self.front.bind(("127.0.0.1", self.port))
                sel.register(self.front, selectors.EVENT_READ)
            timeout = 0.05
            if self.queue:
                timeout = max(0.0, min(timeout, self.queue[0][0] - time.time()))
            for key, _ in sel.select(timeout):
                now = time.time()
                if key.fileobj is self.front:
                    try:
                        data, self.client = self.front.recvfrom(65536, socket.MSG_DONTWAIT)
                    except (BlockingIOError, ConnectionRefusedError):
                        continue
                    direction, dropped = "up", self.drop_up or self.lost("up")
                else:
                    try:
                        data = self.back.recv(65536, socket.MSG_DONTWAIT)
                    except (BlockingIOError, ConnectionRefusedError):
                        continue
                    direction, dropped = "down", (self.drop_down or self.front is None
                                                  or self.lost("down"))
                self.record.append((int(now * 1000), direction, data, dropped))
                if dropped:
                    continue
                if self.delay:
                    self.queued += 1
                    heapq.heappush(self.queue, (now + self.delay, self.queued, direction, data))
                else:
                    self.forward(direction, data)
            now = time.time()
            while self.queue and self.queue[0][0] <= now:
                _, _, direction, data = heapq.heappop(self.queue)
                self.forward(direction, data)

    def stop(self):
        self.stopping = True
        self.join()


def run_client(client, proxy, server, ttl, steps, schedule=()):
    """Runs a client; `schedule` is [(seconds after start, callback)]."""
    binary = RUST
    argv = NICE + [binary, "live", "127.0.0.1", str(proxy.port), server.key, str(ttl)] + steps
    proc = subprocess.Popen(argv, stdout=subprocess.PIPE, text=True)
    started = time.time()
    for at, action in sorted(schedule, key=lambda item: item[0]):
        delay = started + at - time.time()
        if delay > 0:
            time.sleep(delay)
        action()
    out, _ = proc.communicate(timeout=600)
    return [line.split(" ", 3) for line in out.splitlines()]


def sent_lines(lines):
    return [(int(f[1]), bytes.fromhex(f[3]) if len(f) > 3 else b"") for f in lines if f[0] == "sent"]


def frames(lines):
    return [(int(f[1]), bytes.fromhex(f[3]) if len(f) > 3 else b"") for f in lines if f[0] == "frame"]


def clients(choice):
    return {"all": ["rust", "mosh"]}.get(choice, [choice])


def run_mosh_client(proxy, server, seconds, schedule=(), writes=(), typed=None, exited=None):
    """mosh's own mosh-client on an 80x24 pty for `seconds`: [(ms, bytes)]
    it wrote to its terminal. `writes` is [(seconds after start, bytes)]
    typed into it, as fast as the pty takes them; `typed`, if a list, gets
    (ms, bytes) for each when it began; `exited`, if a list, gets the ms
    mosh-client exited at, if it did."""
    import fcntl
    import pty
    import struct
    import termios
    pid, fd = pty.fork()
    if pid == 0:
        # The size before mosh-client reads it (a 0x0 pty makes it fail).
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        os.environ.update(MOSH_KEY=server.key, TERM="xterm-256color", LANG="en_US.UTF-8",
                          LC_ALL="en_US.UTF-8")
        os.execvp("nice", ["nice", "-n", "10", "mosh-client", "127.0.0.1", str(proxy.port)])
    os.set_blocking(fd, False)
    out, started = [], time.time()
    pending = sorted(schedule, key=lambda item: item[0])
    to_write = sorted(writes, key=lambda item: item[0])
    queue, position = b"", 0
    sel = selectors.DefaultSelector()
    sel.register(fd, selectors.EVENT_READ)
    gone = False
    while time.time() < started + seconds and not gone:
        if exited is not None and os.waitpid(pid, os.WNOHANG)[0] == pid:
            exited.append(int(time.time() * 1000))
            gone = True
            break
        while pending and time.time() >= started + pending[0][0]:
            pending.pop(0)[1]()
        while to_write and time.time() >= started + to_write[0][0]:
            data = to_write.pop(0)[1]
            if typed is not None:
                typed.append((int(time.time() * 1000), data))
            queue, position = queue[position:] + data, 0
        while position < len(queue):
            try:
                position += os.write(fd, queue[position:position + 65536])
            except (BlockingIOError, InterruptedError):
                break
        for _ in sel.select(0.002 if position < len(queue) else 0.02):
            try:
                data = os.read(fd, 65536)
            except (BlockingIOError, InterruptedError):
                continue
            except OSError:
                data = b""
            if data:
                out.append((int(time.time() * 1000), data))
    if not gone:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    os.close(fd)
    return out


def received_states(server):
    """The user state numbers mosh-server's -v log says it received."""
    import re
    return [int(m.group(1)) for line in server.server_lines()
            for m in [re.search(r"Received (?:OUT-OF-ORDER )?state (\d+)", line)] if m]


# MARK: experiments

def throwaway(args):
    """Types --count single keys --interval ms apart (no expiry) and reports
    when each reached the remote command."""
    for client in clients(args.client):
        server = Server()
        proxy = Proxy(server.port)
        proxy.start()
        try:
            steps = ["wait:1500", f"typeeach:{args.count}:{args.interval}", "wait:20000"]
            lines = run_client(client, proxy, server, 600_000, steps)
            sent = sent_lines(lines)
            got = server.received()
            latencies = [arrived - typed for (typed, _), (arrived, _) in zip(sent, got)]
            in_order = [byte for _, byte in got] == [key[0] for _, key in sent][:len(got)]
            late = [i for i, ms in enumerate(latencies) if ms > 1000]
            print(f"{client}: typed {len(sent)}, remote read {len(got)}, in order {in_order}")
            if latencies:
                ordered = sorted(latencies)
                print(f"  latency ms: p50 {ordered[len(ordered) // 2]}, "
                      f"p99 {ordered[int(len(ordered) * 0.99)]}, max {ordered[-1]}")
            if late:
                print(f"  first key over 1 s: #{late[0] + 1} ({latencies[late[0]]} ms); "
                      f"{len(late)} keys over 1 s")
            print(f"  server alive {server.alive()}")
        finally:
            proxy.stop()
            server.stop()


def ttl(args):
    """Acks lost for 14 s after typing 'abcde' (TTL 10 s), then 'x' typed
    into the outage, then the return path heals and 'yz' is typed."""
    for client in clients(args.client):
        server = Server()
        proxy = Proxy(server.port)
        proxy.start()
        try:
            steps = ["wait:2000", "type:6162636465", "wait:12000", "type:78", "wait:4000",
                     "type:797a", "wait:3000"]

            def down_off():
                proxy.drop_down = True

            def down_on():
                proxy.drop_down = False

            lines = run_client(client, proxy, server, 10_000, steps,
                               [(1.9, down_off), (16.0, down_on)])
            got = server.received()
            text = bytes(byte for _, byte in got)
            alive = server.alive()
            print(f"{client}: remote read {text!r}, server alive {alive}")
            for when, keys in sent_lines(lines):
                arrived = [ms for ms, byte in got if byte in keys]
                print(f"  typed {keys!r}: reached the remote "
                      f"{'%d ms later' % (arrived[0] - when) if arrived else 'never'}")
        finally:
            proxy.stop()
            server.stop()


def redraw(args):
    """A static 22-line screen; after the first sync, requestFullRedraw.
    A full repaint starts with ESC[H ESC[2J and redraws every line."""
    for client in clients(args.client):
        server = Server(screen="lines")
        proxy = Proxy(server.port)
        proxy.start()
        try:
            steps = ["wait:2500", "mark:redraw", "redraw", "wait:3000"]
            lines = run_client(client, proxy, server, 10_000, steps)
            marker = next(int(f[1]) for f in lines if f[0] == "mark")
            before = b"".join(data for when, data in frames(lines) if when < marker)
            after = b"".join(data for when, data in frames(lines) if when >= marker)
            print(f"{client}: initial sync {len(before)} bytes "
                  f"(clears screen: {b'[2J' in before}, lines drawn: {before.count(b'line ')})")
            print(f"  after redraw {len(after)} bytes "
                  f"(clears screen: {b'[2J' in after}, lines drawn: {after.count(b'line ')})")
            print(f"  after redraw: {after[:160]!r}")
        finally:
            proxy.stop()
            server.stop()


def idle(args):
    """A connected client that never types. (1) The server runs with
    MOSH_SERVER_NETWORK_TMOUT=8: does it outlive 20 s of idle? mosh-server
    dates its client by the newest user state it received. (2) After 14 s
    of idle the remote prints PING, and the frame carrying it is dropped:
    how long until the server sends it again? mosh-server retries at frame
    rate only within 10 s (ACTIVE_RETRY_TIMEOUT) of the client's newest
    state, then at its 3 s ack tick."""
    for client in clients(args.client):
        server = Server(tmout=8, verbose=True)
        proxy = Proxy(server.port)
        proxy.start()
        died = []

        def watch(started=time.time()):
            while not died and time.time() < started + 25:
                if not server.alive():
                    died.append(time.time() - started)
                time.sleep(0.05)

        watcher = threading.Thread(target=watch, daemon=True)
        try:
            watcher.start()
            if client == "mosh":
                run_mosh_client(proxy, server, 20)
            else:
                run_client(client, proxy, server, 10_000, ["wait:20000"])
            watcher.join()
            states = received_states(server)
            idle_line = next((line for line in server.server_lines() if "Network idle" in line), None)
            print(f"{client}: TMOUT=8, 20 s idle: server "
                  + (f"exited at {died[0]:.1f} s ({idle_line!r})" if died else "alive at 20 s"))
            print(f"  user states the server received: {len(states)}, newest {max(states, default=0)}")
        finally:
            proxy.stop()
            server.stop()

        server = Server(screen="ping:14000", verbose=True)
        proxy = Proxy(server.port)
        proxy.start()
        window = []

        def drop_ping():
            deadline = time.time() + 10
            while "start" not in server.events() and time.time() < deadline:
                time.sleep(0.01)
            start = server.events().get("start")
            if start is None:
                return
            on, off = (start + 14_000 - 50) / 1000, (start + 14_000 + 100) / 1000
            time.sleep(max(0, on - time.time()))
            proxy.drop_down = True
            time.sleep(max(0, off - time.time()))
            proxy.drop_down = False
            window.extend([on, off])

        dropper = threading.Thread(target=drop_ping, daemon=True)
        try:
            dropper.start()
            if client == "mosh":
                shown = run_mosh_client(proxy, server, 19)
            else:
                lines = run_client(client, proxy, server, 10_000, ["wait:19000"])
                shown = frames(lines)
            dropper.join()
            ping = server.events().get("ping")
            dropped = [r for r in proxy.record if r[1] == "down" and r[3]]
            arrived = next((ms for ms, data in shown if b"PING" in data), None)
            states = received_states(server)
            print(f"{client}: PING printed after 14 s of idle; {len(dropped)} server datagram(s) "
                  f"dropped from -50 to +100 ms")
            print(f"  PING reached the client {arrived - ping} ms after it was printed"
                  if arrived and ping else "  PING never reached the client")
            print(f"  user states the server received: {len(states)}, newest {max(states, default=0)}")
        finally:
            proxy.stop()
            server.stop()


def paste_bytes(size, pattern):
    if pattern == "periodic":
        return bytes(0x21 + i % 94 for i in range(size))
    table = bytes(0x21 + i % 94 for i in range(256))
    return os.urandom(size).translate(table)


def instruction_size(paste, old=1, new=2, ack=1):
    """Bytes of the client's protobuf instruction carrying `paste` as one
    keystroke (what mosh-server inflates), no chaff."""
    def varint(n):
        out = 1
        while n >= 128:
            n >>= 7
            out += 1
        return out

    def field(length):
        return 1 + varint(length) + length

    keys = field(paste)
    diff = field(field(keys))
    return sum(1 + varint(v) for v in (2, old, new, ack, old)) + field(diff)


def paste(args):
    """One paste of each --sizes, as one input: does the remote get it, in
    order, and when? mosh-server inflates an instruction into a 4 MiB buffer
    (compressor.cc); what it refuses it logs as a "Crypto exception" and
    drops. Also: how many bytes the client sent (in all, and until the last
    byte landed), and mosh-server's peak RSS. mosh-client (`--client mosh`)
    gets the paste typed into its pty."""
    for size in [int(n) for n in args.sizes.split(",")]:
        data = paste_bytes(size, args.pattern)
        for client in clients(args.client):
            server = Server(screen="raw", verbose=True)
            proxy = Proxy(server.port, loss=args.loss, delay_ms=args.delay, seed=args.seed)
            proxy.start()
            path = os.path.join(server.dir, "paste")
            with open(path, "wb") as f:
                f.write(data)
            peak = [0]
            stopping = []

            def watch():
                while not stopping:
                    peak[0] = max(peak[0], server.rss_kb())
                    time.sleep(0.1)

            watcher = threading.Thread(target=watch, daemon=True)
            try:
                watcher.start()
                if client == "mosh":
                    typed = []
                    run_mosh_client(proxy, server, 1.5 + args.wait / 1000,
                                    writes=[(1.5, data)], typed=typed)
                    pasted = typed[0][0]
                else:
                    lines = run_client(client, proxy, server, 10_000,
                                       ["wait:1500", f"pastefile:{path}", f"wait:{args.wait}"])
                    pasted = next(int(f[1]) for f in lines if f[0] == "paste")
                stopping.append(True)
                watcher.join()
                with open(server.log, "rb") as f:
                    got = f.read() if os.path.exists(server.log) else b""
                timing = []
                if os.path.exists(server.log + ".t"):
                    with open(server.log + ".t") as f:
                        timing = [tuple(map(int, line.split())) for line in f if line.strip()]
                refusals = [line for line in server.server_lines() if "Crypto exception" in line]
                refused = len(refusals)
                up = [r for r in proxy.record if r[1] == "up" and r[0] >= pasted]
                landed = timing[-1][0] if timing and got == data else None
                until = [r for r in up if landed is not None and r[0] <= landed]
                print(f"{client}: paste of {size} bytes ({args.pattern}; instruction "
                      f"{instruction_size(size)} bytes, mosh's buffer {2048 * 2048})")
                print(f"  remote read {len(got)} bytes, "
                      f"{'the whole paste in order' if got == data else 'a prefix' if data.startswith(got) and got else 'NOT the paste'}"
                      + (f", the last {timing[-1][0] - pasted} ms after the paste" if timing else ""))
                print(f"  mosh-server refused {refused} instruction(s) (\"Crypto exception\"); "
                      f"client sent {len(up)} datagrams, {sum(len(r[2]) for r in up)} bytes "
                      f"after the paste; server alive {server.alive()}, peak RSS {peak[0] // 1024} MiB")
                if landed is not None:
                    sent_until = sum(len(r[2]) for r in until)
                    print(f"  until the last byte landed: {len(until)} datagrams, {sent_until} bytes "
                          f"({sent_until / size:.2f}x the paste)")
                if refusals:
                    print(f"  {refusals[0][:200]}")
            finally:
                stopping.append(True)
                proxy.stop()
                server.stop()


def typing(args):
    """--count keys --interval ms apart (byte i is 0x21 + i % 94), through a
    proxy that drops --loss of the datagrams each way and holds each one
    --delay ms: the bytes and datagrams each client sent from the first key
    to --tail ms after the last, and whether, in what order and how late
    each key reached the remote command. mosh-client gets the keys typed
    into its pty."""
    lead = 1.5
    for client in clients(args.client):
        server = Server()
        proxy = Proxy(server.port, loss=args.loss, delay_ms=args.delay, seed=args.seed)
        proxy.start()
        try:
            if client == "mosh":
                typed = []
                writes = [(lead + i * args.interval / 1000, bytes([0x21 + i % 94]))
                          for i in range(args.count)]
                run_mosh_client(proxy, server, lead + (args.count * args.interval + args.tail) / 1000,
                                writes=writes, typed=typed)
                sent = typed
            else:
                lines = run_client(client, proxy, server, 600_000,
                                   [f"wait:{int(lead * 1000)}", f"typeeach:{args.count}:{args.interval}",
                                    f"wait:{args.tail}"])
                sent = sent_lines(lines)
            got = server.received()
            first = sent[0][0] if sent else 0
            up = [r for r in proxy.record if r[1] == "up" and r[0] >= first]
            down = [r for r in proxy.record if r[1] == "down" and r[0] >= first]
            keys = [key[0] for _, key in sent]
            in_order = [byte for _, byte in got] == keys[:len(got)]
            latencies = sorted(arrived - typed_at for (typed_at, _), (arrived, _) in zip(sent, got))
            up_bytes = sum(len(r[2]) for r in up)
            print(f"{client}: {len(sent)} keys {args.interval} ms apart, loss {args.loss:.0%} each way, "
                  f"delay {args.delay} ms each way")
            print(f"  remote read {len(got)}, in order {in_order}"
                  + (f"; latency ms p50 {latencies[len(latencies) // 2]}, "
                     f"p99 {latencies[int(len(latencies) * 0.99)]}, max {latencies[-1]}" if latencies else ""))
            print(f"  client sent {len(up)} datagrams, {up_bytes} bytes "
                  f"({up_bytes / max(1, len(sent)):.0f} bytes a key; {sum(r[3] for r in up)} dropped); "
                  f"server sent {len(down)} datagrams, {sum(len(r[2]) for r in down)} bytes")
        finally:
            proxy.stop()
            server.stop()


def exit_(args):
    """The remote command exits 2 s after the client's first contact (as
    `exit` in a shell does): mosh-server sends its shutdown state and quits
    once it is acked. When does the client report that the session ended,
    and what did it send after it (for --wait ms)? mosh-client exits."""
    for client in clients(args.client):
        server = Server(screen="exit:2000", verbose=True)
        proxy = Proxy(server.port)
        proxy.start()
        try:
            if client == "mosh":
                exited = []
                shown = run_mosh_client(proxy, server, args.wait / 1000, exited=exited)
                ended = exited[0] if exited else None
                lines = []
            else:
                lines = run_client(client, proxy, server, 10_000, [f"wait:{args.wait}"])
                shown = frames(lines)
                ended = next((int(f[1]) for f in lines if f[0] == "ended"), None)
            gone = server.events().get("exit")
            # mosh-server quits once it read the client's ack of its shutdown
            # state (or after 16 tries / 10 s without one).
            deadline = time.time() + 12
            while server.alive() and time.time() < deadline:
                time.sleep(0.01)
            server_gone = None if server.alive() else int(time.time() * 1000)
            bye = next((ms for ms, data in shown if b"bye" in data), None)
            after = [r for r in proxy.record if r[1] == "up" and gone and r[0] > gone]
            dead = [int(f[1]) for f in lines if f[0] == "returnpathdead"]
            rebuilds = [int(f[1]) for f in lines if f[0] == "rebuild"]
            print(f"{client}: the remote command exited"
                  + (f"; 'bye' shown {bye - gone} ms later" if bye and gone else "; 'bye' never shown"))
            print("  " + (f"the client reported the session ended {ended - gone} ms after the exit"
                          if ended and gone else
                          f"the client never reported the session ended ({args.wait / 1000:.0f} s)"))
            print(f"  client sent {len(after)} datagrams after the exit"
                  + (f", rebuilt its flow {len(rebuilds)} times (first {rebuilds[0] - gone} ms after)"
                     if rebuilds and gone else "")
                  + (f", reported the return path dead {dead[0] - gone} ms after" if dead and gone else ""))
            print("  " + (f"mosh-server had quit by {server_gone - gone} ms after the exit"
                          if server_gone and gone else "mosh-server still running"))
        finally:
            proxy.stop()
            server.stop()


def refused(args):
    """The proxy's port is gone for 2 s, then back: the client's datagrams
    get ICMP port unreachable meanwhile, as from a NAT that lost the
    mapping or a server that went away and came back. The remote prints a
    line every 500 ms; a key is typed while the port is gone. How soon after
    the port is back does the client show output again? A client whose
    receive loop ended on that error would wait for its liveness check to
    rebuild the flow; on macOS 27 NWConnection holds the ECONNREFUSED until
    the flow is cancelled, so this is a regression check there."""
    for client in clients(args.client):
        server = Server(screen="tick:500")
        proxy = Proxy(server.port)
        proxy.start()
        marks = {}

        def gone():
            proxy.port_gone = True
            marks["gone"] = int(time.time() * 1000)

        def back():
            proxy.port_gone = False
            marks["back"] = int(time.time() * 1000)

        try:
            # A key typed while the port is gone: its datagram is what draws
            # the ICMP error.
            schedule = [(4.0, gone), (6.0, back)]
            if client == "mosh":
                shown = run_mosh_client(proxy, server, 22, schedule, writes=[(5.0, b"a")])
                lines = []
            else:
                lines = run_client(client, proxy, server, 10_000,
                                   ["wait:5000", "type:61", "wait:17000"], schedule)
                shown = frames(lines)
            again = [ms for ms, data in shown if ms > marks["back"] and b"tick" in data]
            rebuilds = [int(f[1]) - marks["back"] for f in lines if f[0] == "rebuild"]
            print(f"{client}: the port was gone 2 s; output again "
                  + (f"{again[0] - marks['back']} ms after it was back" if again else "never (16 s)")
                  + (f"; flow rebuilt at {rebuilds} ms after" if rebuilds else ""))
        finally:
            proxy.stop()
            server.stop()


def rebuild(args):
    """Three forced resumes 600 ms apart (a new flow each, as after a long
    suspension) while the remote prints a line every 100 ms: does output
    keep coming on each flow (one socket read at a time)? The Swift client's
    cancelled NWConnection completed its pending receive with neither data
    nor error, which armed a second receive loop on the replacement flow,
    one more per rebuild."""
    for client in clients(args.client):
        server = Server(screen="tick:100")
        proxy = Proxy(server.port)
        proxy.start()
        try:
            lines = run_client(client, proxy, server, 10_000,
                               ["wait:1500", "rebuild", "wait:600", "rebuild", "wait:600",
                                "rebuild", "wait:2000"])
            text = b"".join(data for _, data in frames(lines)).decode(errors="replace")
            import re
            ticks = [int(n) for n in re.findall(r"tick (\d+)", text)]
            rebuilds = sum(1 for f in lines if f[0] == "rebuild")
            receives = next((f[1] for f in lines if f[0] == "receives"), None)
            diag = next((" ".join(f[1:]) for f in lines if f[0] == "diag"), "")
            print(f"{client}: {rebuilds} rebuilds; last line shown: tick {max(ticks, default=0)}; {diag}")
            if receives:
                print(f"  most receives pending at once, per flow: {receives}")
        finally:
            proxy.stop()
            server.stop()


def capture(args):
    """One session per client (typing, a resize, a screen of output);
    writes the key and every datagram both ways for the wire fixtures."""
    for client in clients(args.client):
        server = Server(screen="lines")
        proxy = Proxy(server.port)
        proxy.start()
        try:
            steps = ["wait:1500", "type:6c73202d6c610d", "wait:300", "resize:100:30",
                     "wait:500", "typeeach:40:20", "wait:300", "redraw", "wait:1500"]
            run_client(client, proxy, server, 10_000, steps)
        finally:
            proxy.stop()
            server.stop()
        os.makedirs(args.out, exist_ok=True)
        path = os.path.join(args.out, f"session-{client}.txt")
        with open(path, "w") as f:
            f.write(f"# mosh-server 1.4.0 and the {client} client, recorded by "
                    f"harness/live.py capture\n")
            f.write(f"key {server.key}\n")
            for ms, direction, data, dropped in proxy.record:
                f.write(f"{direction} {data.hex()}\n")
        up = sum(1 for r in proxy.record if r[1] == "up")
        print(f"{path}: {up} up, {len(proxy.record) - up} down")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("experiment",
                        choices=["throwaway", "ttl", "redraw", "idle", "paste", "typing", "exit",
                                 "refused", "rebuild", "capture"])
    parser.add_argument("--client", choices=["rust", "mosh", "all"], default="rust")
    parser.add_argument("--count", type=int, default=1100)
    parser.add_argument("--interval", type=int, default=15)
    parser.add_argument("--sizes", default="4194000,4194500")
    parser.add_argument("--pattern", choices=["periodic", "random"], default="periodic")
    parser.add_argument("--wait", type=int, default=25_000)
    parser.add_argument("--loss", type=float, default=0.0)
    parser.add_argument("--delay", type=int, default=0)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--tail", type=int, default=5_000)
    parser.add_argument("--out", default=os.path.join(CRATE, "tests", "fixtures", "live"))
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    if not args.no_build:
        build()
    {"throwaway": throwaway, "ttl": ttl, "redraw": redraw, "idle": idle, "paste": paste,
     "typing": typing, "exit": exit_, "refused": refused, "rebuild": rebuild,
     "capture": capture}[args.experiment](args)


if __name__ == "__main__":
    main()
