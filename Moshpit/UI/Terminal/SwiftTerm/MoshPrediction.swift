import Foundation
import SwiftTerm

/// mosh's predictive local echo: show what a keystroke will most likely do to
/// the screen now, instead of one round trip later. A port of
/// `Overlay::PredictionEngine` (mosh `src/frontend/terminaloverlay.cc`) onto
/// SwiftTerm's buffer, kept close to the original so the two can be read side
/// by side.
///
/// **Epochs.** Every guess belongs to an epoch, and an epoch is shown only
/// once the server has confirmed one of its guesses. Anything the engine
/// cannot model — a control key, an escape sequence, a wide character — opens
/// a new, unconfirmed epoch. So a password prompt or a full-screen app that
/// doesn't echo never shows a single predicted character: its first guess is
/// judged wrong before it was ever drawn.
///
/// **Judging.** A guess is judged only when the server's echo ack
/// (``MoshHostFrame/echoAck``) covers the user state that carried the
/// keystroke — the server held it for 50ms, so its echo is on screen if it is
/// coming at all. Wrong in a shown epoch → every guess goes; wrong in a hidden
/// epoch → that epoch goes, unseen.
///
/// **Adaptive.** On a fast link the guesses are not drawn at all (mosh's SRTT
/// trigger: shown above ~60ms round trip, hidden again below ~40ms). On a slow
/// one, unconfirmed characters are drawn dim (mosh underlines them) when
/// `trail` is on — Settings → "Trail on predict".
///
/// **The overlay.** Guesses are written straight into SwiftTerm's cells, and
/// every write is recorded so ``withdraw(from:)`` puts back exactly what was
/// there. The owner MUST withdraw before feeding any host bytes: mosh-server
/// diffs against what it believes this screen shows and never resends a cell
/// it thinks is unchanged, so a predicted character fed over would stay
/// forever (the diff-peer contract). The cycle is withdraw → feed →
/// ``settle(on:)`` → ``apply(to:)``.
///
/// Scope (deliberately smaller than mosh): no newline or wrap prediction — a
/// carriage return, or typing into the last column, waits for the server to
/// say where the cursor went. Overwrite mode is not ported.
///
/// Main thread only.
final class MoshPrediction {

    let mode: PredictMode
    /// Dim unconfirmed guesses on a slow link (mosh's "flagging").
    let trail: Bool
    /// The transport's smoothed round trip, ms. 0 = not measured yet, which
    /// counts as slow (mosh starts its SRTT at one second).
    var srttMs: Double = 0

    /// Monotonic milliseconds; injectable for tests.
    private let clock: () -> UInt64

    init(mode: PredictMode, trail: Bool, clock: @escaping () -> UInt64 = MoshPrediction.monotonicMs) {
        self.mode = mode
        self.trail = trail
        self.clock = clock
    }

    static func monotonicMs() -> UInt64 { DispatchTime.now().uptimeNanoseconds / 1_000_000 }

    // MARK: - Tuning (mosh's constants, in send-interval milliseconds)

    private static let srttTriggerLow: UInt64 = 20
    private static let srttTriggerHigh: UInt64 = 30
    private static let flagTriggerLow: UInt64 = 50
    private static let flagTriggerHigh: UInt64 = 80
    private static let glitchThreshold: UInt64 = 250
    private static let glitchRepairCount: UInt32 = 10
    private static let glitchRepairMinInterval: UInt64 = 150
    private static let glitchFlagThreshold: UInt64 = 5000

    // MARK: - Prediction state

    private struct Cell {
        let col: Int
        var active = false
        var tentativeUntilEpoch: UInt64
        /// The keystroke batch whose user state judges this guess (0 = none).
        var batch = 0
        var predictionTime = UInt64.max
        var replacement = CharData.Null
        /// The column's content can't be known (shifted in from the edge).
        var unknown = false
        /// What was there before; a guess that merely matches one of these
        /// gets no credit for confirming its epoch.
        var originalContents: [CharData] = []

        init(col: Int, epoch: UInt64) {
            self.col = col
            self.tentativeUntilEpoch = epoch
        }

        func tentative(_ confirmed: UInt64) -> Bool { tentativeUntilEpoch > confirmed }

        mutating func reset() {
            batch = 0
            tentativeUntilEpoch = .max
            active = false
            unknown = false
            originalContents.removeAll()
        }

        mutating func resetWithOriginal() {
            guard active, !unknown else { reset(); return }
            originalContents.append(replacement)
            batch = 0
            tentativeUntilEpoch = .max
            active = false
        }

        mutating func expire(batch: Int, now: UInt64) {
            self.batch = batch
            predictionTime = now
        }
    }

    private struct Row {
        let row: Int
        var cells: [Cell]
    }

    private struct CursorMove {
        var row: Int
        var col: Int
        var active = true
        var tentativeUntilEpoch: UInt64
        var batch: Int
        var predictionTime = UInt64.max

