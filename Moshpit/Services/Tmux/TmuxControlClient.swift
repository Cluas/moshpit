import Foundation

// MARK: - TmuxControlError

/// Reported through ``TmuxControlClient/onProtocolError`` when a recognised
/// `%foo` line cannot be decoded (e.g. malformed `%output` with no pane id).
/// Unknown directives (forward-compat) are deliberately *not* errors.
enum TmuxControlError: Error, Equatable, Sendable {
    case malformedLine(String)
}

// MARK: - TmuxCommandResponse

/// Result of a single tmux command echoed inside a `%begin / %end / %error`
/// block.
struct TmuxCommandResponse: Sendable, Equatable {
    /// `command-id` field from the `%begin` line (tmux uses this as a session-
    /// scoped serial).
    let commandId: Int
    /// `command-num` field — increments per dispatched command.
    let commandNum: Int
    /// Whether tmux closed the block with `%end` (false) or `%error` (true).
    let isError: Bool
    /// Lines tmux emitted between `%begin` and the terminator, in order.
    /// Each line is the raw UTF-8 string with the trailing newline stripped.
    let lines: [String]
    /// The block answers a line this client wrote: bit 1 of the `%begin`
    /// line's flags, `true` when the field is missing or unreadable. tmux
    /// also writes blocks for commands nobody here sent — an
    /// `after-<command>` hook one of ours fired, the late half of an
    /// `if-shell -b` / `run-shell -b` — flagged 0 (tmux 3.6a; `cmdq_guard`
    /// passes `CMDQ_STATE_CONTROL` for a control client's own lines). Those
    /// answer no command of ours (B275, the app's half of B273).
    let fromClient: Bool

    init(commandId: Int, commandNum: Int, isError: Bool, lines: [String], fromClient: Bool = true) {
        self.commandId = commandId
        self.commandNum = commandNum
        self.isError = isError
        self.lines = lines
        self.fromClient = fromClient
    }
}

// MARK: - TmuxControlClient

