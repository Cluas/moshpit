import Foundation

/// One message from `herdr terminal session control`.
enum HerdrFrame: Equatable {
    /// A screen update. `bytes` are terminal escape sequences ready to feed
    /// straight into an emulator.
    ///
    /// Note these are **absolutely-positioned cell updates** (`ESC[3;6H` per
    /// run), not an append-only byte stream — herdr paints a screen rather
    /// than emitting a log. So they reproduce the pane exactly when fed to
    /// SwiftTerm, but they never accumulate into meaningful local scrollback:
    /// history has to come from herdr's own buffer via `terminal.scroll`.
    case screen(seq: UInt64, width: Int, height: Int, full: Bool, bytes: Data)
    /// The channel ended server-side (pane closed, server stopped).
    case closed(reason: String?)
    /// Our own retarget loop announcing that it is up and reading targets
    /// (``HerdrFrameCommand/readyMarker``). Not herdr's — ours, printed by
    /// the boot line before its first attach. Seeing it is what licenses the
    /// one-word retarget; not seeing it is what triggers the fallback to the
    /// one-shot start line.
    case loopReady
    /// herdr refused or aborted the attach and SAID SO — a diagnostic that
    /// must reach the user, not the byte stream. Two wire shapes produce it
    /// (both observed live against herdr 0.7/0.8 version skew): a JSON
    /// `{"error":{"code":…,"message":…}}` object, and the CLI's plain-text
    /// `herdr: server rejected handshake (version 16): …` line, which is not
    /// JSON at all. Before this case both were silently dropped, leaving a
    /// black pane with no explanation (and, until the click-recursion fix, a
    /// crash on the first tap).
    case fault(message: String)

    /// The attach CLI saw the server hang up mid-handshake — the only place
    /// `terminal session control` prints this (a drop AFTER the handshake
    /// ends the stream silently). The live cause is client/server version
    /// skew the server can't even parse: herdr 0.9.5 drops a 0.8.0 hello
    /// without a word, so the message names no versions and no direction
    /// (2026-11-11). Worth a `herdr status` before it reaches the user raw.
    static func isHandshakeDrop(_ message: String) -> Bool {
        message.hasPrefix("lost connection to server")
    }

    /// herdr turned ONE line away and the attach that read it goes on:
    /// `terminal session control input ignored: invalid json command: …`
    /// (herdr 0.9.5, probed live — a line that isn't JSON, a resize past
    /// u16, a mouse event to a server before 0.9.2, an attach line read by
    /// an attach that is still alive). Not a refused attach: nothing about
    /// the pane or the channel changed (B117, after the Rust client's
    /// `fault`, `crates/herdr/src/client/attach.rs`).
    static func isInputIgnored(_ message: String) -> Bool {
        message.contains("input ignored")
    }

    /// A `terminal.closed` whose reason says another client has the pane:
    /// it took our attach over ("terminal attach taken over"), or held the
    /// terminal when a plain attach went for it ("… already has an attached
    /// client; retry with --takeover") — herdr 0.9.5, measured.
    static func isHeldElsewhere(_ reason: String?) -> Bool {
        guard let reason else { return false }
        return reason.contains("taken over") || HerdrLaunch.heldElsewhere(reason)
    }
}

/// Incremental newline-delimited JSON parser for the frame channel.
///
/// Tolerant by design. The channel runs inside a login shell on a PTY (the
/// only writable long-lived channel Citadel exposes), so the stream can carry
/// things that aren't frames: a profile's banner before the command starts, a
/// shell prompt between two targets, an error from a mistyped path. Anything
/// that isn't a JSON object is skipped rather than treated as corruption.
struct HerdrFrameParser {
    /// Hard cap on the pending buffer. A full redraw of a large pane runs to
    /// tens of KB, so this is generous — it exists only so a stream that never
    /// produces a newline (a wedged channel, a binary flood) can't grow until
    /// the app is killed.
    static let maxBufferBytes = 16 * 1024 * 1024

    private var buffer = Data()

