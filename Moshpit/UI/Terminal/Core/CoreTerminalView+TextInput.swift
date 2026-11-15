import MoshpitFFI
import MoshpitKit
import UIKit

// UITextInput over the core's input model (crates/term/src/ime.rs,
// docs/nsh/TERMINAL-VIEW.md V5–V11). The shadow document, the phantom
// newline, what a delete or a replacement sends, whether a change is the
// keyboard's own: all decided in Rust. This adapter only translates
// UIKit's position objects to grapheme offsets and back, sends the bytes
// the core returns, tells the input delegate when the core says so, and
// draws the composition at the cursor.

/// A position in the shadow document, in graphemes.
final class CoreTextPosition: UITextPosition {
    let offset: Int
    init(_ offset: Int) { self.offset = offset }
    override var description: String { "\(offset)" }
}

/// A range in the shadow document (ordered).
final class CoreTextRange: UITextRange {
    let lower: Int
    let upper: Int
    init(_ a: Int, _ b: Int) {
        lower = min(a, b)
        upper = max(a, b)
    }
    override var start: UITextPosition { CoreTextPosition(lower) }
    override var end: UITextPosition { CoreTextPosition(upper) }
    override var isEmpty: Bool { lower >= upper }
    override var description: String { "[\(lower)..<\(upper)]" }
}

private final class CoreTextSelectionRect: UITextSelectionRect {
    private let r: CGRect
    init(_ r: CGRect) { self.r = r }
    override var rect: CGRect { r }
    override var writingDirection: NSWritingDirection { .leftToRight }
    override var containsStart: Bool { true }
    override var containsEnd: Bool { true }
    override var isVertical: Bool { false }
}

extension CoreTerminalView: UITextInput {
    // Traits: nothing a keyboard rewrites, fills or offers to insert
    // (patches 47, 60, V11). Spell checking is on: it gates the strip input
    // methods draw their candidates in (TerminalKeyboard's rule);
    // autocorrection stays off. Get and an ignored set: UIKit's traits are
    // settable requirements.
    var keyboardType: UIKeyboardType { get { .default } set {} }
    // The keyboard of the terminal's appearance, its colour scheme's (R12,
    // R18): light under a light scheme, dark otherwise, whatever the app's.
    var keyboardAppearance: UIKeyboardAppearance {
        get { traitCollection.userInterfaceStyle == .light ? .light : .dark }
        set {}
    }
    var returnKeyType: UIReturnKeyType { get { .default } set {} }
    var textContentType: UITextContentType! { get { UITextContentType(rawValue: "terminal") } set {} }
    var autocapitalizationType: UITextAutocapitalizationType { get { .none } set {} }
    var autocorrectionType: UITextAutocorrectionType { get { .no } set {} }
    var spellCheckingType: UITextSpellCheckingType { get { .yes } set {} }
    var smartQuotesType: UITextSmartQuotesType { get { .no } set {} }
    var smartDashesType: UITextSmartDashesType { get { .no } set {} }
    var smartInsertDeleteType: UITextSmartInsertDeleteType { get { .no } set {} }
    var enablesReturnKeyAutomatically: Bool { get { false } set {} }
    var isSecureTextEntry: Bool { get { false } set {} }
    var inlinePredictionType: UITextInlinePredictionType { get { .no } set {} }
    var mathExpressionCompletionType: UITextMathExpressionCompletionType { get { .no } set {} }
    var writingToolsBehavior: UIWritingToolsBehavior { get { .none } set {} }