        func tentative(_ confirmed: UInt64) -> Bool { tentativeUntilEpoch > confirmed }

        mutating func expire(batch: Int, now: UInt64) {
            self.batch = batch
            predictionTime = now
        }
    }

    private enum Validity { case pending, correct, correctNoCredit, incorrectOrExpired, inactive }

    private var overlay: [Row] = []
    private var cursors: [CursorMove] = []
    private var predictionEpoch: UInt64 = 1
    private var confirmedEpoch: UInt64 = 0
    private var flagging = false
    private var srttTrigger = false
    private var glitchTrigger: UInt32 = 0
    private var lastQuickConfirmation: UInt64 = 0
    private var lastSize = (cols: 0, rows: 0)

    /// Keystroke batches stand in for mosh's `local_frame_sent + 1`: a guess
    /// is stamped with its batch when made, and the batch learns its user
    /// state number once the transport has actually queued the bytes
    /// (``sent(batch:as:)``). Unresolved = still pending.
    private var nextBatch = 1
    private var batchStates: [Int: UInt64] = [:]
    /// The newest echo ack seen (mosh's `local_frame_late_acked`).
    private var echoAck: UInt64 = 0
    /// Set by a carriage return or a keystroke into the last column — where
    /// the cursor lands is the server's to say. Nothing is predicted until
    /// the echo ack covers this batch.
    private var cursorBarrier: Int?

    private var scanner = InputScanner()

    // MARK: - Inputs

    /// Keystrokes about to go to the server: guess their echo. Returns the
    /// batch to hand back to ``sent(batch:as:)``, or nil when predicting is
    /// off. Call with the overlay withdrawn.
    func userInput(_ data: Data, on terminal: Terminal) -> Int? {
        guard mode != .never, !data.isEmpty else { return nil }
        let batch = nextBatch
        nextBatch += 1
        let screen = Screen(terminal: terminal)
        cull(screen, batch: batch)
        let now = clock()
        for event in scanner.scan(data) {
            if mode == .experimental { predictionEpoch = confirmedEpoch }
            handle(event, screen: screen, batch: batch, now: now)
        }
        return batch
    }

    /// The transport queued `batch` as user state `num` (nil: it couldn't).
    func sent(batch: Int, as num: UInt64?) {
        guard let num else { reset(); return }
        batchStates[batch] = num
    }

    /// Record the echo ack of a frame that just landed. Monotonic.
    func noteEchoAck(_ ack: UInt64) {
        echoAck = max(echoAck, ack)
    }

    /// Judge every guess the echo ack now covers. Call with the overlay
    /// withdrawn, after the frame's bytes were fed.
    func settle(on terminal: Terminal) {
        cull(Screen(terminal: terminal), batch: nextBatch)
    }

    /// Drop every guess (mosh's `reset`): the screen is the server's again.
    func reset() {
        cursors.removeAll()
        overlay.removeAll()
        cursorBarrier = nil
        becomeTentative()
    }

    // MARK: - Timing

    /// Whether ``tick()`` should run every 50ms: a guessed cell is waiting on
    /// the server and a timing trigger (a guess stuck past 250ms shows
    /// predictions even on a fast link) could still fire. Idle = no ticks.
    var needsTick: Bool {
        guard mode != .never, !(glitchTrigger > 0 && flagging) else { return false }
        return overlay.contains { $0.cells.contains { $0.active && echoAck < state(of: $0.batch) } }
    }

    /// The clock-only part of judging — no cell reads, so it is safe with the
    /// overlay applied. Returns true when what should be drawn changed.
    func tick() -> Bool {
        guard mode != .never else { return false }
        let before = (showing, drawFlagged)
        updateTriggers()
        let now = clock()
        for row in overlay {
            for cell in row.cells where cell.active && echoAck < state(of: cell.batch) {
                notePending(since: cell.predictionTime, now: now)
            }
        }
        return before != (showing, drawFlagged)
    }

    // MARK: - Keystrokes (mosh's new_user_byte)

    private func handle(_ event: InputScanner.Event, screen: Screen, batch: Int, now: UInt64) {
        switch event {
        case .print(let scalar) where scalar.value == 0x7F:
            backspace(screen: screen, batch: batch, now: now)
        case .print(let scalar):
            if Self.predictable(scalar) {
                insert(scalar, screen: screen, batch: batch, now: now)
            } else {
                becomeTentative()
            }
        case .execute(let byte):
            becomeTentative()
            if byte == 0x0D || byte == 0x0A { cursorBarrier = batch }
        case .csi(let final, let plain) where plain && final == UInt8(ascii: "C"):
            moveRight(screen: screen, batch: batch, now: now)
        case .csi(let final, let plain) where plain && final == UInt8(ascii: "D"):
            moveLeft(screen: screen, batch: batch, now: now)
        case .csi, .escape, .string:
            becomeTentative()
        }
    }