    /// Feed raw channel bytes, get back whatever completed.
    mutating func consume(_ data: Data) -> [HerdrFrame] {
        buffer.append(data)
        if buffer.count > Self.maxBufferBytes { buffer.removeAll(keepingCapacity: false) }

        var frames: [HerdrFrame] = []
        while let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
            let line = buffer[buffer.startIndex..<newline]
            buffer.removeSubrange(buffer.startIndex...newline)
            if let frame = Self.parse(line) { frames.append(frame) }
        }
        return frames
    }

    /// Parse one line, or `nil` if it isn't a frame.
    ///
    /// Self-healing on purpose, which is what lets the channel be retargeted
    /// without resetting anything: if the previous target's last line was cut
    /// off mid-write, the leftover glues onto whatever follows, fails to
    /// parse, and is dropped — and the next target opens with a full repaint,
    /// so nothing is permanently missing.
    static func parse<C: Collection>(_ line: C) -> HerdrFrame? where C.Element == UInt8 {
        // Start at the first `{`: a shell prompt with no trailing newline
        // prefixes the next line, and dropping it costs nothing.
        guard let start = line.firstIndex(of: UInt8(ascii: "{")) else {
            // No JSON on the line at all — but the CLI reports a refused
            // handshake as PLAIN TEXT ("herdr: server rejected handshake…"),
            // and swallowing that leaves a black pane with no story. The
            // prefix match is deliberately tight: with `stty raw -echo` in
            // force nothing we write comes back, so a line opening with
            // "herdr: " can only be the binary talking to us.
            if let text = String(bytes: line, encoding: .utf8) {
                // whitespacesAndNewlines, not whitespaces: the PTY delivers
                // \r\n endings and the split leaves the \r on the line.
                let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmed.hasPrefix("herdr: ") {
                    return .fault(message: String(trimmed.dropFirst("herdr: ".count)))
                }
            }
            return nil
        }
        let data = Data(line[start...])
        guard let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return nil }
        // An error envelope has no "type" — herdr answers a rejected command
        // as {"id":…,"error":{"code":…,"message":…}} (protocol_mismatch is
        // the live example). Report it rather than dropping it.
        guard let type = json["type"] as? String else {
            if let error = json["error"] as? [String: Any],
               let message = error["message"] as? String {
                let brief = message.components(separatedBy: "\n").first ?? message
                return .fault(message: brief)
            }
            return nil
        }

        switch type {
        case "terminal.frame":
            guard let encoded = json["bytes"] as? String,
                  let bytes = Data(base64Encoded: encoded) else { return nil }
            return .screen(
                seq: (json["seq"] as? UInt64) ?? 0,
                width: (json["width"] as? Int) ?? 0,
                height: (json["height"] as? Int) ?? 0,
                full: (json["full"] as? Bool) ?? false,
                bytes: bytes)
        case "terminal.closed":
            return .closed(reason: json["reason"] as? String)
        case HerdrFrameCommand.readyType:
            return .loopReady
        default:
            return nil
        }
    }
}

/// The commands the frame channel accepts on stdin, and the shell line that
/// starts it.
enum HerdrFrameCommand {

    /// Boot line for the channel, to be written into an interactive remote
    /// shell (append `\r` at the call site).
    ///
    /// Two non-obvious pieces:
    ///
    /// * **`stty raw -echo`** — mandatory, not hygiene. On a PTY the line
    ///   discipline would echo every JSON command we write back into stdout,
    ///   interleaving it with frames, and canonical mode would truncate our
    ///   base64 input lines at 4096 bytes. Verified locally: with it, zero of
    ///   our commands come back; without it the stream is unusable.
    /// * **no `exec`** — the shell has to survive the command so we can
    ///   retarget to another pane without rebuilding the SSH channel. The
    ///   prompt printed in between never reaches the screen, because only
    ///   decoded frame bytes are fed to the terminal.
    /// * **`--takeover` only when the person chose the pane** (`takeover`: a
    ///   tap, a reply for it, "Open here"). Following the shared focus always
    ///   took over, so a second device lost its pane whenever this one
    ///   followed the focus onto it; a plain attach to a terminal another
    ///   client holds is refused instead (`terminal.closed`, "… already has
    ///   an attached client; retry with --takeover", herdr 0.9.5).
    static func start(target: String, cols: Int, rows: Int, customPath: String?, takeover: Bool) -> String {
        let herdr = HerdrLaunch.attachCommand(customPath: customPath)
        return "stty raw -echo; \(herdr) terminal session control "
            + "\(HerdrLaunch.quote(target)) --cols \(cols) --rows \(rows)"
            + (takeover ? " --takeover" : "")
    }

    /// Where the frame loop (``boot(target:cols:rows:customPath:takeover:connectionId:)``)
    /// records its own pid, per connection: the next connection's boot line
    /// stops the attach a connection that dropped left holding its pane.
    static func loopPidPath(connectionId: String) -> String {
        "$HOME/.moshpit/herdr-frame-\(connectionId).pid"
    }