/// Pure protocol parser for tmux's control mode (`tmux -CC`).
///
/// ### Responsibilities
///   - Buffer raw transport bytes and split on newlines.
///   - Recognise notification lines (`%output`, `%window-add`, etc.) and route
///     them through typed closures.
///   - Track open `%begin … %end / %error` response blocks and surface the
///     batched lines via ``onCommandResponse``.
///   - Decode tmux's octal escape sequences (`\NNN`, `\\`) inside `%output`
///     payloads so callers receive raw byte sequences they can feed straight
///     into SwiftTerm.
///
/// ### Anti-responsibilities
///   - **No state ownership.** This client does not store sessions, windows
///     or panes — that is ``TmuxSessionController``'s job.
///   - **No SwiftUI / Observation.** No `@Published`, no `@Observable`, no
///     `objectWillChange`.
///   - **No buffering of output by pane.** Output flows directly to the
///     callback; if the consumer is not ready yet that's their problem to
///     model.
///   - **No transport.** The caller pumps bytes via ``feed(_:)``; the client
///     never reads the SSH stream itself.
///
/// ### Threading
/// `TmuxControlClient` is an actor. Callers feed bytes via `await feed(_:)`;
/// callbacks fire on the actor's executor — handlers that need MainActor
/// affinity must hop themselves. The Rust parser runs there too, called
/// synchronously from ``feed(_:)``, and its events fire the same callbacks
/// before ``feed(_:)`` returns.
actor TmuxControlClient {

    // MARK: - Callbacks

    /// Invoked with raw, octal-decoded bytes whenever tmux emits
    /// `%output %paneId <data>`. `paneId` is the literal `"%0"` etc.
    var onPaneOutput: (@Sendable (_ paneId: String, _ data: Data) -> Void)?

    /// Invoked for every `%layout-change @windowId <layout> [...]` line.
    /// `zoomed` is true when the window's flags carry `Z` (`*Z`, `-Z`,
    /// `*MZ`, …).
    var onLayoutChange: (@Sendable (_ windowId: String, _ layout: String, _ zoomed: Bool) -> Void)?

    /// Invoked for `%window-add @windowId`.
    var onWindowAdd: (@Sendable (_ windowId: String) -> Void)?

    /// Invoked for `%window-close @windowId`.
    var onWindowClose: (@Sendable (_ windowId: String) -> Void)?

    /// Invoked for `%window-renamed @windowId <name>`.
    var onWindowRenamed: (@Sendable (_ windowId: String, _ name: String) -> Void)?

    /// Invoked for `%session-changed $sessionId <name>`.
    var onSessionChanged: (@Sendable (_ sessionId: String, _ name: String) -> Void)?

    /// Invoked for `%session-renamed $sessionId <name>` — any client's
    /// rename of any session; the name is the rest of the line.
    var onSessionRenamed: (@Sendable (_ sessionId: String, _ name: String) -> Void)?

    /// Invoked for `%session-window-changed $sessionId @windowId`.
    var onSessionWindowChanged: (@Sendable (_ sessionId: String, _ windowId: String) -> Void)?

    /// Invoked for `%sessions-changed`: a session was created or destroyed,
    /// anywhere on the server.
    var onSessionsChanged: (@Sendable () -> Void)?

    /// Invoked for `%unlinked-window-add @windowId`: a window was linked into
    /// a session that is not this client's (a window or a whole session
    /// created elsewhere).
    var onUnlinkedWindowAdd: (@Sendable (_ windowId: String) -> Void)?

    /// Invoked for `%unlinked-window-close @windowId`. tmux 3.6a sends this,
    /// not `%window-close`, for a window killed in this client's own session
    /// too: the window is already unlinked when the notification goes out
    /// (captures/layout.bin, `kill-window`).
    var onUnlinkedWindowClose: (@Sendable (_ windowId: String) -> Void)?

    /// Invoked for `%unlinked-window-renamed @windowId <name>`; the name is
    /// the rest of the line.
    var onUnlinkedWindowRenamed: (@Sendable (_ windowId: String, _ name: String) -> Void)?

    /// Invoked for `%client-session-changed <client> $sessionId <name>`:
    /// ANOTHER client attached to or switched to a session (this client's
    /// own switches arrive as `%session-changed`). The client name is a tty
    /// path or `client-<pid>` and holds no space; the session name is the
    /// rest of the line.
    var onClientSessionChanged: (@Sendable (_ client: String, _ sessionId: String, _ name: String) -> Void)?

    /// Invoked for `%window-pane-changed @windowId %paneId`.
    var onActivePaneChanged: (@Sendable (_ windowId: String, _ paneId: String) -> Void)?

    /// Invoked for `%pause %paneId` — caller should respond with
    /// `refresh-client -A %paneId:+` to resume the stream.
    var onPause: (@Sendable (_ paneId: String) -> Void)?

    /// Invoked for `%continue %paneId`.
    var onContinue: (@Sendable (_ paneId: String) -> Void)?

    /// Invoked for `%client-detached <client>`. The single string is
    /// whatever tmux placed after the directive.
    var onClientDetached: (@Sendable (_ payload: String) -> Void)?

    /// Invoked when tmux closes the control mode session (`%exit [reason]`).
    var onExit: (@Sendable (_ reason: String?) -> Void)?

    /// Invoked at the end of each `%begin … %end` (or `%error`) block.
    var onCommandResponse: (@Sendable (_ response: TmuxCommandResponse) -> Void)?

    /// Invoked when a recognised line cannot be parsed. Unknown `%foo` lines
    /// are *not* errors — they pass through silently for forward-compat.
    var onProtocolError: (@Sendable (_ error: TmuxControlError) -> Void)?

    // MARK: - Internal state

    /// Raw transport bytes after the last `\n` fed so far: a line still on
    /// its way. Complete lines never wait here — ``drainLines(_:)``
    /// dispatches them straight out of the data ``feed(_:)`` was given.
    private var partialLine = Data()

    /// `%begin` data captured at block open; cleared at `%end` / `%error`.
    private var openBlock: (commandId: Int, commandNum: Int, fromClient: Bool, lines: [String])?

    /// The open block's own terminators, `%end <cid> <num>` and
    /// `%error <cid> <num>`, built once at `%begin`: the embedded-terminator
    /// check runs on every line of the block, and a backfill reply is
    /// thousands of lines (B102).
    private var openBlockNeedles: [[UInt8]] = []

    /// Ids of the most recent REAL `%begin`, for the truncated-line recovery
    /// below: command numbers increment per command on a connection, so a
    /// protocol line glued onto a truncated one carries the NEXT number,
    /// while pane content quoting old dumps carries stale ones.
    private var lastBlockCid = 0
    private var lastBlockNum = 0

    /// A reply block cannot legitimately hold more lines than the biggest
    /// capture this app ever requests (a 2 000-line backfill plus a screen).
    /// Past this, the block's terminator was lost in transit (a lossy hop —
    /// e.g. a pty-based tap wrapper under load — can drop the tail of a
    /// line, gluing the next protocol line onto it; 真机取证 2026-08-19) and
    /// the block would otherwise swallow every reply and %output forever:
    /// the frozen-screen, 381-pending-commands failure. Force-close it.
    private let maxBlockLines = 5000

    /// True while the NEXT completed block is the boot command's own reply —
    /// the block tmux emits for the line that entered control mode
    /// (`-CC attach` / `-CC new`), which no caller ever sent through the
    /// command queue. Swallowed here rather than paired positionally
    /// upstream, because ONE stream can carry SEVERAL control sessions:
    /// a preferred-session boot chain
    /// (`tmux -CC attach -t 'x' || while ! tmux -CC attach; do sleep 2; done`)
    /// emits a full `%begin/%error/%exit` for EVERY failed attach and a
    /// fresh banner for the one that succeeds (verified against tmux 3.6a,
    /// 2026-08-19) — a fixed one-slot reservation upstream shifts
    /// command↔response pairing by one per extra banner, which is how the
    /// mosh sidecar's `list-clients` answers came back wrong and the
    /// renderer re-typed its attach line into a live pane. Re-armed by
    /// `%exit`: whatever block follows a control session's end belongs to
    /// the next session's boot line.
    private var awaitingBootBlock = true

    // MARK: - Public API

    /// Feed transport bytes from the SSH session. Safe to call with empty
    /// data; partial lines are buffered until the next call.
    func feed(_ data: Data) {
        guard !data.isEmpty else { return }
        drainLines(data)
    }

    /// Discard any partial line in the buffer and any open response block.
    /// Use when reconnecting or after a protocol error to resync.
    func reset() {
        partialLine.removeAll(keepingCapacity: false)
        openBlock = nil
        awaitingBootBlock = true
    }


    // MARK: - Callback installers

    /// Convenience: install all callbacks in one call from a non-isolated
    /// context. Any nil argument leaves the previously-installed handler in
    /// place; pass an explicit empty closure to clear.
    func setCallbacks(
        onPaneOutput: (@Sendable (String, Data) -> Void)? = nil,
        onLayoutChange: (@Sendable (String, String, Bool) -> Void)? = nil,
        onWindowAdd: (@Sendable (String) -> Void)? = nil,
        onWindowClose: (@Sendable (String) -> Void)? = nil,
        onWindowRenamed: (@Sendable (String, String) -> Void)? = nil,
        onSessionChanged: (@Sendable (String, String) -> Void)? = nil,
        onSessionRenamed: (@Sendable (String, String) -> Void)? = nil,
        onSessionWindowChanged: (@Sendable (String, String) -> Void)? = nil,
        onSessionsChanged: (@Sendable () -> Void)? = nil,
        onUnlinkedWindowAdd: (@Sendable (String) -> Void)? = nil,
        onUnlinkedWindowClose: (@Sendable (String) -> Void)? = nil,
        onUnlinkedWindowRenamed: (@Sendable (String, String) -> Void)? = nil,
        onClientSessionChanged: (@Sendable (String, String, String) -> Void)? = nil,
        onActivePaneChanged: (@Sendable (String, String) -> Void)? = nil,
        onPause: (@Sendable (String) -> Void)? = nil,
        onContinue: (@Sendable (String) -> Void)? = nil,
        onClientDetached: (@Sendable (String) -> Void)? = nil,
        onExit: (@Sendable (String?) -> Void)? = nil,
        onCommandResponse: (@Sendable (TmuxCommandResponse) -> Void)? = nil,
        onProtocolError: (@Sendable (TmuxControlError) -> Void)? = nil
    ) {
        if let onPaneOutput { self.onPaneOutput = onPaneOutput }
        if let onLayoutChange { self.onLayoutChange = onLayoutChange }
        if let onWindowAdd { self.onWindowAdd = onWindowAdd }
        if let onWindowClose { self.onWindowClose = onWindowClose }
        if let onWindowRenamed { self.onWindowRenamed = onWindowRenamed }
        if let onSessionChanged { self.onSessionChanged = onSessionChanged }
        if let onSessionRenamed { self.onSessionRenamed = onSessionRenamed }
        if let onSessionWindowChanged { self.onSessionWindowChanged = onSessionWindowChanged }
        if let onSessionsChanged { self.onSessionsChanged = onSessionsChanged }
        if let onUnlinkedWindowAdd { self.onUnlinkedWindowAdd = onUnlinkedWindowAdd }
        if let onUnlinkedWindowClose { self.onUnlinkedWindowClose = onUnlinkedWindowClose }
        if let onUnlinkedWindowRenamed { self.onUnlinkedWindowRenamed = onUnlinkedWindowRenamed }
        if let onClientSessionChanged { self.onClientSessionChanged = onClientSessionChanged }
        if let onActivePaneChanged { self.onActivePaneChanged = onActivePaneChanged }
        if let onPause { self.onPause = onPause }
        if let onContinue { self.onContinue = onContinue }
        if let onClientDetached { self.onClientDetached = onClientDetached }
        if let onExit { self.onExit = onExit }
        if let onCommandResponse { self.onCommandResponse = onCommandResponse }
        if let onProtocolError { self.onProtocolError = onProtocolError }
    }

    // MARK: - Line extraction

    /// Dispatch every complete line in ``partialLine`` + `data`, and keep
    /// what follows the last `\n` for the next ``feed(_:)``. Lines are
    /// separated by `\n`; one trailing `\r` is stripped so CRLF and LF
    /// transports look identical to the rest of the parser.
    ///
    /// Linear in the bytes fed: complete lines are dispatched as slices of
    /// `data`, never copied; only the NEW bytes are searched for a newline
    /// (the partial line has none); only the trailing partial line is
    /// copied. The old loop kept everything in one buffer and re-copied the
    /// rest of it after every line — O(lines × bytes) per feed, so a single
    /// 3.8 MB feed (a burst of output, a reconnect's backlog) parsed at
    /// 0.6 MB/s on an M2, 40 times slower than the same bytes in 4 KB feeds.
    private func drainLines(_ data: Data) {
        let newline = UInt8(ascii: "\n")
        var rest = data
        if !partialLine.isEmpty {
            guard let end = rest.firstIndex(of: newline) else {
                partialLine.append(rest)
                return
            }
            partialLine.append(rest[..<end])
            let line = partialLine
            partialLine = Data()
            dispatchLine(Self.droppingCarriageReturn(line))
            rest = rest[rest.index(after: end)...]
        }
        while let end = rest.firstIndex(of: newline) {
            // Defer UTF-8 decoding to the dispatcher so we can capture the
            // raw payload of malformed lines.
            dispatchLine(Self.droppingCarriageReturn(rest[..<end]))
            rest = rest[rest.index(after: end)...]
        }
        // A copy, so a short partial line never pins the caller's buffer.
        partialLine = Data(rest)
    }

    /// `line` without one trailing `\r` (CRLF line endings).
    private static func droppingCarriageReturn(_ line: Data) -> Data {
        line.last == UInt8(ascii: "\r") ? line.dropLast() : line
    }

    /// Decode and route a single raw line.
    private func dispatchLine(_ raw: Data) {
        // Inside an open `%begin` block (e.g. a capture-pane scrollback dump):
        // capture lines VERBATIM. We must NOT run decodeLine here — its
        // leading-junk strip truncates any content line that merely *contains* a
        // `%keyword` substring (e.g. "build 80%done", "x%window") down to that
        // suffix, silently corrupting scrollback. Only `%end`/`%error` (which
        // tmux always emits at column 0) terminate the block.
        if openBlock != nil {
            let blockLine = String(data: raw, encoding: .utf8)
                ?? String(decoding: raw, as: UTF8.self)
            // Its bytes, for the terminator checks: protocol syntax is
            // matched on bytes, never on Characters (see "Protocol bytes"
            // at the end of this file).
            let bytes = ArraySlice(blockLine.utf8)
            // Truncated-line recovery, in-block half: a lossy hop can drop a
            // content line's tail, gluing the block's OWN terminator onto it
            // ("<content>%end <cid> <num> 1" as one line). The prefix checks
            // below then miss it and the block never closes. The suffix must
            // echo the open block's EXACT ids to count — content quoting some
            // other dump can't match a live command's unique pair.
            if let (content, terminator) = splitEmbeddedTerminator(bytes) {
                if !content.isEmpty { openBlock?.lines.append(content.string) }
                handleBlockEnd(line: terminator, isError: terminator.hasPrefix("%error"))
                return
            }
            // A terminator must carry the OPEN block's ids to count. tmux
            // always echoes `%end <cid> <num>` matching its `%begin`, so a
            // "%end"-shaped line with different ids is pane CONTENT — a
            // capture-pane frame of a pane that itself displays control-mode
            // text (someone developing against tmux -CC — this app included —
            // has exactly that on screen). Taking one of those as the
            // terminator cut the frame short, spilled its remaining lines
            // into the notification parser, and let a content "%begin" open
            // a phantom block that swallowed the NEXT real reply — from
            // there, command↔response pairing was shifted for good.
            //
            // Checked BEFORE the line cap below: a reply of exactly
            // `maxBlockLines` lines is complete, not runaway — the cap used
            // to fire on its own terminator, delivering the reply as an
            // error and then reporting the real %end as a stray one.
            if terminatesOpenBlock(bytes, prefix: "%end") {
                handleBlockEnd(line: bytes, isError: false)
                return
            }
            if terminatesOpenBlock(bytes, prefix: "%error") {
                handleBlockEnd(line: bytes, isError: true)
                return
            }
            // Runaway block: its terminator was lost outright. Deliver what
            // accumulated as an error (the sender's callback occupies a FIFO
            // slot — silence would shift pairing forever) and re-dispatch the
            // current line as if the block had closed.
            if let block = openBlock, block.lines.count >= maxBlockLines {
                openBlock = nil
                onProtocolError?(.malformedLine(
                    "block #\(block.commandId) exceeded \(maxBlockLines) lines — forcing close"))
                deliverBlock(commandId: block.commandId, commandNum: block.commandNum,
                             isError: true, fromClient: block.fromClient, lines: block.lines)
                dispatchLine(raw)
                return
            }
            openBlock?.lines.append(blockLine)
            return
        }

        // `%output` MUST be handled at the byte level, before any String
        // decoding: its payload is the pane's raw byte stream (tmux only
        // octal-escapes control bytes and backslash — bytes ≥ 0x80 pass
        // through verbatim), and tmux freely flushes MID-character, so a
        // multi-byte UTF-8 sequence can straddle two %output events. Each
        // line is then individually invalid UTF-8, and a String round-trip
        // bakes U+FFFD into the stream ("？？" pairs smeared across Claude
        // Code's borders and CJK echo). SwiftTerm reassembles split
        // sequences across feeds — but only if we hand it the raw bytes.
        if raw.starts(with: Self.outputPrefix) {
            handleOutputRaw(raw)
            return
        }

        // Some DCS-style transports wrap the first burst in `\ePxxxp%begin…`.
        // Skip leading non-`%` bytes when present so we still see the marker.
        // From here on `line` is the decoded line's UTF-8 BYTES: every
        // keyword, delimiter and number below is matched on bytes.
        let line = decodeLine(raw)

        // Outside a block: only `%` lines are notifications. Anything else is
        // stray transport noise (banner, DCS prologue, ssh MOTD) — ignore.
        guard line.hasPrefix("%") else { return }

        if line.hasPrefix("%begin ") {
            handleBegin(line)
        } else if line.hasPrefix("%layout-change ") {
            handleLayoutChange(line)
        } else if line.hasPrefix("%window-add ") {
            handleWindowAdd(line)
        } else if line.hasPrefix("%window-close ") {
            handleWindowClose(line)
        } else if line.hasPrefix("%window-renamed ") {
            handleWindowRenamed(line)
        } else if line.hasPrefix("%session-changed ") {
            handleSessionChanged(line)
        } else if line.hasPrefix("%session-renamed ") {
            handleSessionRenamed(line)
        } else if line.hasPrefix("%session-window-changed ") {
            handleSessionWindowChanged(line)
        } else if line.hasPrefix("%window-pane-changed ") {
            handleWindowPaneChanged(line)
        } else if line.equals("%sessions-changed") || line.hasPrefix("%sessions-changed ") {
            onSessionsChanged?()
        } else if line.hasPrefix("%unlinked-window-add ") {
            handleWindowId(line, prefix: "%unlinked-window-add ", onUnlinkedWindowAdd)
        } else if line.hasPrefix("%unlinked-window-close ") {
            handleWindowId(line, prefix: "%unlinked-window-close ", onUnlinkedWindowClose)
        } else if line.hasPrefix("%unlinked-window-renamed ") {
            handleUnlinkedWindowRenamed(line)
        } else if line.hasPrefix("%client-session-changed ") {
            handleClientSessionChanged(line)
        } else if line.hasPrefix("%pause ") {
            handlePause(line)
        } else if line.hasPrefix("%continue ") {
            handleContinue(line)
        } else if line.hasPrefix("%client-detached") {
            handleClientDetached(line)
        } else if line.equals("%exit") || line.hasPrefix("%exit ") {
            handleExit(line)
        } else if line.hasPrefix("%end ") || line.equals("%end") ||
                  line.hasPrefix("%error ") || line.equals("%error") {
            // Block terminator without a matching `%begin`: ignore but report
            // so logs can surface the desync.
            onProtocolError?(.malformedLine(line.string))
        } else {
            // Unknown directive — forward-compat: silent skip.
        }
    }

    /// UTF-8 decode the line and strip any leading bytes before the first
    /// `%`-starting tmux keyword. This handles transports that prepend DCS
    /// (`\eP1000p…`) or shell banners on the very first line. Returns the
    /// decoded line's UTF-8 bytes.
    private func decodeLine(_ raw: Data) -> ArraySlice<UInt8> {
        let decoded = String(data: raw, encoding: .utf8) ?? String(
            decoding: raw, as: UTF8.self
        )
        let line = ArraySlice(decoded.utf8)
        guard !line.hasPrefix("%") else { return line }
        guard let pct = line.firstIndex(of: UInt8(ascii: "%")) else { return line }
        let candidate = line[pct...]
        if candidate.hasPrefix("%begin") || candidate.hasPrefix("%end") ||
            candidate.hasPrefix("%error") || candidate.hasPrefix("%output") ||
            candidate.hasPrefix("%layout") || candidate.hasPrefix("%window") ||
            candidate.hasPrefix("%session") || candidate.hasPrefix("%pause") ||
            candidate.hasPrefix("%continue") || candidate.hasPrefix("%client") ||
            candidate.hasPrefix("%exit") || candidate.hasPrefix("%unlinked") {
            return candidate
        }
        return line
    }

    // MARK: - Block lifecycle

    /// Truncated-line recovery helpers. A lossy transport hop (measured:
    /// a server-side script(1) tap wrapper dropping pty bytes at 1024-byte
    /// boundaries under load) can cut a line WITHOUT its newline, so the
    /// next protocol line arrives glued to the truncated one and every
    /// prefix-based check misses it. These recover the two glue shapes that
    /// break command↔response pairing for good; pure content damage needs no
    /// recovery (the next repair frame repaints it).

    /// `"<content>%end <cid> <num> …"` where cid/num are the OPEN block's own
    /// ids → `(content, terminator)`. nil when no such suffix.
    private func splitEmbeddedTerminator(
        _ line: ArraySlice<UInt8>
    ) -> (ArraySlice<UInt8>, ArraySlice<UInt8>)? {
        guard openBlock != nil else { return nil }
        for needle in openBlockNeedles {
            guard let range = Self.lastRange(of: needle, nearEndOf: line,
                                             tail: Self.maxTerminatorTail) else { continue }
            // Must run to end-of-line (allowing trailing flags) — a mention
            // mid-line is content. Flags are ASCII digits: a "٣" there is
            // content too.
            let tail = line[range.upperBound...]
            guard tail.count <= Self.maxTerminatorTail,
                  tail.allSatisfy({ $0 == UInt8(ascii: " ") || $0.isASCIIDigit }) else { continue }
            // …and the number must END where the needle does: block 10's
            // needle "%end <cid> 10" also matches the front of
            // "%end <cid> 105 1", a content line (a log quoting another
            // reply) that closed block 10 early and spilled the rest of
            // the reply out as stray lines.
            guard tail.isEmpty || tail.first == UInt8(ascii: " ") else { continue }
            // The prefix checks in dispatchLine already handled range at
            // line start; here we only accept a strictly embedded suffix.
            guard range.lowerBound != line.startIndex else { return nil }
            return (line[..<range.lowerBound], line[range.lowerBound...])
        }
        return nil
    }

    /// Bytes an embedded terminator may carry after its ids: a space and
    /// the flags digit(s).
    private static let maxTerminatorTail = 8

    /// The last occurrence of `needle` in `line` that leaves at most `tail`
    /// bytes after it — the only kind an embedded terminator can be. Only
    /// the line's last `needle + tail` bytes are searched, so the check
    /// costs the same on a 10 000-byte line as on a short one; an earlier
    /// occurrence could not count, and a later one would be in the window
    /// too (B102: searching the whole line with a needle formatted for every
    /// line made a backfill reply parse at 3–6 MB/s).
    static func lastRange(
        of needle: [UInt8], nearEndOf line: ArraySlice<UInt8>, tail: Int
    ) -> Range<Int>? {
        guard !needle.isEmpty, line.count >= needle.count else { return nil }
        let lowest = max(line.startIndex, line.endIndex - needle.count - tail)
        var start = line.endIndex - needle.count
        while start >= lowest {
            var matched = true
            for offset in 0..<needle.count where line[start + offset] != needle[offset] {
                matched = false
                break
            }
            if matched { return start..<(start + needle.count) }
            start -= 1
        }
        return nil
    }

    /// `"…%begin <cid> <num> <flags>"` suffix whose num is a PLAUSIBLE next
    /// command number (strictly after the last real block, within a small
    /// window) → the byte offset where the `%begin` starts. Content quoting
    /// old dumps carries stale numbers and fails the window.
    private func embeddedBeginOffset(in raw: Data) -> Data.Index? {
        let marker = Data("%begin ".utf8)
        guard let range = raw.lastRange(of: marker), range.lowerBound != raw.startIndex else { return nil }
        // Valid UTF-8 or not a protocol line; then its fields, as bytes. The
        // flag is ONE ASCII digit — `Character.isNumber` took "五" for one
        // and opened a phantom block that swallowed the next real reply.
        guard String(data: raw[range.lowerBound...], encoding: .utf8) != nil else { return nil }
        let parts = ArraySlice(raw[range.lowerBound...])
            .split(separator: UInt8(ascii: " "), omittingEmptySubsequences: false)
        guard parts.count == 4,
              let cid = parts[1].int, let num = parts[2].int,
              parts[3].count == 1, parts[3].allSatisfy(\.isASCIIDigit) else { return nil }
        // Both distances without trapping: the ids are pane TEXT — anything
        // that parses as an Int, `-9223372036854775808` included — and
        // `num - lastBlockNum`, `cid - lastBlockCid` and `abs` all trap on
        // overflow. One line of pane output crashed the app.
        let (numGap, numOverflow) = num.subtractingReportingOverflow(lastBlockNum)
        let (cidGap, cidOverflow) = cid.subtractingReportingOverflow(lastBlockCid)
        guard num > lastBlockNum, !numOverflow, numGap <= 200,
              !cidOverflow, cidGap.magnitude <= 86_400 else { return nil }
        return range.lowerBound
    }

    /// True when `line` is the OPEN block's own terminator: `%end`/`%error`
    /// echoing the ids from its `%begin` (or the bare keyword, kept for
    /// defensive symmetry — tmux always sends the ids). Anything else that
    /// merely starts with the keyword is content. See dispatchLine.
    private func terminatesOpenBlock(_ line: ArraySlice<UInt8>, prefix: String) -> Bool {
        guard let block = openBlock else { return false }
        if line.equals(prefix) { return true }
        guard line.hasPrefix(prefix + " ") else { return false }
        let parts = line.split(separator: UInt8(ascii: " "), maxSplits: 4,
                               omittingEmptySubsequences: false)
        guard parts.count >= 3, let cid = parts[1].int, let num = parts[2].int else {
            return false
        }
        return cid == block.commandId && num == block.commandNum
    }

    /// `%begin <commandId> <commandNum> [flags]`
    private func handleBegin(_ line: ArraySlice<UInt8>) {
        let parts = line.split(separator: UInt8(ascii: " "), maxSplits: 4,
                               omittingEmptySubsequences: false)
        guard parts.count >= 3,
              let cid = parts[1].int,
              let num = parts[2].int else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        // `[flags]`: a block for a command that came from this client's own
        // input has bit 1 set. Missing or unreadable reads as ours, the old
        // way of reading every block (crates/tmux reads it the same way).
        let fromClient = parts.count >= 4 ? (parts[3].int.map { $0 & 1 == 1 } ?? true) : true
        // A previous block still open means its terminator never came (or
        // was mangled in transit). Surface the desync, but ALSO deliver the
        // stale block as an error response: its sender's callback occupies a
        // FIFO slot, and silently discarding the block would leave that slot
        // forever unpopped — every later response then lands one command
        // back (the same pairing shift expectBootBlock's doc describes).
        if let stale = openBlock {
            openBlock = nil
            onProtocolError?(.malformedLine("orphan %begin while block #\(stale.commandId) open"))
            deliverBlock(commandId: stale.commandId, commandNum: stale.commandNum,
                         isError: true, fromClient: stale.fromClient, lines: stale.lines)
        }
        lastBlockCid = cid
        lastBlockNum = num
        openBlock = (commandId: cid, commandNum: num, fromClient: fromClient, lines: [])
        openBlockNeedles = [Array("%end \(cid) \(num)".utf8), Array("%error \(cid) \(num)".utf8)]
    }

    /// Handle `%end <id> <num>` or `%error <id> <num>`.
    private func handleBlockEnd(line: ArraySlice<UInt8>, isError: Bool) {
        guard let block = openBlock else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        openBlock = nil
        // We don't strictly need to re-parse the ids on the terminator (tmux
        // matches them to the open block by ordering), but log a mismatch.
        let parts = line.split(separator: UInt8(ascii: " "), maxSplits: 4,
                               omittingEmptySubsequences: false)
        if parts.count >= 3,
           let cid = parts[1].int,
           cid != block.commandId {
            onProtocolError?(.malformedLine("mismatched terminator id \(cid) for block #\(block.commandId)"))
        }
        deliverBlock(commandId: block.commandId, commandNum: block.commandNum,
                     isError: isError, fromClient: block.fromClient, lines: block.lines)
    }

    /// Single exit for EVERY completed block — normal, orphaned, or
    /// force-closed. The boot line's own reply never had a queued callback:
    /// delivering it upstream pops someone else's slot, so whichever path a
    /// boot block leaves through, it must be swallowed here exactly once.
    /// See `awaitingBootBlock`.
    private func deliverBlock(commandId: Int, commandNum: Int,
                              isError: Bool, fromClient: Bool, lines: [String]) {
        if awaitingBootBlock {
            awaitingBootBlock = false
            return
        }
        onCommandResponse?(TmuxCommandResponse(
            commandId: commandId,
            commandNum: commandNum,
            isError: isError,
            lines: lines,
            fromClient: fromClient))
    }

    // MARK: - Notification handlers

    /// ASCII bytes of `"%output "` — the prefix check runs on raw Data.
    private static let outputPrefix = Data("%output ".utf8)

    /// `%output %paneId <data>` — data uses `\NNN` / `\\` escapes, and may
    /// contain raw bytes ≥ 0x80 that do NOT form complete UTF-8 sequences
    /// (tmux flushes mid-character). The whole path stays on raw bytes; see
    /// the dispatch comment. The pane id itself is plain ASCII (`%N`).
    private func handleOutputRaw(_ raw: Data) {
        let space = UInt8(ascii: " ")
        let body = raw.dropFirst(Self.outputPrefix.count)
        guard let sep = body.firstIndex(of: space), sep > body.startIndex,
              let paneId = String(data: body[body.startIndex..<sep], encoding: .utf8) else {
            onProtocolError?(.malformedLine(String(decoding: raw, as: UTF8.self)))
            return
        }
        // One fresh, zero-based copy used for BOTH the offset search and the
        // slicing below — Data slice indices are parent-relative, and mixing
        // instances here once produced an empty prefix.
        let payload = Data(body[body.index(after: sep)...])
        // Truncated-line recovery, output half: a lossy hop cut this %output
        // line's tail and the newline, gluing the NEXT protocol line onto it
        // ("%output %0 …<cut>%begin 1787125682 81830 1" as one line — 真机取证
        // 2026-08-19). Without the split, the %begin paints as literal pane
        // text and its reply block is swallowed — pairing shifts by one for
        // good. The number-window check keeps pane content that merely QUOTES
        // an old dump intact.
        if let beginAt = embeddedBeginOffset(in: payload) {
            let beginLine = ArraySlice(payload.suffix(from: beginAt))
            onProtocolError?(.malformedLine("recovered %begin glued to truncated %output"))
            let decoded = Self.decodeOctalEscapes(Data(payload.prefix(upTo: beginAt)))
            onPaneOutput?(paneId, decoded)
            handleBegin(beginLine)
            return
        }
        let decoded = Self.decodeOctalEscapes(payload)
        onPaneOutput?(paneId, decoded)
    }

    /// `%layout-change @windowId <layout> [visible_layout] [flags]`
    private func handleLayoutChange(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%layout-change ".utf8.count)
        let parts = body.split(separator: UInt8(ascii: " "), maxSplits: 3,
                               omittingEmptySubsequences: true)
        guard parts.count >= 2 else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        let windowId = parts[0].string
        let layout = parts[1].string
        // Zoomed when a window-flags field carries `Z`. tmux 3.6a sends
        // `#{window_raw_flags}` there, and `*Z` only for the session's
        // current window with no other flag set: a zoomed window that is
        // not current reads `-Z` or `Z`, a marked pane makes it `*MZ`,
        // activity or a bell `#*Z` / `!*Z`. Matching `*Z` exactly reported
        // all of those as unzoomed.
        let zoomed = parts.dropFirst(2).contains(where: Self.isZoomedFlags)
        onLayoutChange?(windowId, layout, zoomed)
    }

    /// A tmux window-flags field — only the characters `#!~*-MZ` — with
    /// the zoomed flag `Z` in it. A layout never qualifies: it always
    /// contains digits.
    private static func isZoomedFlags(_ field: ArraySlice<UInt8>) -> Bool {
        field.contains(UInt8(ascii: "Z")) && field.allSatisfy { "#!~*-MZ".utf8.contains($0) }
    }

    private func handleWindowAdd(_ line: ArraySlice<UInt8>) {
        let id = line.dropFirst("%window-add ".utf8.count).string
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        onWindowAdd?(id)
    }

    private func handleWindowClose(_ line: ArraySlice<UInt8>) {
        let id = line.dropFirst("%window-close ".utf8.count).string
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        onWindowClose?(id)
    }

    private func handleWindowRenamed(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%window-renamed ".utf8.count)
        guard let space = body.firstIndex(of: UInt8(ascii: " ")) else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        let id = body[body.startIndex..<space].string
        let name = body[body.index(after: space)...].string
        onWindowRenamed?(id, name)
    }

    private func handleSessionChanged(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%session-changed ".utf8.count)
        guard let space = body.firstIndex(of: UInt8(ascii: " ")) else {
            // Some tmux builds emit just the id with no name — tolerate it.
            let id = body.string
            guard !id.isEmpty else {
                onProtocolError?(.malformedLine(line.string))
                return
            }
            onSessionChanged?(id, "")
            return
        }
        let id = body[body.startIndex..<space].string
        let name = body[body.index(after: space)...].string
        onSessionChanged?(id, name)
    }

    /// `%session-renamed $sessionId <name>` (tmux 3.6a, verified on a
    /// private server). Cut at the FIRST space: the id never holds one, and
    /// the name is the rest of the line — spaces, `|` and all.
    private func handleSessionRenamed(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%session-renamed ".utf8.count)
        guard let space = body.firstIndex(of: UInt8(ascii: " ")), space > body.startIndex else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        let id = body[body.startIndex..<space].string
        let name = body[body.index(after: space)...].string
        onSessionRenamed?(id, name)
    }

    private func handleSessionWindowChanged(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%session-window-changed ".utf8.count)
        let parts = body.split(separator: UInt8(ascii: " "), maxSplits: 1)
        guard parts.count == 2 else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        onSessionWindowChanged?(parts[0].string, parts[1].string)
    }

    private func handleWindowPaneChanged(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%window-pane-changed ".utf8.count)
        let parts = body.split(separator: UInt8(ascii: " "), maxSplits: 1)
        guard parts.count == 2 else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        onActivePaneChanged?(parts[0].string, parts[1].string)
    }

    /// `<prefix>@windowId` — the id is the rest of the line, as for
    /// `%window-add`.
    private func handleWindowId(_ line: ArraySlice<UInt8>, prefix: String,
                                _ callback: (@Sendable (String) -> Void)?) {
        let id = line.dropFirst(prefix.utf8.count).string
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        callback?(id)
    }

    /// `%unlinked-window-renamed @windowId <name>`, cut like
    /// `%window-renamed`: at the first space, the name the rest.
    private func handleUnlinkedWindowRenamed(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%unlinked-window-renamed ".utf8.count)
        guard let space = body.firstIndex(of: UInt8(ascii: " ")) else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        let id = body[body.startIndex..<space].string
        let name = body[body.index(after: space)...].string
        onUnlinkedWindowRenamed?(id, name)
    }

    /// `%client-session-changed <client> $sessionId <name>` (tmux 3.6a,
    /// captures/session-switch.bin: `%client-session-changed /dev/ttys024 $1
    /// renamed`). The client name and the id hold no space, so both cuts
    /// are at the first space; the name is the rest of the line.
    private func handleClientSessionChanged(_ line: ArraySlice<UInt8>) {
        let body = line.dropFirst("%client-session-changed ".utf8.count)
        guard let first = body.firstIndex(of: UInt8(ascii: " ")), first > body.startIndex else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        let rest = body[body.index(after: first)...]
        guard let second = rest.firstIndex(of: UInt8(ascii: " ")), second > rest.startIndex else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        let client = body[body.startIndex..<first].string
        let id = rest[rest.startIndex..<second].string
        let name = rest[rest.index(after: second)...].string
        onClientSessionChanged?(client, id, name)
    }

    private func handlePause(_ line: ArraySlice<UInt8>) {
        let id = line.dropFirst("%pause ".utf8.count).string
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        onPause?(id)
    }

    private func handleContinue(_ line: ArraySlice<UInt8>) {
        let id = line.dropFirst("%continue ".utf8.count).string
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line.string))
            return
        }
        onContinue?(id)
    }

    private func handleClientDetached(_ line: ArraySlice<UInt8>) {
        // Accept both `%client-detached <client>` and the bare directive.
        let payload: String
        if line.hasPrefix("%client-detached ") {
            payload = line.dropFirst("%client-detached ".utf8.count).string
        } else {
            payload = ""
        }
        onClientDetached?(payload)
    }

    private func handleExit(_ line: ArraySlice<UInt8>) {
        // A control session just ended on this stream. If another one boots
        // behind it (a failed preferred attach falling through its `||`
        // chain, the `-CC new` fallback), its first block is that boot
        // line's own reply — swallow it too. See `awaitingBootBlock`.
        awaitingBootBlock = true
        if line.equals("%exit") {
            onExit?(nil)
            return
        }
        let reason = line.dropFirst("%exit ".utf8.count).string
        onExit?(reason.isEmpty ? nil : reason)
    }

    // MARK: - Octal-escape decoder

    /// tmux's `%output` payload encodes any byte outside the printable ASCII
    /// range as `\NNN` (three octal digits) and escapes a literal backslash as
    /// `\\`. Everything else passes through untouched.
    ///
    /// Made `static` so the parser tests can hit it directly without spinning
    /// up the actor.
    static func decodeOctalEscapes(_ raw: Data) -> Data {
        var result = Data()
        result.reserveCapacity(raw.count)

        let backslash = UInt8(ascii: "\\")
        let zero = UInt8(ascii: "0")
        let seven = UInt8(ascii: "7")

        var i = raw.startIndex
        while i < raw.endIndex {
            let byte = raw[i]
            if byte == backslash {
                let remaining = raw.endIndex - i - 1
                if remaining >= 3 {
                    let d0 = raw[i + 1]
                    let d1 = raw[i + 2]
                    let d2 = raw[i + 3]
                    if d0 >= zero && d0 <= seven &&
                        d1 >= zero && d1 <= seven &&
                        d2 >= zero && d2 <= seven {
                        let value = (d0 - zero) &* 64 &+ (d1 - zero) &* 8 &+ (d2 - zero)
                        result.append(value)
                        i += 4
                        continue
                    }
                }
                if remaining >= 1, raw[i + 1] == backslash {
                    result.append(backslash)
                    i += 2
                    continue
                }
                // Lone backslash — pass through as-is.
                result.append(byte)
                i += 1
            } else {
                result.append(byte)
                i += 1
            }
        }

        return result
    }
}