    private func insert(_ scalar: Unicode.Scalar, screen: Screen, batch: Int, now: UInt64) {
        guard cursorBarrier == nil else { becomeTentative(); return }
        initCursor(screen, batch: batch)
        var cursor = cursors[cursors.count - 1]
        let width = screen.cols
        guard cursor.row >= 0, cursor.row < screen.rows, cursor.col >= 0, cursor.col < width else {
            becomeTentative()
            return
        }
        let r = rowIndex(cursor.row, width: width)
        if cursor.col + 1 >= width {
            // The last column is tricky — an editor may show a wrap mark, a
            // shell just puts the character there.
            becomeTentative()
        }
        // Heuristic: the new character looks like the one to its left.
        let attribute = cursor.col > 0
            ? attributeLeft(ofCol: cursor.col, row: cursor.row, r: r, screen: screen)
            : screen.rendition
        let ghost = ghostTail(at: cursor.col, row: cursor.row, r: r, screen: screen, text: attribute)

        // Shift the rest of the row right by one.
        var i = width - 1
        while i > cursor.col {
            var cell = overlay[r].cells[i]
            cell.resetWithOriginal()
            cell.active = true
            cell.tentativeUntilEpoch = predictionEpoch
            cell.expire(batch: batch, now: now)
            cell.originalContents.append(screen.cell(cursor.row, i))
            let prev = overlay[r].cells[i - 1]
            if i == width - 1 || ghost {
                cell.unknown = true
            } else if prev.active {
                if prev.unknown {
                    cell.unknown = true
                } else {
                    cell.unknown = false
                    cell.replacement = prev.replacement
                }
            } else {
                cell.unknown = false
                cell.replacement = screen.cell(cursor.row, i - 1)
            }
            overlay[r].cells[i] = cell
            i -= 1
        }

        var cell = overlay[r].cells[cursor.col]
        cell.resetWithOriginal()
        cell.active = true
        cell.tentativeUntilEpoch = predictionEpoch
        cell.expire(batch: batch, now: now)
        cell.replacement = screen.terminal.makeCharData(attribute: attribute, scalar: scalar)
        cell.originalContents.append(screen.cell(cursor.row, cursor.col))
        overlay[r].cells[cursor.col] = cell

        cursor.expire(batch: batch, now: now)
        if cursor.col < width - 1 {
            cursor.col += 1
        } else {
            becomeTentative()
            cursorBarrier = batch
        }
        cursors[cursors.count - 1] = cursor
    }

    private func backspace(screen: Screen, batch: Int, now: UInt64) {
        guard cursorBarrier == nil else { becomeTentative(); return }
        initCursor(screen, batch: batch)
        var cursor = cursors[cursors.count - 1]
        let width = screen.cols
        guard cursor.row >= 0, cursor.row < screen.rows, cursor.col > 0, cursor.col < width else { return }
        let r = rowIndex(cursor.row, width: width)
        // A wide character to the left goes two columns at once.
        let erased = displayed(cursor.row, cursor.col - 1, r: r, screen: screen)
        if erased.width == 0 {
            becomeTentative()
            cursorBarrier = batch
            return
        }
        let ghost = ghostTail(at: cursor.col, row: cursor.row, r: r, screen: screen, text: erased.attribute)

        cursor.col -= 1
        cursor.expire(batch: batch, now: now)
        cursors[cursors.count - 1] = cursor

        // Shift the rest of the row left by one.
        for i in cursor.col..<width {
            var cell = overlay[r].cells[i]
            cell.resetWithOriginal()
            cell.active = true
            cell.tentativeUntilEpoch = predictionEpoch
            cell.expire(batch: batch, now: now)
            cell.originalContents.append(screen.cell(cursor.row, i))
            if ghost {
                // The erased character goes; what the suggestion turns into
                // is the server's to draw.
                if i == cursor.col {
                    cell.unknown = false
                    cell.replacement = screen.terminal.makeCharData(attribute: erased.attribute, scalar: " ")
                } else {
                    cell.unknown = true
                }
            } else if i + 2 < width {
                let next = overlay[r].cells[i + 1]
                if next.active {
                    if next.unknown {
                        cell.unknown = true
                    } else {
                        cell.unknown = false
                        cell.replacement = next.replacement
                    }
                } else {
                    cell.unknown = false
                    cell.replacement = screen.cell(cursor.row, i + 1)
                }
            } else {
                cell.unknown = true
            }
            overlay[r].cells[i] = cell
        }
    }