    /// The `type` of ``readyMarker``, in herdr's own frame namespace-by-
    /// convention. Prefixed `moshpit.` precisely because it is NOT herdr's:
    /// nothing on the server ever emits it.
    static let readyType = "moshpit.ready"

    /// Printed by ``boot(target:cols:rows:customPath:)`` before its first
    /// attach, so the app can tell "the retarget loop is running" from "the
    /// login shell couldn't take that shape".
    static let readyMarker = #"{"type":"\#(readyType)"}"#

    /// Boot line for a channel that can be **retargeted in one word**.
    ///
    /// The one-shot ``start(target:cols:rows:customPath:)`` above is written
    /// at the remote login shell's prompt, and a switch therefore has to wait
    /// for that prompt to come back: on a stock zsh with a themed prompt and
    /// precmd hooks that is ~250ms of pure ceremony per switch, on top of the
    /// round trips (measured against a real herdr 0.8.0: 545ms per switch,
    /// of which ~500ms was the settle and the prompt).
    ///
    /// So the channel runs a reader loop instead. It attaches, and when the
    /// attach ends it reads ONE line — `<target> <cols> <rows>` — and attaches
    /// again. No prompt, no rc files, no `stty` re-negotiation with a line
    /// editor. Same measurement: 40ms per switch.
    ///
    /// Load-bearing details:
    ///
    /// * **The first target rides in as `$1`.** Writing it as a second line
    ///   would race the outer shell, which is still reading the boot line and
    ///   would happily run the target id as a command.
    /// * **`sh -c`, not the login shell's own syntax.** A `while` loop typed
    ///   into fish or csh is a syntax error; a single-quoted argument to `sh`
    ///   is not. The whole script is quoted with ``HerdrLaunch/quote(_:)``, so
    ///   a custom herdr path containing a quote can't break out of it.
    /// * **`stty raw -echo` inside the loop**, not just once: the attach we
    ///   just ran owned the tty and is under no obligation to hand it back in
    ///   the mode we need. Same two hazards as the one-shot line documents.
    /// * **`read … || break`** — EOF means the channel is gone; anything else
    ///   (a failed attach, an unknown target) loops back to read, so one bad
    ///   switch can't strand the session.
    /// * **A line opening with `{` is dropped, and so is the target.** Those
    ///   are OUR commands (resize, release, keystrokes) meant for an attach
    ///   that has already died. Read as a target, each one spawned an attach
    ///   at `{"type":"terminal.resize",…}`, whose "not found" close the app
    ///   counted as an eviction — four attaches in 300 ms behind a dead
    ///   first one (herdr-client.log, 2026-11-11). `set --` rather than
    ///   keeping the old target: re-attaching to what just failed is the
    ///   app's call, not the loop's.
    /// * **`--takeover` only for a target the person chose**: a fourth word
    ///   on the line (`take`, ``retarget(target:cols:rows:takeover:)``); the
    ///   first target's rides in as `$4`. A follow's attach is plain, and
    ///   herdr refuses it while another client holds the terminal.
    /// * **This connection's stale attach is stopped first.** sshd keeps a
    ///   dropped connection's session — its loop and its attach, holding the
    ///   pane — until TCP gives up on it, and a plain attach to the pane this
    ///   device had would be refused by this device's own ghost. The loop
    ///   records its pid (``loopPidPath(connectionId:)``); the next boot
    ///   kills a child of that loop (its command line names this
    ///   connection's pid file) whose command line is `terminal session
    ///   control` — never what reused a pid.
    static func boot(target: String, cols: Int, rows: Int, customPath: String?, takeover: Bool,
                     connectionId: String) -> String {
        "stty raw -echo; /bin/sh -c \(HerdrLaunch.quote(loopScript(customPath: customPath, connectionId: connectionId))) "
            + "moshpit-herdr \(HerdrLaunch.quote(target)) \(cols) \(rows)"
            + (takeover ? " take" : "")
    }

