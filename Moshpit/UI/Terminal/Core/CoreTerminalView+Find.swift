import MoshpitFFI
import UIKit

// Find on the core's view (TERMINAL-VIEW.md V45): the system's find bar
// (`UIFindInteraction`) over the core's find (crates/term `Terminal::find`:
// the text as written, every match in history and on screen counted, the
// newest first, round past either end, the view scrolled to the match and
// the match drawn marked). It opens with ⌘F (⌘G and ⇧⌘G step) on a
// keyboard, and from the edit menu's Find (with a selection: that text).

/// One find bar's search: the core does the finding, this keeps what the
/// bar shows ("3 of 12").
@MainActor
final class CoreTermFindSession: UIFindSession {
    private weak var view: CoreTerminalView?
    private var text = ""
    private var flags: UInt8 = 0
    private var count = 0
    private var index = 0

    init(view: CoreTerminalView) {
        self.view = view
        super.init()
    }

    override var resultCount: Int { count }
    override var highlightedResultIndex: Int { index }
    override var supportsReplacement: Bool { false }

    override func performSearch(query: String, options: UITextSearchOptions?) {
        text = query
        flags = Self.flags(options)
        step(0)
    }

    override func highlightNextResult(in direction: UITextStorageDirection) {
        step(direction == .forward ? 1 : 2)
    }

    override func invalidateFoundResults() {
        text = ""
        count = 0
        index = 0
        view?.clearFind()
    }

    private func step(_ step: UInt8) {
        let found = view?.findText(text, flags: flags, step: step)
        count = found.map { Int($0.count) } ?? 0
        index = found.map { Int($0.index) } ?? 0
    }

    /// The bar's options as the core's flags: any case unless the bar says
    /// to match it; "starts with" and "whole words" as word edges.
    static func flags(_ options: UITextSearchOptions?) -> UInt8 {
        guard let options else { return 0 }
        var f: UInt8 = 0
        if !options.stringCompareOptions.contains(.caseInsensitive) { f |= UInt8(MP_FIND_CASE_SENSITIVE) }
        switch options.wordMatchMethod {
        case .startsWith: f |= UInt8(MP_FIND_WORD_START)
        case .fullWord: f |= UInt8(MP_FIND_WORD_START) | UInt8(MP_FIND_WORD_END)
        default: break
        }
        return f
    }
}

extension CoreTerminalView: UIFindInteractionDelegate {
    func findInteraction(_ interaction: UIFindInteraction, sessionFor view: UIView) -> UIFindSession? {
        CoreTermFindSession(view: self)
    }

    func findInteraction(_ interaction: UIFindInteraction, didEnd session: UIFindSession) {
        clearFind()
    }

    /// The find bar shows (its text field has the keyboard: the terminal
    /// must not take it back).
    var isFinding: Bool { finder?.isFindNavigatorVisible == true }

    /// The core's find step: 0 the match kept or the newest, 1 newer, 2
    /// older. nil: no match.
    func findText(_ text: String, flags: UInt8, step: UInt8) -> MpFound? {
        guard let term else { return nil }
        var out = MpFound()
        let found = text.withCString { mp_term_find(term, $0, flags, step, &out) }
        takeFrame()
        return found ? out : nil
    }

    func clearFind() {
        guard let term else { return }
        mp_term_search_clear(term)
        takeFrame()
    }

    /// Opens the find bar, `text` in it when given.
    func presentFind(_ text: String? = nil) {
        guard let finder else { return }
        if let text, !text.isEmpty {
            finder.searchText = text
        }
        finder.presentFindNavigator(showingReplace: false)
    }

    override func find(_ sender: Any?) { presentFind() }
    override func findNext(_ sender: Any?) { finder?.findNext() }
    override func findPrevious(_ sender: Any?) { finder?.findPrevious() }

    override func useSelectionForFind(_ sender: Any?) {
        let text = selectedText
        clearSelection()
        presentFind(text)
    }

    /// The edit menu's Find, unless the system put one in already.
    func withFind(_ suggested: [UIMenuElement]) -> UIMenu {
        func hasFind(_ elements: [UIMenuElement]) -> Bool {
            elements.contains { element in
                if let menu = element as? UIMenu { return hasFind(menu.children) }
                guard let command = element as? UICommand else { return false }
                return [#selector(UIResponder.find(_:)), #selector(UIResponder.useSelectionForFind(_:))]
                    .contains(command.action)
            }
        }
        guard !hasFind(suggested) else { return UIMenu(children: suggested) }
        let action = hasSelection ? #selector(UIResponder.useSelectionForFind(_:)) : #selector(UIResponder.find(_:))
        let find = UICommand(title: String(localized: "Find"), image: UIImage(systemName: "magnifyingglass"),
                             action: action)
        return UIMenu(children: suggested + [find])
    }
}