    private func moveRight(screen: Screen, batch: Int, now: UInt64) {
        guard cursorBarrier == nil else { becomeTentative(); return }
        initCursor(screen, batch: batch)
        var cursor = cursors[cursors.count - 1]
        let width = screen.cols
        guard cursor.row >= 0, cursor.row < screen.rows, cursor.col >= 0, cursor.col < width - 1 else { return }
        let r = rowIndex(cursor.row, width: width)
        // Past the end of the text a line editor doesn't move (or jumps to
        // accept a suggestion) — mosh guesses one column and is wrong.
        if (cursor.col..<width).allSatisfy({ isBlank(displayed(cursor.row, $0, r: r, screen: screen), screen) }) {
            becomeTentative()
            return
        }
        if displayed(cursor.row, cursor.col + 1, r: r, screen: screen).width == 0 {
            becomeTentative()
            cursorBarrier = batch
            return
        }
        cursor.col += 1
        cursor.expire(batch: batch, now: now)
        cursors[cursors.count - 1] = cursor
    }

    private func moveLeft(screen: Screen, batch: Int, now: UInt64) {
        guard cursorBarrier == nil else { becomeTentative(); return }
        initCursor(screen, batch: batch)
        var cursor = cursors[cursors.count - 1]
        guard cursor.row >= 0, cursor.row < screen.rows, cursor.col > 0, cursor.col < screen.cols else { return }
        let r = rowIndex(cursor.row, width: screen.cols)
        if displayed(cursor.row, cursor.col - 1, r: r, screen: screen).width == 0 {
            becomeTentative()
            cursorBarrier = batch
            return
        }
        cursor.col -= 1
        cursor.expire(batch: batch, now: now)
        cursors[cursors.count - 1] = cursor
    }

    /// Characters the engine can place: one column, no combining, no
    /// controls. Everything from U+1100 up (Hangul, CJK, emoji…) may be
    /// double width — leave it to the server.
    static func predictable(_ scalar: Unicode.Scalar) -> Bool {
        let value = scalar.value
        guard value >= 0x20, !(0x7F...0x9F).contains(value), value < 0x1100 else { return false }
        switch scalar.properties.generalCategory {
        case .nonspacingMark, .spacingMark, .enclosingMark, .format, .control,
             .surrogate, .privateUse, .unassigned:
            return false
        default:
            return true
        }
    }

    // MARK: - Judging (mosh's cull)

    private func cull(_ screen: Screen, batch: Int) {
        guard mode != .never else { return }
        if screen.cols != lastSize.cols || screen.rows != lastSize.rows {
            lastSize = (screen.cols, screen.rows)
            reset()
        }
        let now = clock()
        updateTriggers()

        var i = 0
        while i < overlay.count {
            let row = overlay[i].row
            if row < 0 || row >= screen.rows {
                overlay.remove(at: i)
                continue
            }
            for j in overlay[i].cells.indices {
                let cell = overlay[i].cells[j]
                switch validity(of: cell, row: row, screen: screen) {
                case .incorrectOrExpired:
                    if mode == .experimental {
                        overlay[i].cells[j].reset()
                    } else if cell.tentative(confirmedEpoch) {
                        killEpoch(cell.tentativeUntilEpoch, screen: screen, batch: batch)
                    } else {
                        reset()
                        return
                    }
                case .correct:
                    if cell.tentativeUntilEpoch > confirmedEpoch {
                        confirmedEpoch = cell.tentativeUntilEpoch
                    }
                    // Quick confirmations slowly take the glitch trigger away.
                    if now &- cell.predictionTime < Self.glitchThreshold, glitchTrigger > 0,
                       now >= lastQuickConfirmation &+ Self.glitchRepairMinInterval {
                        glitchTrigger -= 1
                        lastQuickConfirmation = now
                    }
                    matchRenditions(row: i, from: j, screen: screen)
                    overlay[i].cells[j].reset()
                case .correctNoCredit:
                    // No credit, but the server did draw this character here
                    // — its colours are still the truth for the guesses after
                    // it. (mosh skips this; typing along a zsh autosuggestion
                    // is all no-credit, so the first word's "unknown command"
                    // red rode every guess of the burst.)
                    if !cell.unknown, !isBlank(cell.replacement, screen),
                       contentsMatch(screen.cell(row, cell.col), cell.replacement, screen) {
                        matchRenditions(row: i, from: j, screen: screen)
                    }
                    overlay[i].cells[j].reset()
                case .pending:
                    notePending(since: cell.predictionTime, now: now)
                case .inactive:
                    break
                }
            }
            i += 1
        }

        if let barrier = cursorBarrier, echoAck >= state(of: barrier) {
            // The server has had the keystroke long enough: its cursor is
            // the truth again, and the next guess starts from it. Guesses
            // from before the barrier can't be judged against where it
            // went.
            cursorBarrier = nil
            cursors.removeAll()
        }

        if let last = cursors.last, validity(of: last, screen: screen) == .incorrectOrExpired {
            if mode == .experimental {
                cursors.removeAll()
            } else {
                reset()
                return
            }
        }
        cursors.removeAll { validity(of: $0, screen: screen) != .pending }
        pruneBatches()
    }

