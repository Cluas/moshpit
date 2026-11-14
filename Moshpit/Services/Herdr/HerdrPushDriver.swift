import Foundation

/// Builds the program of the herdr socket pump's terminal channel.
enum HerdrPushBoot {

    /// The pump, PTY-shaped: kill the line discipline first (echo would feed
    /// our own JSON back into the parser, and canonical mode truncates lines
    /// at 4096 bytes — the same two hazards the frame channel documents),
    /// then replace the shell with a python bridge to herdr's JSON socket.
    ///
    /// Why python3: present on effectively every host that runs coding
    /// agents, and it's PROBED (`HostCapabilities.hasPython3`) rather than
    /// assumed — a host without it simply stays on polling. Why not socat:
    /// far less commonly installed, and one probe is cheaper than two.
    ///
    /// Shape constraints, all load-bearing:
    ///   - single-line — a raw newline would submit the shell command early,
    ///     so the real script hides inside an `exec("…\n…")` string where
    ///     `\n` is python's escape, not a byte on the wire;
    ///   - no single quotes in the python body — the whole `-c` argument
    ///     rides inside shell single quotes;
    ///   - `exec` replaces the shell, so channel teardown kills the pump
    ///     rather than orphaning it behind a login shell;
    ///   - python3 found as the probe finds it, with
    ///     ``HostCapabilities/extraPathDirs`` after the PATH the command gets.
    ///
    /// The push channel's own program (`requestPTY(command:)`, run as
    /// `sh -c`), never a line typed into the login shell the channel would
    /// open (B267): a shell still starting swallowed it — one that flushes
    /// pending input (Powerlevel10k's instant prompt) — and push stayed off
    /// for the session.
    /// Printed by the pump the moment it holds a socket.
    ///
    /// Load-bearing, not decoration. Writing to this channel before python
    /// holds the socket means writing to nobody (a login shell, when the
    /// pump was typed into one) — and the subscribe that gets lost is never
    /// retried, so the
    /// request simply times out and push is written off as unavailable for
    /// the whole session. Measured locally: subscribing immediately after the
    /// boot line failed every time, at 3s it succeeded every time. That is
    /// how push came to be silently dead on every host while still costing a
    /// second SSH connection and a 10s timeout per connect.
    ///
    /// A bare word, not JSON: the whole python body rides inside shell single
    /// quotes inside a python string, and every `"` in it needs two levels of
    /// escaping. A marker with no quotes in it cannot get that wrong.
    static let readyMarker = "MOSHPIT_PUMP_READY"

    static func bootLine() -> String {
        let script =
            "import os,socket,threading\\n" +
            "s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)\\n" +
            "s.connect(os.path.expanduser(\\\"~/.config/herdr/herdr.sock\\\"))\\n" +
            "os.write(1,b\\\"\(readyMarker)\\\\n\\\")\\n" +
            "def up():\\n" +
            " while True:\\n" +
            "  d=os.read(0,65536)\\n" +
            "  if not d: break\\n" +
            "  s.sendall(d)\\n" +
            " s.shutdown(socket.SHUT_WR)\\n" +
            "threading.Thread(target=up,daemon=True).start()\\n" +
            "while True:\\n" +
            " d=s.recv(65536)\\n" +
            " if not d: break\\n" +
            " os.write(1,d)"
        // One word for any login shell (B138): hand-quoted, fish read the
        // `\\n` the program holds inside single quotes as `\n`, and python
        // then met a raw newline inside a bytes literal.
        return "stty raw -echo; PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" exec python3 -c "
            + Shell.quote("exec(\"\(script)\")")
    }
}

/// Turns herdr's event stream into control-plane invalidations.
///
/// Phase 0 push mode (docs/design/roaming-transport.md): a PTY-hosted pump
/// (``HerdrPushBoot/bootLine()``) bridges this device to the host's
/// `~/.config/herdr/herdr.sock`; this driver subscribes to every GLOBAL
/// event kind and squeezes the stream into one debounced "something changed"
/// signal, which the owner answers with an immediate snapshot read
/// (`HerdrControlClient.quicken`). The poll timer stays alive underneath as
/// the safety net — push narrows the update window from the 2–8s cadence to
/// ~instant; it does not replace polling's correctness.
///
/// Per-pane kinds (`pane.agent_status_changed`, `pane.scroll_changed`,
/// `pane.output_matched`) are deliberately NOT subscribed: they require a
/// `pane_id` filter — a bare subscription is an `invalid_request`, after
/// which the server hangs up on the whole pipe — and pane churn would mean
/// resubscribing forever. Agent-status flips still surface promptly: they
/// bump the pane's revision and arrive as global `pane.updated` events
/// (observed live against 0.8.0), and the safety-net poll catches anything
/// that doesn't.
@MainActor
final class HerdrPushDriver {

    /// Every subscription kind that takes no filter, i.e. the full schema
    /// minus the three pane-scoped ones. Kept as data so the test can verify
    /// the exclusion — subscribing a pane-scoped kind bare doesn't degrade,
    /// it KILLS the connection.
    static let globalKinds: [String] = [
        "workspace.created", "workspace.updated", "workspace.metadata_updated",
        "workspace.renamed", "workspace.moved", "workspace.reordered",
        "workspace.closed", "workspace.focused",
        "worktree.created", "worktree.opened", "worktree.removed",
        "tab.created", "tab.closed", "tab.renamed", "tab.moved", "tab.focused",
        "pane.created", "pane.closed", "pane.updated", "pane.focused",
        "pane.moved", "pane.exited", "pane.agent_detected",
        "layout.updated",
    ]

