import Foundation
import MoshpitFFI
import Testing
import UIKit
@testable import Moshpit

/// T3: find on the core's view (TERMINAL-VIEW.md V45): the system's find bar
/// over the core's find — counts, the newest first, round past either end,
/// the view scrolled to the match and the match marked; the bar's own
/// session cleared when it closes; the terminal never takes the keyboard back
/// from the bar.
@Suite("Core view: find", .serialized)
@MainActor
struct CoreFindTests {
    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private func makeCore() -> (CoreTerminalView, UIWindow) {
        let core = TerminalSurfaces.make(.core, font: Self.font, grid: TerminalGrid(cols: 30, rows: 6)) as! CoreTerminalView
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow()
        window.frame = CGRect(x: 0, y: 0, width: 600, height: 800)
        let host = UIViewController()
        window.rootViewController = host
        host.view.addSubview(core)
        window.makeKeyAndVisible()
        var text = ""
        for i in 0..<40 { text += String(format: "%02d %@\r\n", i, i % 10 == 3 ? "Needle" : "hay") }
        core.push(Data((text + "$ ").utf8))
        #expect(core.settle { $0.rowText(5) == "$" })
        return (core, window)
    }

    private func marked(_ core: CoreTerminalView) -> Bool {
        core.rows.rows.contains { $0.cells.contains { Int($0.flags) & Int(MP_CELL_MATCH) != 0 } }
    }

    @Test("the find bar's session counts, starts at the newest and goes round")
    func session() {
        let (core, window) = makeCore()
        defer { window.isHidden = true }
        let session = CoreTermFindSession(view: core)
        session.performSearch(query: "needle", options: nil)
        #expect(session.resultCount == 4)
        #expect(session.highlightedResultIndex == 3, "the newest")
        #expect(core.isReadingScrollback, "scrolled to it")
        #expect(marked(core))
        session.highlightNextResult(in: .forward)
        #expect(session.highlightedResultIndex == 0, "round past the newest")
        session.highlightNextResult(in: .backward)
        #expect(session.highlightedResultIndex == 3)
        session.performSearch(query: "nothing", options: nil)
        #expect(session.resultCount == 0)
        #expect(!marked(core))
        session.performSearch(query: "needle", options: nil)
        session.invalidateFoundResults()
        #expect(session.resultCount == 0)
        #expect(!marked(core), "cleared with the session")
    }

    @Test("⌘F, the edit menu's Find and Find Selection are the view's")
    func entries() {
        let (core, window) = makeCore()
        defer { window.isHidden = true }
        #expect(core.canPerformAction(#selector(UIResponder.find(_:)), withSender: nil))
        #expect(core.canPerformAction(#selector(UIResponder.findNext(_:)), withSender: nil))
        #expect(!core.canPerformAction(#selector(UIResponder.useSelectionForFind(_:)), withSender: nil),
                "nothing selected")
        let menu = core.withFind([])
        let command = menu.children.first as? UICommand
        #expect(command?.action == #selector(UIResponder.find(_:)))
        #expect(command?.title == String(localized: "Find"))
        core.select(kind: 1, row: 0, col: 4)
        #expect(core.canPerformAction(#selector(UIResponder.useSelectionForFind(_:)), withSender: nil))
        #expect((core.withFind([]).children.first as? UICommand)?.action
                == #selector(UIResponder.useSelectionForFind(_:)))
        let suggested = core.withFind([UICommand(title: "x", action: #selector(UIResponder.find(_:)))])
        #expect(suggested.children.count == 1, "the system's own Find is not doubled")
    }

    @Test("with the bar up, the terminal's focus rule leaves the keyboard to it")
    func focusLeftToTheBar() async {
        let (core, window) = makeCore()
        defer { window.isHidden = true }
        _ = core.becomeFirstResponder()
        core.find(nil)
        let deadline = Date().addingTimeInterval(3)
        while !core.isFinding, Date() < deadline { try? await Task.sleep(for: .milliseconds(20)) }
        #expect(core.isFinding, "⌘F's action opens the bar")
        #expect(!core.isFirstResponder, "the bar's field has the keyboard")
        SwiftTerminalView.applyFocus(.take, to: core)
        try? await Task.sleep(for: .milliseconds(100))
        #expect(!core.isFirstResponder, "taken back from the bar")
        core.finder?.dismissFindNavigator()
    }

    @Test("without the bar's options: any case, anywhere in a word")
    func flags() {
        #expect(CoreTermFindSession.flags(nil) == 0)
    }
}