    /// Match the rest of the row to the renditions it really got.
    private func matchRenditions(row i: Int, from j: Int, screen: Screen) {
        let actual = screen.cell(overlay[i].row, overlay[i].cells[j].col).attribute
        for k in j..<overlay[i].cells.count {
            overlay[i].cells[k].replacement.attribute = actual
        }
    }

    private func updateTriggers() {
        let interval = sendInterval
        if interval > Self.srttTriggerHigh {
            srttTrigger = true
        } else if srttTrigger, interval <= Self.srttTriggerLow, !isActive {
            srttTrigger = false
        }
        if interval > Self.flagTriggerHigh {
            flagging = true
        } else if interval <= Self.flagTriggerLow {
            flagging = false
        }
        if glitchTrigger > Self.glitchRepairCount { flagging = true }
    }

    /// A guess still unconfirmed this long is a glitch: show predictions even
    /// on a fast link, and past five seconds mark them too.
    private func notePending(since predictionTime: UInt64, now: UInt64) {
        let age = now &- predictionTime
        if age >= Self.glitchFlagThreshold {
            glitchTrigger = Self.glitchRepairCount * 2
        } else if age >= Self.glitchThreshold, glitchTrigger < Self.glitchRepairCount {
            glitchTrigger = Self.glitchRepairCount
        }
    }

    private func validity(of cell: Cell, row: Int, screen: Screen) -> Validity {
        guard cell.active else { return .inactive }
        guard row < screen.rows, cell.col < screen.cols else { return .incorrectOrExpired }
        if echoAck < state(of: cell.batch) { return .pending }
        if cell.unknown { return .correctNoCredit }
        // Too easy for a blank to match by accident.
        if isBlank(cell.replacement, screen) { return .correctNoCredit }
        let current = screen.cell(row, cell.col)
        guard contentsMatch(current, cell.replacement, screen) else { return .incorrectOrExpired }
        return cell.originalContents.contains { contentsMatch($0, cell.replacement, screen) }
            ? .correctNoCredit : .correct
    }

    private func validity(of cursor: CursorMove, screen: Screen) -> Validity {
        guard cursor.active else { return .inactive }
        guard cursor.row < screen.rows, cursor.col < screen.cols else { return .incorrectOrExpired }
        guard echoAck >= state(of: cursor.batch) else { return .pending }
        return screen.cursorRow == cursor.row && screen.cursorCol == cursor.col ? .correct : .incorrectOrExpired
    }

    private func killEpoch(_ epoch: UInt64, screen: Screen, batch: Int) {
        cursors.removeAll { $0.tentativeUntilEpoch >= epoch }
        cursors.append(CursorMove(row: screen.cursorRow, col: screen.cursorCol,
                                  tentativeUntilEpoch: predictionEpoch, batch: batch))
        for i in overlay.indices {
            for j in overlay[i].cells.indices where overlay[i].cells[j].tentativeUntilEpoch >= epoch {
                overlay[i].cells[j].reset()
            }
        }
        becomeTentative()
    }

    private func becomeTentative() {
        if mode != .experimental { predictionEpoch += 1 }
    }

    private func initCursor(_ screen: Screen, batch: Int) {
        if cursors.isEmpty {
            cursors.append(CursorMove(row: screen.cursorRow, col: screen.cursorCol,
                                      tentativeUntilEpoch: predictionEpoch, batch: batch))
        } else if let last = cursors.last, last.tentativeUntilEpoch != predictionEpoch {
            cursors.append(CursorMove(row: last.row, col: last.col,
                                      tentativeUntilEpoch: predictionEpoch, batch: batch))
        }
    }

    private func rowIndex(_ row: Int, width: Int) -> Int {
        if let index = overlay.firstIndex(where: { $0.row == row }) { return index }
        overlay.append(Row(row: row, cells: (0..<width).map { Cell(col: $0, epoch: predictionEpoch) }))
        return overlay.count - 1
    }

    private func state(of batch: Int) -> UInt64 { batchStates[batch] ?? .max }

    private func pruneBatches() {
        var oldest = nextBatch
        for row in overlay {
            for cell in row.cells where cell.active { oldest = min(oldest, cell.batch) }
        }
        for cursor in cursors { oldest = min(oldest, cursor.batch) }
        if let cursorBarrier { oldest = min(oldest, cursorBarrier) }
        batchStates = batchStates.filter { $0.key >= oldest }
    }

    private var isActive: Bool {
        !cursors.isEmpty || overlay.contains { $0.cells.contains { $0.active } }
    }

    /// mosh's `send_interval`: half the round trip, clamped to 20…250ms.
    private var sendInterval: UInt64 {
        let srtt = srttMs > 0 ? srttMs : 1000
        return UInt64(min(max((srtt / 2).rounded(.up), 20), 250))
    }

    private var showing: Bool {
        switch mode {
        case .never: return false
        case .always, .experimental: return true
        case .adaptive: return srttTrigger || glitchTrigger > 0
        }
    }