// MARK: - Protocol bytes

/// tmux writes its control protocol in ASCII, so keywords, delimiters, ids
/// and flags are matched on a line's UTF-8 BYTES — never through String's
/// Character view, which lets the remote text around them change what
/// they mean: a space or `%` followed by a combining mark is ONE Character
/// (a window named "\u{301}x" made its `%window-renamed` unparseable, a
/// session id swallow its name, the junk strip skip a keyword), and
/// `Character.isNumber` accepts "五" or "٣" as a flag digit. A field cut at
/// ASCII bytes of a valid UTF-8 line is valid UTF-8 itself, so ``string``
/// returns it exactly.
private extension ArraySlice where Element == UInt8 {
    /// Whether these bytes begin with `prefix`'s UTF-8.
    func hasPrefix(_ prefix: String) -> Bool { starts(with: prefix.utf8) }

    /// Whether these bytes are exactly `text`'s UTF-8.
    func equals(_ text: String) -> Bool { elementsEqual(text.utf8) }

    /// The bytes as a String (lossy only for invalid UTF-8).
    var string: String { String(decoding: self, as: UTF8.self) }

    /// `Int(_:)` of the field: an optional sign and ASCII digits, nil on
    /// overflow.
    var int: Int? { Int(string) }
}

private extension UInt8 {
    var isASCIIDigit: Bool { self >= UInt8(ascii: "0") && self <= UInt8(ascii: "9") }
}
