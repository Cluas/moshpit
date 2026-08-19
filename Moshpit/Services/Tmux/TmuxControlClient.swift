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
/// affinity must hop themselves.
actor TmuxControlClient {

    // MARK: - Callbacks

    /// Invoked with raw, octal-decoded bytes whenever tmux emits
    /// `%output %paneId <data>`. `paneId` is the literal `"%0"` etc.
    var onPaneOutput: (@Sendable (_ paneId: String, _ data: Data) -> Void)?

    /// Invoked for every `%layout-change @windowId <layout> [...]` line.
    /// `zoomed` is true when the line contains a `*Z` marker.
    var onLayoutChange: (@Sendable (_ windowId: String, _ layout: String, _ zoomed: Bool) -> Void)?

    /// Invoked for `%window-add @windowId`.
    var onWindowAdd: (@Sendable (_ windowId: String) -> Void)?

    /// Invoked for `%window-close @windowId`.
    var onWindowClose: (@Sendable (_ windowId: String) -> Void)?

    /// Invoked for `%window-renamed @windowId <name>`.
    var onWindowRenamed: (@Sendable (_ windowId: String, _ name: String) -> Void)?

    /// Invoked for `%session-changed $sessionId <name>`.
    var onSessionChanged: (@Sendable (_ sessionId: String, _ name: String) -> Void)?

    /// Invoked for `%session-window-changed $sessionId @windowId`.
    var onSessionWindowChanged: (@Sendable (_ sessionId: String, _ windowId: String) -> Void)?

    /// Invoked for `%window-pane-changed @windowId %paneId`.
    var onActivePaneChanged: (@Sendable (_ windowId: String, _ paneId: String) -> Void)?

    /// Invoked for `%pause %paneId` — caller should respond with
    /// `refresh-client -A %paneId:+` to resume the stream.
    var onPause: (@Sendable (_ paneId: String) -> Void)?

    /// Invoked for `%continue %paneId`.
    var onContinue: (@Sendable (_ paneId: String) -> Void)?

    /// Invoked for `%client-detached <client>` / `%client-session-changed …`.
    /// The single string is whatever tmux placed after the directive.
    var onClientDetached: (@Sendable (_ payload: String) -> Void)?

    /// Invoked when tmux closes the control mode session (`%exit [reason]`).
    var onExit: (@Sendable (_ reason: String?) -> Void)?

    /// Invoked at the end of each `%begin … %end` (or `%error`) block.
    var onCommandResponse: (@Sendable (_ response: TmuxCommandResponse) -> Void)?

    /// Invoked when a recognised line cannot be parsed. Unknown `%foo` lines
    /// are *not* errors — they pass through silently for forward-compat.
    var onProtocolError: (@Sendable (_ error: TmuxControlError) -> Void)?

    // MARK: - Internal state

    /// Raw transport bytes that have arrived but do not yet form a full line.
    /// Lines are separated by `\n`; a dangling final `\r` is stripped during
    /// line extraction so CRLF and LF transports look identical to the rest
    /// of the parser.
    private var buffer = Data()

    /// `%begin` data captured at block open; cleared at `%end` / `%error`.
    private var openBlock: (commandId: Int, commandNum: Int, lines: [String])?

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

    // MARK: - Init

    init() {}

    // MARK: - Public API

    /// Feed transport bytes from the SSH session. Safe to call with empty
    /// data; partial lines are buffered until the next call.
    func feed(_ data: Data) {
        guard !data.isEmpty else { return }
        buffer.append(data)
        drainLines()
    }

    /// Discard any partial line in the buffer and any open response block.
    /// Use when reconnecting or after a protocol error to resync.
    func reset() {
        buffer.removeAll(keepingCapacity: false)
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
        onSessionWindowChanged: (@Sendable (String, String) -> Void)? = nil,
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
        if let onSessionWindowChanged { self.onSessionWindowChanged = onSessionWindowChanged }
        if let onActivePaneChanged { self.onActivePaneChanged = onActivePaneChanged }
        if let onPause { self.onPause = onPause }
        if let onContinue { self.onContinue = onContinue }
        if let onClientDetached { self.onClientDetached = onClientDetached }
        if let onExit { self.onExit = onExit }
        if let onCommandResponse { self.onCommandResponse = onCommandResponse }
        if let onProtocolError { self.onProtocolError = onProtocolError }
    }

    // MARK: - Line extraction

    /// Pull every complete line out of ``buffer`` and dispatch it. A trailing
    /// partial line (no `\n`) is preserved for the next ``feed(_:)``.
    private func drainLines() {
        let newline = UInt8(ascii: "\n")
        let carriageReturn = UInt8(ascii: "\r")

        while let nlIndex = buffer.firstIndex(of: newline) {
            let lineSlice = buffer[buffer.startIndex..<nlIndex]
            // Strip trailing CR if present (CRLF line endings).
            let stripped: Data
            if let last = lineSlice.last, last == carriageReturn {
                stripped = lineSlice.dropLast()
            } else {
                stripped = Data(lineSlice)
            }
            // Advance past the newline byte.
            buffer = Data(buffer[buffer.index(after: nlIndex)...])

            // Defer UTF-8 decoding to the dispatcher so we can capture the
            // raw payload of malformed lines.
            dispatchLine(stripped)
        }
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
            // Truncated-line recovery, in-block half: a lossy hop can drop a
            // content line's tail, gluing the block's OWN terminator onto it
            // ("<content>%end <cid> <num> 1" as one line). The prefix checks
            // below then miss it and the block never closes. The suffix must
            // echo the open block's EXACT ids to count — content quoting some
            // other dump can't match a live command's unique pair.
            if let (content, terminator) = splitEmbeddedTerminator(blockLine) {
                if !content.isEmpty { openBlock?.lines.append(content) }
                handleBlockEnd(line: terminator, isError: terminator.hasPrefix("%error"))
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
                             isError: true, lines: block.lines)
                dispatchLine(raw)
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
            if terminatesOpenBlock(blockLine, prefix: "%end") {
                handleBlockEnd(line: blockLine, isError: false)
            } else if terminatesOpenBlock(blockLine, prefix: "%error") {
                handleBlockEnd(line: blockLine, isError: true)
            } else {
                openBlock?.lines.append(blockLine)
            }
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
        } else if line.hasPrefix("%session-window-changed ") {
            handleSessionWindowChanged(line)
        } else if line.hasPrefix("%window-pane-changed ") {
            handleWindowPaneChanged(line)
        } else if line.hasPrefix("%pause ") {
            handlePause(line)
        } else if line.hasPrefix("%continue ") {
            handleContinue(line)
        } else if line.hasPrefix("%client-detached") {
            handleClientDetached(line)
        } else if line == "%exit" || line.hasPrefix("%exit ") {
            handleExit(line)
        } else if line.hasPrefix("%end ") || line == "%end" ||
                  line.hasPrefix("%error ") || line == "%error" {
            // Block terminator without a matching `%begin`: ignore but report
            // so logs can surface the desync.
            onProtocolError?(.malformedLine(line))
        } else {
            // Unknown directive — forward-compat: silent skip.
        }
    }

    /// UTF-8 decode the line and strip any leading bytes before the first
    /// `%`-starting tmux keyword. This handles transports that prepend DCS
    /// (`\eP1000p…`) or shell banners on the very first line.
    private func decodeLine(_ raw: Data) -> String {
        let decoded = String(data: raw, encoding: .utf8) ?? String(
            decoding: raw, as: UTF8.self
        )
        guard !decoded.hasPrefix("%") else { return decoded }
        guard let pct = decoded.firstIndex(of: "%") else { return decoded }
        let candidate = decoded[pct...]
        if candidate.hasPrefix("%begin") || candidate.hasPrefix("%end") ||
            candidate.hasPrefix("%error") || candidate.hasPrefix("%output") ||
            candidate.hasPrefix("%layout") || candidate.hasPrefix("%window") ||
            candidate.hasPrefix("%session") || candidate.hasPrefix("%pause") ||
            candidate.hasPrefix("%continue") || candidate.hasPrefix("%client") ||
            candidate.hasPrefix("%exit") {
            return String(candidate)
        }
        return decoded
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
    private func splitEmbeddedTerminator(_ line: String) -> (String, String)? {
        guard let block = openBlock else { return nil }
        for keyword in ["%end ", "%error "] {
            guard let range = line.range(of: "\(keyword)\(block.commandId) \(block.commandNum)",
                                         options: .backwards) else { continue }
            // Must run to end-of-line (allowing trailing flags) — a mention
            // mid-line is content.
            let tail = line[range.upperBound...]
            guard tail.count <= 8, !tail.contains(where: { !$0.isNumber && $0 != " " }) else { continue }
            // The prefix checks in dispatchLine already handled range at
            // line start; here we only accept a strictly embedded suffix.
            guard range.lowerBound != line.startIndex else { return nil }
            return (String(line[..<range.lowerBound]), String(line[range.lowerBound...]))
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
        guard let suffix = String(data: raw[range.lowerBound...], encoding: .utf8) else { return nil }
        let parts = suffix.split(separator: " ", omittingEmptySubsequences: false)
        guard parts.count == 4,
              let cid = Int(parts[1]), let num = Int(parts[2]),
              parts[3].count == 1, parts[3].allSatisfy(\.isNumber) else { return nil }
        guard num > lastBlockNum, num - lastBlockNum <= 200,
              abs(cid - lastBlockCid) <= 86_400 else { return nil }
        return range.lowerBound
    }

    /// True when `line` is the OPEN block's own terminator: `%end`/`%error`
    /// echoing the ids from its `%begin` (or the bare keyword, kept for
    /// defensive symmetry — tmux always sends the ids). Anything else that
    /// merely starts with the keyword is content. See dispatchLine.
    private func terminatesOpenBlock(_ line: String, prefix: String) -> Bool {
        guard let block = openBlock else { return false }
        if line == prefix { return true }
        guard line.hasPrefix(prefix + " ") else { return false }
        let parts = line.split(separator: " ", maxSplits: 4, omittingEmptySubsequences: false)
        guard parts.count >= 3, let cid = Int(parts[1]), let num = Int(parts[2]) else {
            return false
        }
        return cid == block.commandId && num == block.commandNum
    }

    /// `%begin <commandId> <commandNum> [flags]`
    private func handleBegin(_ line: String) {
        let parts = line.split(separator: " ", maxSplits: 4, omittingEmptySubsequences: false)
        guard parts.count >= 3,
              let cid = Int(parts[1]),
              let num = Int(parts[2]) else {
            onProtocolError?(.malformedLine(line))
            return
        }
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
                         isError: true, lines: stale.lines)
        }
        lastBlockCid = cid
        lastBlockNum = num
        openBlock = (commandId: cid, commandNum: num, lines: [])
    }

    /// Handle `%end <id> <num>` or `%error <id> <num>`.
    private func handleBlockEnd(line: String, isError: Bool) {
        guard let block = openBlock else {
            onProtocolError?(.malformedLine(line))
            return
        }
        openBlock = nil
        // We don't strictly need to re-parse the ids on the terminator (tmux
        // matches them to the open block by ordering), but log a mismatch.
        let parts = line.split(separator: " ", maxSplits: 4, omittingEmptySubsequences: false)
        if parts.count >= 3,
           let cid = Int(parts[1]),
           cid != block.commandId {
            onProtocolError?(.malformedLine("mismatched terminator id \(cid) for block #\(block.commandId)"))
        }
        deliverBlock(commandId: block.commandId, commandNum: block.commandNum,
                     isError: isError, lines: block.lines)
    }

    /// Single exit for EVERY completed block — normal, orphaned, or
    /// force-closed. The boot line's own reply never had a queued callback:
    /// delivering it upstream pops someone else's slot, so whichever path a
    /// boot block leaves through, it must be swallowed here exactly once.
    /// See `awaitingBootBlock`.
    private func deliverBlock(commandId: Int, commandNum: Int,
                              isError: Bool, lines: [String]) {
        if awaitingBootBlock {
            awaitingBootBlock = false
            return
        }
        onCommandResponse?(TmuxCommandResponse(
            commandId: commandId,
            commandNum: commandNum,
            isError: isError,
            lines: lines))
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
            let beginLine = String(decoding: payload.suffix(from: beginAt), as: UTF8.self)
            onProtocolError?(.malformedLine("recovered %begin glued to truncated %output"))
            let decoded = Self.decodeOctalEscapes(Data(payload.prefix(upTo: beginAt)))
            onPaneOutput?(paneId, decoded)
            handleBegin(beginLine)
            return
        }
        let decoded = Self.decodeOctalEscapes(payload)
        onPaneOutput?(paneId, decoded)
    }

    /// `%layout-change @windowId <layout> [visible_layout] [*Z]`
    private func handleLayoutChange(_ line: String) {
        let body = line.dropFirst("%layout-change ".count)
        let parts = body.split(separator: " ", maxSplits: 3, omittingEmptySubsequences: true)
        guard parts.count >= 2 else {
            onProtocolError?(.malformedLine(line))
            return
        }
        let windowId = String(parts[0])
        let layout = String(parts[1])
        let zoomed: Bool
        if parts.count > 2 {
            zoomed = parts[2...].contains(where: { $0 == "*Z" })
        } else {
            zoomed = false
        }
        onLayoutChange?(windowId, layout, zoomed)
    }

    private func handleWindowAdd(_ line: String) {
        let id = String(line.dropFirst("%window-add ".count))
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line))
            return
        }
        onWindowAdd?(id)
    }

    private func handleWindowClose(_ line: String) {
        let id = String(line.dropFirst("%window-close ".count))
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line))
            return
        }
        onWindowClose?(id)
    }

    private func handleWindowRenamed(_ line: String) {
        let body = line.dropFirst("%window-renamed ".count)
        guard let space = body.firstIndex(of: " ") else {
            onProtocolError?(.malformedLine(line))
            return
        }
        let id = String(body[body.startIndex..<space])
        let name = String(body[body.index(after: space)...])
        onWindowRenamed?(id, name)
    }

    private func handleSessionChanged(_ line: String) {
        let body = line.dropFirst("%session-changed ".count)
        guard let space = body.firstIndex(of: " ") else {
            // Some tmux builds emit just the id with no name — tolerate it.
            let id = String(body)
            guard !id.isEmpty else {
                onProtocolError?(.malformedLine(line))
                return
            }
            onSessionChanged?(id, "")
            return
        }
        let id = String(body[body.startIndex..<space])
        let name = String(body[body.index(after: space)...])
        onSessionChanged?(id, name)
    }

    private func handleSessionWindowChanged(_ line: String) {
        let body = line.dropFirst("%session-window-changed ".count)
        let parts = body.split(separator: " ", maxSplits: 1)
        guard parts.count == 2 else {
            onProtocolError?(.malformedLine(line))
            return
        }
        onSessionWindowChanged?(String(parts[0]), String(parts[1]))
    }

    private func handleWindowPaneChanged(_ line: String) {
        let body = line.dropFirst("%window-pane-changed ".count)
        let parts = body.split(separator: " ", maxSplits: 1)
        guard parts.count == 2 else {
            onProtocolError?(.malformedLine(line))
            return
        }
        onActivePaneChanged?(String(parts[0]), String(parts[1]))
    }

    private func handlePause(_ line: String) {
        let id = String(line.dropFirst("%pause ".count))
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line))
            return
        }
        onPause?(id)
    }

    private func handleContinue(_ line: String) {
        let id = String(line.dropFirst("%continue ".count))
        guard !id.isEmpty else {
            onProtocolError?(.malformedLine(line))
            return
        }
        onContinue?(id)
    }

    private func handleClientDetached(_ line: String) {
        // Accept both `%client-detached <client>` and the bare directive.
        let payload: String
        if line.hasPrefix("%client-detached ") {
            payload = String(line.dropFirst("%client-detached ".count))
        } else {
            payload = ""
        }
        onClientDetached?(payload)
    }

    private func handleExit(_ line: String) {
        // A control session just ended on this stream. If another one boots
        // behind it (a failed preferred attach falling through its `||`
        // chain, the `-CC new` fallback), its first block is that boot
        // line's own reply — swallow it too. See `awaitingBootBlock`.
        awaitingBootBlock = true
        if line == "%exit" {
            onExit?(nil)
            return
        }
        let reason = String(line.dropFirst("%exit ".count))
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