    private var drawFlagged: Bool { trail && flagging }

    // MARK: - Reading the screen

    /// The server's screen as SwiftTerm holds it — read with the overlay
    /// withdrawn, viewport at the live bottom.
    private struct Screen {
        let terminal: Terminal
        var cols: Int { terminal.cols }
        var rows: Int { terminal.rows }
        var cursorRow: Int { terminal.buffer.y }
        /// SwiftTerm parks x one past the edge while a wrap is pending; mosh
        /// keeps the cursor on the last column.
        var cursorCol: Int { min(terminal.buffer.x, terminal.cols - 1) }
        var rendition: Attribute { terminal.currentAttribute }
        func cell(_ row: Int, _ col: Int) -> CharData {
            guard col >= 0, col < cols, let line = terminal.getLine(row: row) else { return .Null }
            return line[col]
        }
    }

    /// The cell as the prediction would show it.
    private func displayed(_ row: Int, _ col: Int, r: Int, screen: Screen) -> CharData {
        let cell = overlay[r].cells[col]
        return cell.active && !cell.unknown ? cell.replacement : screen.cell(row, col)
    }

    /// Whether what follows the cursor is drawn unlike the text being edited
    /// — a shell's autosuggestion, a prompt's placeholder. Such a tail isn't
    /// pushed along by typing (mosh assumes it is): it's consumed or redrawn,
    /// and only the server knows which, so its cells are left unknown.
    /// Shifting it drew doubled letters for a round trip and then, judged
    /// wrong, threw every guess away — typing along a zsh autosuggestion
    /// predicted nothing. A blank (or a TUI's drawn cursor block) is no tail.
    private func ghostTail(at col: Int, row: Int, r: Int, screen: Screen, text: Attribute) -> Bool {
        let here = displayed(row, col, r: r, screen: screen)
        return !isBlank(here, screen) && here.attribute != text
    }

    /// The rendition to give a character typed at `col`: its left neighbour's
    /// — skipping a wide character's continuation cell, whose inverted
    /// background is a placeholder, not a colour anyone chose.
    private func attributeLeft(ofCol col: Int, row: Int, r: Int, screen: Screen) -> Attribute {
        var left = col - 1
        if left > 0, displayed(row, left, r: r, screen: screen).width == 0 { left -= 1 }
        return displayed(row, left, r: r, screen: screen).attribute
    }

    private func isBlank(_ cell: CharData, _ screen: Screen) -> Bool {
        let character = screen.terminal.getCharacter(for: cell)
        return character == " " || character == "\u{0}"
    }

    private func contentsMatch(_ a: CharData, _ b: CharData, _ screen: Screen) -> Bool {
        if isBlank(a, screen), isBlank(b, screen) { return true }
        return a.width == b.width && screen.terminal.getCharacter(for: a) == screen.terminal.getCharacter(for: b)
    }

    // MARK: - The overlay

    private struct Write {
        let line: BufferLine
        let col: Int
        let original: CharData
    }

    private var writes: [Write] = []
    private var cursorBefore: (x: Int, y: Int)?
    private var appliedTo: (terminal: ObjectIdentifier, buffer: ObjectIdentifier, cols: Int, rows: Int)?
    private var dirty: ClosedRange<Int>?
    private var dimmedAttributes: [Attribute: Attribute] = [:]

    /// Whether anything is drawn into the terminal right now.
    var isApplied: Bool { appliedTo != nil }

    /// Draw the guesses that should show. Call after ``withdraw(from:)``,
    /// with the viewport at the live bottom. Returns true if anything was
    /// drawn (the view then needs a redraw).
    @discardableResult
    func apply(to terminal: Terminal) -> Bool {
        guard appliedTo == nil, showing else { return false }
        let screen = Screen(terminal: terminal)
        let buffer = terminal.buffer
        let flag = drawFlagged

        // The newest shown cursor guess wins; one that agrees with the
        // server's cursor leaves it alone (and its pending-wrap state too).
        if let cursor = cursors.last(where: { $0.active && !$0.tentative(confirmedEpoch) }),
           cursor.row >= 0, cursor.row < screen.rows, cursor.col >= 0, cursor.col < screen.cols,
           (cursor.row, cursor.col) != (screen.cursorRow, screen.cursorCol) {
            cursorBefore = (buffer.x, buffer.y)
            markDirty(buffer.y)
            buffer.x = cursor.col
            buffer.y = cursor.row
            markDirty(cursor.row)
        }

        for row in overlay {
            guard let line = terminal.getLine(row: row.row) else { continue }
            for cell in row.cells where cell.active && !cell.tentative(confirmedEpoch) && cell.col < screen.cols {
                let current = line[cell.col]
                let flagged = flag && !(isBlank(cell.replacement, screen) && isBlank(current, screen))
                if cell.unknown {
                    if flagged, cell.col != screen.cols - 1 {
                        var marked = current
                        marked.attribute = dimmed(current.attribute)
                        write(marked, to: line, col: cell.col, row: row.row)
                    }
                    continue
                }
                guard current.attribute != cell.replacement.attribute
                        || !contentsMatch(current, cell.replacement, screen) else { continue }
                var replacement = cell.replacement
                if flagged { replacement.attribute = dimmed(replacement.attribute) }
                write(replacement, to: line, col: cell.col, row: row.row)
            }
        }

        guard cursorBefore != nil || !writes.isEmpty else { return false }
        appliedTo = (ObjectIdentifier(terminal), ObjectIdentifier(buffer), terminal.cols, terminal.rows)
        if let dirty { terminal.refresh(startRow: dirty.lowerBound, endRow: dirty.upperBound) }
        return true
    }