    /// The reader loop of ``boot(target:cols:rows:customPath:takeover:connectionId:)``
    /// as the SSH terminal channel's own program (`requestPTY(command:)`),
    /// with no first target: it says it reads (``readyMarker``) and waits
    /// for one (``retarget(target:cols:rows:takeover:)`` lines). Nothing is
    /// typed into a login shell — there is none (B267): typed into the
    /// channel's interactive login shell, the boot line met shells still
    /// starting (a flushed type-ahead, a plugin re-highlighting every
    /// character, a startup prompt reading stdin), and the channel never
    /// came up. The loop ending (EOF) ends the channel. crates/herdr's
    /// `frame::loop_program`, the same string.
    static func loopProgram(customPath: String?, connectionId: String) -> String {
        "stty raw -echo; exec /bin/sh -c \(HerdrLaunch.quote(loopScript(customPath: customPath, connectionId: connectionId))) moshpit-herdr"
    }

    private static func loopScript(customPath: String?, connectionId: String) -> String {
        let herdr = HerdrLaunch.attachCommand(customPath: customPath)
        let pid = loopPidPath(connectionId: connectionId)
        return "f=\"\(pid)\"; mkdir -p \"$HOME/.moshpit\" 2>/dev/null; "
            + "l=$(cat \"$f\" 2>/dev/null); case \"$l\" in ''|*[!0-9]*) ;; *) "
            + HerdrLaunch.commandLineOf("$l")
            + "case \"$a\" in *\"herdr-frame-\(connectionId).pid\"*) "
            + "for c in $(pgrep -P \"$l\" 2>/dev/null || ps -A -o pid= -o ppid= 2>/dev/null "
            + "| awk -v l=\"$l\" '$2 == l { print $1 }'); do "
            + HerdrLaunch.commandLineOf("$c")
            + "case \"$a\" in *\"terminal session control\"*) kill -9 \"$c\" 2>/dev/null ;; esac; "
            + "done ;; esac ;; esac; "
            + "{ echo $$ > \"$f\"; } 2>/dev/null; "
            + "printf '%s\\n' '\(readyMarker)'; while :; do "
            + "if [ -n \"$1\" ]; then stty raw -echo; "
            + "\(herdr) terminal session control \"$1\" "
            + "--cols \"${2:-80}\" --rows \"${3:-24}\" ${4:+--takeover}; fi; "
            + "read -r t c r k || break; "
            + "case \"$t\" in \"{\"*) set --; continue;; esac; "
            + "set -- \"$t\" \"$c\" \"$r\" \"$k\"; done"
    }

    /// One line for the loop's `read`: where to point next, at what size, and
    /// `take` when the person chose it (`--takeover`). Only valid after
    /// ``boot(target:cols:rows:customPath:takeover:connectionId:)`` announced
    /// itself with ``readyMarker`` — and only once the previous attach has
    /// released, because until then the attach still owns stdin and swallows
    /// this line whole (verified: pipelining it with the release loses it).
    static func retarget(target: String, cols: Int, rows: Int, takeover: Bool) -> String {
        "\(target) \(max(1, cols)) \(max(1, rows))" + (takeover ? " take" : "")
    }

    /// Keystrokes. Sent as base64 so arbitrary bytes — escape sequences, UTF-8
    /// continuation bytes, a pasted NUL — survive the JSON round trip intact.
    static func input(_ data: Data) -> String {
        #"{"type":"terminal.input","bytes":"\#(data.base64EncodedString())"}"#
    }

    static func resize(cols: Int, rows: Int) -> String {
        #"{"type":"terminal.resize","cols":\#(max(1, cols)),"rows":\#(max(1, rows))}"#
    }

    /// Scroll, with the distinction herdr's server needs to route it.
    ///
    /// This is the piece that deletes work rather than adding it: on tmux the
    /// app has to inspect `#{mouse_any_flag}` itself and choose between
    /// sending wheel escapes and driving copy-mode. herdr's server makes that
    /// call — `wheel` becomes a mouse report for an app that grabbed the
    /// mouse, or scrollback movement for one that didn't.
    static func scroll(up: Bool, lines: Int, source: ScrollSource = .wheel) -> String {
        #"{"type":"terminal.scroll","direction":"\#(up ? "up" : "down")","lines":\#(max(1, lines)),"source":"\#(source.rawValue)"}"#
    }

    enum ScrollSource: String {
        /// A drag/wheel gesture — the server decides mouse-report vs scrollback.
        case wheel
        /// PageUp/PageDown semantics, for the keyboard bar's paging keys.
        case pageKey = "page_key"
    }

    /// Detach without killing the pane. Sent before retargeting so the server
    /// releases its exclusive attach owner.
    static let release = #"{"type":"terminal.release"}"#
}