    override var canBecomeFirstResponder: Bool { true }
    /// Never a stop of the keyboard's focus: Tab is the terminal's (V25).
    override var canBecomeFocused: Bool { false }

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        if ok {
            sendFocus(true)
            ime { mp_term_ime_focus($0, true) }
        }
        return ok
    }

    override func resignFirstResponder() -> Bool {
        let ok = super.resignFirstResponder()
        if ok {
            sendFocus(false)
            ime { mp_term_ime_focus($0, false) }
            keyRepeat?.invalidate()
            keyRepeat = nil
        }
        return ok
    }

    private func sendFocus(_ focused: Bool) {
        guard let term else { return }
        let b = mp_term_focus(term, focused)
        if let p = b.ptr { sendReply(Data(bytes: p, count: b.len)) }
        mp_bytes_free(b)
    }

    // MARK: Calls into the core's input model

    /// Runs one input-model call: sends what it returns (lines inserted at
    /// once go the session's paste way, B414), tells the input delegate
    /// when the core says the change was not the keyboard's own (V9),
    /// refreshes the composition on screen.
    private func ime(_ call: (OpaquePointer) -> MpImeResult) {
        guard let term else { return }
        let result = call(term)
        if result.notify {
            inputDelegate?.selectionWillChange(self)
            inputDelegate?.textWillChange(self)
        }
        imeState = mp_term_ime_state(term)
        if let p = result.bytes.ptr, result.bytes.len > 0 {
            surfaceDelegate?.surface(self, typed: Data(bytes: p, count: result.bytes.len))
        }
        mp_bytes_free(result.bytes)
        if let p = result.paste {
            let text = String(cString: p)
            mp_string_free(p)
            if !text.isEmpty { surfaceDelegate?.surface(self, paste: text) }
        }
        if result.notify {
            inputDelegate?.textDidChange(self)
            inputDelegate?.selectionDidChange(self)
        }
        updateMarkedText()
    }

    /// A key's bytes, encoded by the core; nothing when it has none.
    func sendKey(_ key: UInt32, scalar: UInt32 = 0, mods: UInt8, down: Bool = true, repeat count: UInt16 = 1) {
        guard let term else { return }
        let b = mp_term_key(term, key, scalar, mods, down, count)
        if let p = b.ptr, b.len > 0 {
            surfaceDelegate?.surface(self, typed: Data(bytes: p, count: b.len))
        }
        mp_bytes_free(b)
    }

    /// Typed text through the input model, as the keyboard types it.
    func sendText(_ text: String) {
        ime { t in text.withCString { mp_term_ime_insert(t, $0) } }
    }

    private var composing: Bool { imeState.marked_start >= 0 }

    private func clamp(_ i: Int) -> Int { max(0, min(i, Int(imeState.len))) }

    // MARK: UIKeyInput

    /// Always: a terminal has a screen behind its caret (V8).
    var hasText: Bool { true }

    func insertText(_ text: String) {
        let anchor = composing ? markedLabel?.frame : nil
        ime { t in text.withCString { mp_term_ime_insert(t, $0) } }
        if let anchor { holdCaretAfterCommit(at: anchor) }
    }

    func deleteBackward() {
        ime { mp_term_ime_delete($0) }
        // A keyboard puts back what it deleted within this turn; a Return
        // after it is the person's (B410).
        DispatchQueue.main.async { [weak self] in
            guard let term = self?.term else { return }
            mp_term_ime_turn_end(term)
        }
    }

    // MARK: UITextInput: the document

    func text(in range: UITextRange) -> String? {
        guard let term, let r = range as? CoreTextRange else { return nil }
        if r.isEmpty { return "" }
        guard let p = mp_term_ime_text(term, UInt32(clamp(r.lower)), UInt32(clamp(r.upper))) else { return "" }
        defer { mp_string_free(p) }
        return String(cString: p)
    }

    func replace(_ range: UITextRange, withText text: String) {
        guard let r = range as? CoreTextRange else { return }
        let (a, b) = (UInt32(clamp(r.lower)), UInt32(clamp(r.upper)))
        ime { t in text.withCString { mp_term_ime_replace(t, a, b, $0) } }
    }

    var selectedTextRange: UITextRange? {
        get { CoreTextRange(Int(imeState.sel_start), Int(imeState.sel_end)) }
        set {
            // The input system moves its own caret: nothing to report (V9).
            guard let term, let r = newValue as? CoreTextRange else { return }
            mp_term_ime_select(term, UInt32(clamp(r.lower)), UInt32(clamp(r.upper)))
            imeState = mp_term_ime_state(term)
        }
    }

    var markedTextRange: UITextRange? {
        composing ? CoreTextRange(Int(imeState.marked_start), Int(imeState.marked_end)) : nil
    }

    func setMarkedText(_ markedText: String?, selectedRange sel: NSRange) {
        // UIKit's range is UTF-16 within the composition; the core counts
        // graphemes.
        var start = 0, length = 0
        if let markedText {
            let ns = markedText as NSString
            let location = min(sel.location, ns.length)
            start = ns.substring(to: location).count
            length = ns.substring(with: NSRange(location: location, length: min(sel.length, ns.length - location))).count
        }
        let starting = !composing
        ime { t in
            guard let markedText else { return mp_term_ime_set_marked(t, nil, 0, 0) }
            return markedText.withCString { mp_term_ime_set_marked(t, $0, UInt32(start), UInt32(length)) }
        }
        // B691: the bar's Ctrl armed, a key an input method takes to start
        // composing (Pinyin's `c`) is the chord the person asked for, not a
        // composition: with a Chinese keyboard up, Ctrl-C from the bar
        // composed `c`, sent the candidate picked later, and spent the Ctrl
        // on it. Once the input method's call is over (it is not told of its
        // own edit inside it), the composition goes unsent and the key goes
        // as typed, which folds it.
        if starting, let markedText, let chord = Self.ctrlChordKey(markedText),
           surfaceDelegate?.surfaceCtrlArmed == true {
            DispatchQueue.main.async { [weak self] in
                guard let self, self.composing, self.surfaceDelegate?.surfaceCtrlArmed == true else { return }
                self.endComposition(cancel: true)
                self.surfaceDelegate?.surface(self, typed: Data([chord]))
            }
        }
    }

    /// The byte of `text` when it is one key a Ctrl chord folds (a letter,
    /// `@[\]^_`, Space, `/`, `?`): an input method's first marked character.
    nonisolated static func ctrlChordKey(_ text: String) -> UInt8? {
        let bytes = Array(text.utf8)
        guard bytes.count == 1, let byte = bytes.first else { return nil }
        switch byte {
        case 0x40...0x7e, 0x20, 0x2f, 0x3f: return byte
        default: return nil
        }
    }

    func unmarkText() {
        let anchor = composing ? markedLabel?.frame : nil
        ime { mp_term_ime_unmark($0) }
        if let anchor { holdCaretAfterCommit(at: anchor) }
    }

    var beginningOfDocument: UITextPosition { CoreTextPosition(0) }
    var endOfDocument: UITextPosition { CoreTextPosition(Int(imeState.len)) }

    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        guard let a = fromPosition as? CoreTextPosition, let b = toPosition as? CoreTextPosition else { return nil }
        return CoreTextRange(clamp(a.offset), clamp(b.offset))
    }

    func position(from position: UITextPosition, offset: Int) -> UITextPosition? {
        guard let p = position as? CoreTextPosition else { return nil }
        return CoreTextPosition(clamp(p.offset + offset))
    }

    func position(from position: UITextPosition, in direction: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        self.position(from: position, offset: offset)
    }

    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        guard let a = position as? CoreTextPosition, let b = other as? CoreTextPosition else { return .orderedSame }
        return a.offset < b.offset ? .orderedAscending : a.offset > b.offset ? .orderedDescending : .orderedSame
    }

    func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int {
        guard let a = from as? CoreTextPosition, let b = toPosition as? CoreTextPosition else { return 0 }
        return b.offset - a.offset
    }

    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        range.end
    }

    func characterRange(byExtending position: UITextPosition, in direction: UITextLayoutDirection) -> UITextRange? {
        guard let p = position as? CoreTextPosition else { return nil }
        return CoreTextRange(p.offset, Int(imeState.len))
    }

    func baseWritingDirection(for position: UITextPosition, in direction: UITextStorageDirection) -> NSWritingDirection {
        .leftToRight
    }

    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {}

    /// The caret's rectangle (the candidate bar and the system's
    /// caret-relative popovers sit by it): the caret the person sees. While
    /// composing that is the composition's, at its end, and after a commit
    /// the one held there until the echo; the cursor's cell otherwise.
    /// Output that moved the remote's cursor mid-composition (which doesn't
    /// move the composition, V7) took the candidates away from the pinyin
    /// (B694).
    func caretRect(for position: UITextPosition) -> CGRect {
        if markedLabel != nil, compositionCaret.superview != nil { return compositionCaret.frame }
        if let pendingCaret { return pendingCaret.frame }
        return cursorCellRect()
    }

    func firstRect(for range: UITextRange) -> CGRect { markedLabel?.frame ?? cursorCellRect() }

    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] {
        [CoreTextSelectionRect(markedLabel?.frame ?? cursorCellRect())]
    }

    func closestPosition(to point: CGPoint) -> UITextPosition? { selectedTextRange?.start }

    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? { range.start }

    func characterRange(at point: CGPoint) -> UITextRange? { CoreTextRange(0, Int(imeState.len)) }

    func position(within range: UITextRange, atCharacterOffset offset: Int) -> UITextPosition? {
        guard let r = range as? CoreTextRange, r.lower + offset <= r.upper else { return nil }
        return CoreTextPosition(r.lower + offset)
    }

    func characterOffset(of position: UITextPosition, within range: UITextRange) -> Int {
        guard let r = range as? CoreTextRange, let p = position as? CoreTextPosition else { return 0 }
        return p.offset - r.lower
    }

    // MARK: Dictation

    func insertDictationResult(_ dictationResult: [UIDictationPhrase]) {
        let text = dictationResult.map(\.text).joined()
        if !text.isEmpty { insertText(text) }
    }

    // MARK: The space bar's trackpad

    func beginFloatingCursor(at point: CGPoint) { lastFloatingCursorLocation = point }

    func updateFloatingCursor(at point: CGPoint) {
        guard let last = lastFloatingCursorLocation, let term else { return }
        let dx = last.x - point.x
        if abs(dx) > 5 {
            sendKey(UInt32(dx > 0 ? MP_KEY_LEFT : MP_KEY_RIGHT), mods: 0)
            let caret = clamp(Int(imeState.sel_start) + (dx > 0 ? -1 : 1))
            mp_term_ime_select(term, UInt32(caret), UInt32(caret))
            imeState = mp_term_ime_state(term)
            lastFloatingCursorLocation = point
        }
        if termModes.alt_screen {
            let dy = last.y - point.y
            if abs(dy) > 2 {
                sendKey(UInt32(dy > 0 ? MP_KEY_UP : MP_KEY_DOWN), mods: 0)
                lastFloatingCursorLocation = point
            }
        }
    }

    func endFloatingCursor() { lastFloatingCursorLocation = nil }

    // MARK: The composition, on screen

    /// The cursor's cell in this view.
    func cursorCellRect() -> CGRect {
        let c = rows.info.cursor
        return CGRect(x: x(ofColumn: Int(c.col)), y: y(ofRow: Int(c.row)),
                      width: metrics.width, height: metrics.height)
    }

    /// The composition the core holds, drawn as an underlined label where
    /// the cursor was when it began, with the caret at its end; the canvas
    /// draws no cursor meanwhile, so output that moves the remote's cursor
    /// does not move the caret (V5, V7).
    func updateMarkedText() {
        guard composing, let term,
              let p = mp_term_ime_text(term, UInt32(imeState.marked_start), UInt32(imeState.marked_end))
        else {
            markedLabel?.removeFromSuperview()
            markedLabel = nil
            if pendingCaret == nil { canvasCursorHidden = false }
            return
        }
        let text = String(cString: p)
        mp_string_free(p)
        let label: UILabel
        if let existing = markedLabel {
            label = existing
        } else {
            // Composing is typing: back on the live screen, where it lands,
            // as a key brings it back (B693). Begun while the view showed
            // the history, the composition sat at the cursor below the view.
            if isReadingScrollback { scrollToBottom() }
            label = UILabel()
            label.numberOfLines = 1
            label.isUserInteractionEnabled = false
            label.accessibilityIdentifier = "core-terminal-marked-text"
            addSubview(label)
            markedLabel = label
            markedAnchor = liveCursorCell
        }
        let fg = UIColor(cgColor: coreCGColor(style.foreground))
        label.attributedText = NSAttributedString(string: text, attributes: [
            .font: metrics.font, .foregroundColor: fg,
            .backgroundColor: UIColor(cgColor: coreCGColor(style.background)),
            .underlineStyle: NSUnderlineStyle.single.rawValue, .underlineColor: fg,
        ])
        label.sizeToFit()
        canvasCursorHidden = true
        pendingCaret?.removeFromSuperview()
        pendingCaret = nil
        positionMarkedText()
    }

    /// Where the composition sits: where the cursor was when it began —
    /// moved left as far as it takes to show it whole with its caret
    /// (B571). Near the right edge it ran past it: the view clips, so the
    /// pinyin being typed at the end of a long line, and the caret after
    /// it, were cut off or gone.
    func positionMarkedText() {
        guard let label = markedLabel else { return }
        let anchor = markedAnchor ?? liveCursorCell
        let caretWidth = max(2, metrics.width / 4)
        let x = Self.markedX(at: x(ofColumn: Int(anchor.col)), width: label.frame.width + caretWidth,
                             in: bounds.width)
        // On its row as the view shows it: a history scroll mid-composition
        // takes it down with the row (B693); it stayed put over old lines.
        label.frame.origin = CGPoint(x: x, y: y(ofRow: anchor.row + Int(rows.info.display_offset)))
        let caret = compositionCaret
        caret.backgroundColor = UIColor(cgColor: coreCGColor(style.cursorColor))
        caret.frame = CGRect(x: label.frame.maxX, y: label.frame.minY, width: caretWidth,
                             height: metrics.height)
        if caret.superview == nil { addSubview(caret) }
        bringSubviewToFront(label)
        bringSubviewToFront(caret)
    }

    /// Ends the composition from outside the input system (B573: a bar key
    /// pressed while composing): `cancel` takes it away unsent, else it is
    /// committed as it stands — what shows inline, the pinyin itself when
    /// no candidate was picked. The input system hears that the text
    /// changed under it, and lets go of its own composition.
    @discardableResult
    func endComposition(cancel: Bool) -> Bool {
        guard composing else { return false }
        inputDelegate?.textWillChange(self)
        if cancel {
            ime { mp_term_ime_set_marked($0, nil, 0, 0) }
        } else {
            let anchor = markedLabel?.frame
            ime { mp_term_ime_unmark($0) }
            if let anchor { holdCaretAfterCommit(at: anchor) }
        }
        inputDelegate?.textDidChange(self)
        return true
    }

    /// Where a composition `width` points wide (its caret too) starts, put
    /// at `x` in a view `viewWidth` wide: at `x` while it fits, else moved
    /// left to end at the view's edge (never past its left one).
    nonisolated static func markedX(at x: CGFloat, width: CGFloat, in viewWidth: CGFloat) -> CGFloat {
        max(0, min(x, viewWidth - width))
    }

    /// A commit: the caret stays where the composition ended until the echo
    /// moves the real cursor (V7), at most half a second.
    private func holdCaretAfterCommit(at anchor: CGRect) {
        let caret = UIView(frame: CGRect(x: anchor.maxX, y: anchor.minY,
                                         width: max(2, metrics.width / 4), height: metrics.height))
        caret.backgroundColor = UIColor(cgColor: coreCGColor(style.cursorColor))
        caret.isUserInteractionEnabled = false
        addSubview(caret)
        pendingCaret = caret
        canvasCursorHidden = true
        let cursor = rows.info.cursor
        pendingCaretFrom = (cursor.col, cursor.row)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self, weak caret] in
            guard let self, let caret, self.pendingCaret === caret else { return }
            self.dropPendingCaret()
        }
    }

    /// The echo came (the cursor moved), or the wait is over.
    func dropPendingCaret() {
        pendingCaret?.removeFromSuperview()
        pendingCaret = nil
        if markedLabel == nil { canvasCursorHidden = false }
    }
}
