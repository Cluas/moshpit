import MoshpitFFI
import UIKit

// VoiceOver on the core view: every visible row is an element of its own,
// read as its text, in order (blank rows have nothing to say and are
// passed over); the cursor's row says it holds the cursor. The view also
// answers UIAccessibilityReadingContent, so "read all" and line-by-line
// navigation walk the screen. What appeared is the core's to say (V33:
// the rows from where the cursor was, never the cursor's row), and when: a
// second of quiet after each announcement, what appears in it said at its
// end (B427: an agent streaming output would otherwise talk over
// everything, and the last of it must not go unsaid).

/// One row of the screen, for VoiceOver.
final class CoreTermRowElement: UIAccessibilityElement {
    var row = 0
}

extension CoreTerminalView: UIAccessibilityReadingContent {
    /// The text a row shows (wide characters once, trailing blanks off).
    func rowText(_ r: Int) -> String {
        guard r < rows.rows.count else { return "" }
        let row = rows.rows[r]
        var s = ""
        for cell in row.cells where !coreHas(cell, Int(MP_CELL_SPACER)) {
            s += cell.ch == 0 ? " " : rows.text(of: cell, in: row)
        }
        while s.last == " " { s.removeLast() }
        return s
    }

    override var isAccessibilityElement: Bool {
        get { false }
        set {}
    }

    override var accessibilityElements: [Any]? {
        get {
            if accessibilityStale || accessibilityRows.count != rows.gridRows { rebuildAccessibility() }
            return accessibilityRows
        }
        set {}
    }

    private func rebuildAccessibility() {
        accessibilityStale = false
        let cursorRow = rows.info.cursor.visible ? Int(rows.info.cursor.row) : -1
        var elements: [CoreTermRowElement] = []
        elements.reserveCapacity(rows.gridRows)
        let switching = paneSwitchActions()
        for r in 0..<rows.gridRows {
            let e = accessibilityRows.indices.contains(r) ? accessibilityRows[r] : CoreTermRowElement(accessibilityContainer: self)
            e.row = r
            let text = rowText(r)
            e.accessibilityLabel = text
            e.isAccessibilityElement = !text.isEmpty || r == cursorRow
            e.accessibilityTraits = [.staticText]
            e.accessibilityHint = r == cursorRow ? String(localized: "Cursor") : nil
            e.accessibilityCustomActions = switching
            e.accessibilityFrameInContainerSpace = CGRect(x: 0, y: y(ofRow: r), width: bounds.width,
                                                          height: metrics.height)
            elements.append(e)
        }
        accessibilityRows = elements
    }

    /// The screen changed: VoiceOver hears what appeared when the core says
    /// (B427), and is asked again when the core wants (frame or no frame).
    /// The rows' elements are refreshed at most once a second.
    func announceChanges() {
        guard UIAccessibility.isVoiceOverRunning, let term else { return }
        let now = CACurrentMediaTime()
        if now - lastAccessibilityPost > 1 {
            lastAccessibilityPost = now
            UIAccessibility.post(notification: .layoutChanged, argument: nil)
        }
        var again: UInt32 = 0
        if let p = mp_term_a11y_announce(term, &again) {
            let text = String(cString: p)
            mp_string_free(p)
            UIAccessibility.post(notification: .announcement, argument: text)
        }
        guard again > 0, announceAgain == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                self?.announceAgain = nil
                self?.announceChanges()
            }
        }
        announceAgain = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Double(again) / 1000, execute: work)
    }

    // MARK: UIAccessibilityReadingContent

    func accessibilityLineNumber(for point: CGPoint) -> Int {
        cell(at: point).row
    }

    func accessibilityContent(forLineNumber lineNumber: Int) -> String? {
        guard lineNumber >= 0, lineNumber < rows.gridRows else { return nil }
        return rowText(lineNumber)
    }

    func accessibilityFrame(forLineNumber lineNumber: Int) -> CGRect {
        UIAccessibility.convertToScreenCoordinates(
            CGRect(x: 0, y: y(ofRow: lineNumber), width: bounds.width, height: metrics.height), in: self)
    }

    func accessibilityPageContent() -> String? {
        (0..<rows.gridRows).map(rowText).joined(separator: "\n")
    }
}