    /// Put back exactly what ``apply(to:)`` covered. A terminal that was
    /// resized (or swapped) since is left alone: its cells moved, and the
    /// server repaints a resized screen from scratch anyway. Returns true if
    /// anything was restored.
    @discardableResult
    func withdraw(from terminal: Terminal) -> Bool {
        defer {
            writes.removeAll()
            cursorBefore = nil
            appliedTo = nil
            dirty = nil
        }
        guard let appliedTo,
              appliedTo.terminal == ObjectIdentifier(terminal),
              appliedTo.buffer == ObjectIdentifier(terminal.buffer),
              appliedTo.cols == terminal.cols, appliedTo.rows == terminal.rows
        else { return false }
        for write in writes.reversed() where write.col < write.line.count {
            write.line[write.col] = write.original
        }
        if let cursorBefore {
            terminal.buffer.x = cursorBefore.x
            terminal.buffer.y = cursorBefore.y
        }
        if let dirty { terminal.refresh(startRow: dirty.lowerBound, endRow: dirty.upperBound) }
        return true
    }

    private func write(_ cell: CharData, to line: BufferLine, col: Int, row: Int) {
        writes.append(Write(line: line, col: col, original: line[col]))
        line[col] = cell
        markDirty(row)
    }

    private func markDirty(_ row: Int) {
        dirty = dirty.map { min($0.lowerBound, row)...max($0.upperBound, row) } ?? row...row
    }

    private func dimmed(_ attribute: Attribute) -> Attribute {
        if attribute.style.contains(.dim) { return attribute }
        if let known = dimmedAttributes[attribute] { return known }
        let made = AttributeForge.adding(dim: attribute)
        dimmedAttributes[attribute] = made
        return made
    }
}

// MARK: - Attributes

/// SwiftTerm builds `Attribute`s only from SGR, so a dimmed copy of one is
/// made the same way: spell it out as SGR, add `2`, and read back what a
/// scratch terminal parsed.
enum AttributeForge {
    private final class Sink: TerminalDelegate {
        func send(source: Terminal, data: ArraySlice<UInt8>) {}
    }

    private static let sink = Sink()
    private static let scratch = Terminal(delegate: sink, options: TerminalOptions(cols: 4, rows: 1, scrollback: 0))

    /// `attribute` plus faint — or `attribute` unchanged when SGR can't
    /// reproduce it faithfully.
    static func adding(dim attribute: Attribute) -> Attribute {
        guard let base = sgr(attribute) else { return attribute }
        scratch.feed(text: "\u{1b}[\(base);2m")
        let made = scratch.currentAttribute
        guard made.fg == attribute.fg, made.bg == attribute.bg,
              made.style == attribute.style.union(.dim),
              made.underlineStyle == attribute.underlineStyle,
              made.underlineColor == attribute.underlineColor
        else { return attribute }
        return made
    }

    /// The SGR parameters that recreate `attribute` from a reset, or nil
    /// when it holds something SGR has no words for.
    static func sgr(_ attribute: Attribute) -> String? {
        var parts = ["0"]
        let style = attribute.style
        if style.contains(.bold) { parts.append("1") }
        if style.contains(.dim) { parts.append("2") }
        if style.contains(.italic) { parts.append("3") }
        if style.contains(.underline) {
            switch attribute.underlineStyle {
            case .none, .single: parts.append("4")
            case .double: parts.append("4:2")
            case .curly: parts.append("4:3")
            case .dotted: parts.append("4:4")
            case .dashed: parts.append("4:5")
            }
        }
        if style.contains(.blink) { parts.append("5") }
        if style.contains(.inverse) { parts.append("7") }
        if style.contains(.invisible) { parts.append("8") }
        if style.contains(.crossedOut) { parts.append("9") }
        guard let fg = color(attribute.fg, base: 38),
              let bg = color(attribute.bg, base: 48) else { return nil }
        parts += fg + bg
        if let underline = attribute.underlineColor {
            guard let spelled = color(underline, base: 58) else { return nil }
            parts += spelled
        }
        return parts.joined(separator: ";")
    }