    /// The kinds that must never appear in ``globalKinds``.
    static let paneScopedKinds: Set<String> = [
        "pane.agent_status_changed", "pane.scroll_changed", "pane.output_matched",
    ]

    /// Trailing debounce for the invalidate signal — each event RESETS it, so
    /// a burst costs one read after the burst goes quiet.
    ///
    /// It used to only look like a debounce: the guard was "is a timer already
    /// running", which fires once per interval for as long as events keep
    /// coming. Measured against a real server the moment push actually came
    /// up: the subscribe's own bootstrap replay (one synthetic event per
    /// existing workspace/tab/pane, by design) produced **37 snapshot reads in
    /// 8 seconds**, each one a fresh exec channel and a fork on the host.
    /// Push is supposed to REMOVE polling, not amplify it.
    static let debounce: Duration = .milliseconds(250)

    /// …but a debounce that only ever resets starves under a stream that
    /// never stops (an agent printing output bumps its pane on every chunk).
    /// Never let a burst defer the read longer than this.
    static let maxDeferral: TimeInterval = 2

    private let client: HerdrSocketClient
    private let onInvalidate: @MainActor () -> Void
    private var eventLoop: Task<Void, Never>?
    private var pendingInvalidate: Task<Void, Never>?

    /// True from a successful subscribe until push ends (``onEnded``) or the
    /// owner shuts it down.
    private(set) var isActive = false

    /// Told once when push that was up ends on its own: its transport, or the
    /// stream under it, finished — a herdr server restart closes the socket
    /// under the pump, and the pump exits. Not for ``shutdown()``: the owner
    /// knows that one. The owner must stop counting on push: its poll had
    /// eased to the 45 s safety net, and with no events coming agent states
    /// and the tree lagged by up to 45 s for the rest of the session (B188).
    var onEnded: (@MainActor () -> Void)?

    /// `requestTimeout` bounds the subscribe handshake — the "no python3 on
    /// the host" failure shows up as silence (the boot line's error text is
    /// skipped as shell noise), so this deadline IS that failure's detector.
    /// Injectable so tests can exercise the failure path without a 10s wait.
    /// How long to wait for the pump to say it is connected. Generous: it
    /// covers a login shell's rc files plus a python start on a slow host,
    /// and the cost of being wrong is losing push for the session.
    private let pumpTimeout: Duration

    init(requestTimeout: Duration = .seconds(10),
         pumpTimeout: Duration = .seconds(15),
         write: @escaping @Sendable (Data) async throws -> Void,
         onInvalidate: @escaping @MainActor () -> Void) {
        self.client = HerdrSocketClient(requestTimeout: requestTimeout, write: write)
        self.pumpTimeout = pumpTimeout
        self.onInvalidate = onInvalidate
    }

    /// Inbound transport bytes (the owner pumps its SSH dataStream here).
    func feed(_ data: Data) async {
        await client.feed(data)
    }

    /// Subscribe to the global kinds. `false` means push never came up —
    /// missing python3 (the boot line's error text is skipped as noise and
    /// the request times out), a wedged pipe, or a server refusal — and the
    /// owner stays on pure polling. Never throws: push is an upgrade, not a
    /// dependency.
    func activate() async -> Bool {
        // The pump has to own stdin before anything we write reaches herdr;
        // until then the login shell is still there, and it eats whatever it
        // is given. See `HerdrPushBoot.readyMarker`.
        guard await client.waitForPump(timeout: pumpTimeout) else { return false }
        do {
            try await client.subscribe(Self.globalKinds)
        } catch {
            return false
        }
        isActive = true
        eventLoop = Task { [weak self] in
            guard let client = self?.client else { return }
            for await _ in client.events {
                self?.scheduleInvalidate()
            }
            // Stream finished: the transport is gone (`transportEnded`), or
            // the owner shut push down — cancelled, and told nothing.
            guard let self, !Task.isCancelled else { return }
            self.ended()
        }
        return true
    }

    /// The transport under the pump ended — the owner's read of it finished.
    /// Whatever waits on it fails and the event stream ends; push that was up
    /// is over, and ``onEnded`` says so.
    func transportEnded() async {
        await client.finishInput()
    }

    private func ended() {
        guard isActive else { return }
        isActive = false
        pendingInvalidate?.cancel()
        pendingInvalidate = nil
        burstStartedAt = nil
        onEnded?()
    }

    /// Start of the burst the current debounce belongs to, for the cap above.
    private var burstStartedAt: Date?

    private func scheduleInvalidate() {
        let now = Date()
        let started = burstStartedAt ?? now
        burstStartedAt = started
        guard now.timeIntervalSince(started) < Self.maxDeferral else {
            fireInvalidate()
            return
        }
        pendingInvalidate?.cancel()
        pendingInvalidate = Task { [weak self] in
            try? await Task.sleep(for: Self.debounce)
            guard let self, !Task.isCancelled else { return }
            self.fireInvalidate()
        }
    }

    private func fireInvalidate() {
        pendingInvalidate?.cancel()
        pendingInvalidate = nil
        burstStartedAt = nil
        onInvalidate()
    }

    func shutdown() async {
        eventLoop?.cancel()
        eventLoop = nil
        pendingInvalidate?.cancel()
        pendingInvalidate = nil
        await client.finishInput()
        isActive = false
    }
}