    private static func color(_ color: Attribute.Color, base: Int) -> [String]? {
        switch color {
        case .defaultColor: return []
        case .ansi256(let code): return ["\(base)", "5", "\(code)"]
        case .trueColor(let r, let g, let b): return ["\(base)", "2", "\(r)", "\(g)", "\(b)"]
        case .defaultInvertedColor: return nil
        }
    }
}

// MARK: - Keystroke scanner

/// Splits outgoing keystroke bytes into what the engine cares about — a
/// small VT input parser (mosh runs its full terminal parser here). State
/// carries across calls, so a sequence split between sends still parses.
struct InputScanner {
    enum Event: Equatable {
        case print(Unicode.Scalar)
        /// A C0 control.
        case execute(UInt8)
        /// `ESC x` (Alt-x and friends).
        case escape
        /// A CSI (or SS3, which mosh folds into CSI) sequence; `plain` = no
        /// parameters or intermediates — `ESC [ C` but not `ESC [ 1 ; 5 C`.
        case csi(final: UInt8, plain: Bool)
        /// An OSC/DCS/APC/PM/SOS string.
        case string
    }

    private enum State { case ground, escape, escapeIntermediate, csi, ss3, string, stringEscape }

    private var state = State.ground
    private var plain = true
    private var utf8: [UInt8] = []
    private var utf8Needed = 0

    mutating func scan(_ data: Data) -> [Event] {
        var events: [Event] = []
        for byte in data { feed(byte, into: &events) }
        return events
    }

    private mutating func feed(_ byte: UInt8, into events: inout [Event]) {
        // CAN / SUB abort any sequence.
        if byte == 0x18 || byte == 0x1A {
            dropUTF8()
            state = .ground
            events.append(.execute(byte))
            return
        }
        switch state {
        case .ground:
            ground(byte, into: &events)

        case .escape:
            switch byte {
            case 0x1B: break
            case 0x5B: state = .csi; plain = true
            case 0x4F: state = .ss3
            case 0x50, 0x58, 0x5D, 0x5E, 0x5F: state = .string
            case 0x20...0x2F: state = .escapeIntermediate
            case 0x30...0x7E: state = .ground; events.append(.escape)
            case 0x7F: break
            case 0x00...0x1F: events.append(.execute(byte))
            default:
                state = .ground
                events.append(.escape)
                ground(byte, into: &events)
            }

        case .escapeIntermediate:
            switch byte {
            case 0x1B: state = .escape
            case 0x20...0x2F, 0x7F: break
            case 0x30...0x7E: state = .ground; events.append(.escape)
            case 0x00...0x1F: events.append(.execute(byte))
            default: state = .ground; events.append(.escape)
            }

        case .csi:
            switch byte {
            case 0x1B: state = .escape
            case 0x20...0x3F: plain = false
            case 0x40...0x7E: state = .ground; events.append(.csi(final: byte, plain: plain))
            case 0x7F: break
            case 0x00...0x1F: events.append(.execute(byte))
            default: state = .ground; events.append(.escape)
            }

        case .ss3:
            switch byte {
            case 0x1B: state = .escape
            case 0x40...0x7E: state = .ground; events.append(.csi(final: byte, plain: true))
            case 0x00...0x1F: events.append(.execute(byte))
            default: state = .ground; events.append(.escape)
            }

        case .string:
            if byte == 0x07 {
                state = .ground
                events.append(.string)
            } else if byte == 0x1B {
                state = .stringEscape
            }

        case .stringEscape:
            events.append(.string)
            state = .escape
            if byte != 0x5C { feed(byte, into: &events) } else { state = .ground }
        }
    }

    private mutating func ground(_ byte: UInt8, into events: inout [Event]) {
        if utf8Needed > 0 {
            if byte & 0xC0 == 0x80 {
                utf8.append(byte)
                utf8Needed -= 1
                if utf8Needed == 0 {
                    let scalar = String(decoding: utf8, as: UTF8.self).unicodeScalars.first ?? "\u{FFFD}"
                    utf8.removeAll()
                    events.append(.print(scalar))
                }
                return
            }
            dropUTF8()
            events.append(.print("\u{FFFD}"))
        }
        switch byte {
        case 0x1B:
            state = .escape
        case 0x00...0x1F:
            events.append(.execute(byte))
        case 0x20...0x7F:
            events.append(.print(Unicode.Scalar(byte)))
        case 0xC2...0xDF:
            utf8 = [byte]; utf8Needed = 1
        case 0xE0...0xEF:
            utf8 = [byte]; utf8Needed = 2
        case 0xF0...0xF4:
            utf8 = [byte]; utf8Needed = 3
        default:
            events.append(.print("\u{FFFD}"))
        }
    }

    private mutating func dropUTF8() {
        utf8.removeAll()
        utf8Needed = 0
    }
}
