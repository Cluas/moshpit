import Foundation
import MoshpitFFI
import Testing
import UIKit
@testable import Moshpit

/// T3: the app's seam (``CoreTerminalView``) on the core's view, and tmux
/// panes on it — the pane-switch marks (veil, reveal) act when the bytes
/// before them are on screen, not when they are pushed; a held resize takes
/// effect before the capture after it.
@Suite("Core view: the seam and tmux panes", .serialized)
@MainActor
struct CorePaneTests {
    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private static let look = RustTmuxController.Appearance(
        theme: .dracula, fontSize: 12, fontName: "system", cursorShape: .block,
        cursorColorId: "teal", cursorBlink: false)

    /// A Rust session's tmux surface on a fake core (nothing connects).
    private func tmux() -> (RustTmuxController, RustSessionDriver, FakeCoreSessions) {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        var connection = ServerConnection(host: "h", username: "u")
        connection.multiplexer = .tmux
        let session = SessionHub.ActiveSession(connection: connection)
        let driver = RustSessionDriver(session: session, core: core, appearance: Self.look)
        let controller = RustTmuxController(driver: driver, appearance: Self.look, grid: (40, 12))
        controller.mirror(RustSessionTests.model(active: "%1"))
        return (controller, driver, fake)
    }

    /// Waits until `done` holds, taking the core's frames meanwhile.
    @discardableResult
    private func settle(_ surface: CoreTerminalView, timeout: TimeInterval = 5,
                        until done: @escaping () -> Bool) -> Bool {
        if let core = surface as? CoreTerminalView { return core.settle(timeout: timeout) { _ in done() } }
        return done()
    }

    private func mark(_ code: UInt32, _ arg: Int64 = 0) -> MpStreamMark {
        var m = MpStreamMark()
        m.code = code
        m.arg = arg
        return m
    }

    // MARK: tmux panes

    @Test("a Rust tmux pane's terminal is the core's view, one per pane")
    func paneViewIsTheCores() {
        #expect(mp_term_abi_version() == UInt32(MP_TERM_ABI_VERSION), "the header and the linked core agree")
        let (tmux, driver, _) = tmux()
        let surface = tmux.surface(for: "%1")
        #expect(surface is CoreTerminalView)
        #expect(surface === tmux.surface(for: "%1"), "one terminal per pane, for its life")
        let coordinator = tmux.coordinator(for: "%1")
        #expect(surface.surfaceDelegate === coordinator, "the pane's coordinator hears its terminal")
        #expect(surface.grid == TerminalGrid(cols: 40, rows: 12), "born at the client's grid")
        _ = driver
    }

    @Test("a pane switch's reveal waits for the frame with the repaint before it")
    func revealWaitsForTheRepaint() throws {
        let (tmux, driver, _) = tmux()
        let surface = try #require(tmux.surface(for: "%1") as? CoreTerminalView)
        let coordinator = try #require(tmux.coordinator(for: "%1"))
        coordinator.feed(data: Data("stale".utf8))
        settle(surface) { surface.rowText(0) == "stale" }

        tmux.read(mark(RustTmuxController.Mark.veil), for: "%1")
        #expect(settle(surface) { coordinator.isCoverPresented }, "the veil never went up")

        var shownAtReveal: String?
        coordinator.onReveal = { shownAtReveal = surface.rowText(surface.grid.rows - 1) }
        // The repaint, then the reveal: the reveal must not lift the veil
        // before the repaint is drawn, however long the parse takes.
        let repaint = "\u{1b}[H\u{1b}[2J" + "fresh" + String(repeating: "\r\n.", count: 500) + "\r\nEND"
        coordinator.feed(data: Data(repaint.utf8))
        tmux.read(mark(RustTmuxController.Mark.reveal), for: "%1")
        #expect(coordinator.isCoverPresented, "revealed when pushed, before the core parsed the repaint")
        #expect(settle(surface) { shownAtReveal != nil }, "the reveal never came")
        #expect(shownAtReveal == "END", "revealed over a frame without the whole repaint: \(shownAtReveal ?? "-")")
        _ = driver
    }

    @Test("a held resize takes effect before the capture that follows it")
    func heldResizeLandsBeforeTheCapture() throws {
        let (tmux, driver, _) = tmux()
        let surface = try #require(tmux.surface(for: "%1") as? CoreTerminalView)
        let coordinator = try #require(tmux.coordinator(for: "%1"))
        let container = TerminalHostContainer(frame: surface.frame)
        container.host(surface)
        coordinator.hostContainer = container
        // The keyboard came up: the controller holds the resize until tmux's
        // capture at the new size comes back.
        coordinator.shouldDeferResize = { _, _ in true }
        let cell = surface.cellSize
        container.frame = CGRect(x: 0, y: 0, width: cell.width * 10, height: cell.height * 12)
        container.layoutSubviews()
        #expect(surface.grid.cols == 40, "the held size was applied early")

        // Bytes drawn for 40 columns, the mark, the capture drawn for 10: its
        // `R` at the right edge lands in column 10 only if the core took the
        // new width before parsing it (at 40 it would land in column 40, and
        // the shrink would rewrap it four rows down).
        coordinator.feed(data: Data(("\u{1b}[H\u{1b}[2J" + String(repeating: "x", count: 30)).utf8))
        tmux.read(mark(RustTmuxController.Mark.applyDeferredResize), for: "%1")
        #expect(surface.grid.cols == 10)
        coordinator.feed(data: Data("\u{1b}[H\u{1b}[2J\u{1b}[999CR".utf8))
        #expect(settle(surface) { surface.rowText(0).hasSuffix("R") },
                "the capture was parsed at the old width: \((0..<5).map(surface.rowText))")
        #expect(surface.rowText(0) == String(repeating: " ", count: 9) + "R")
        _ = driver
    }

    // MARK: Marks among the bytes

    @Test("marks come back in order, each with the frame that has the bytes before it")
    func marksRideTheirFrames() throws {
        let surface = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 20, rows: 3))
        let core = try #require(surface as? CoreTerminalView)
        final class Sink: CoreTerminalViewDelegate {
            var seen: [(arg: Int64, row: String)] = []
            weak var core: CoreTerminalView?
            func surface(_ surface: CoreTerminalView, send data: Data) {}
            func surface(_ surface: CoreTerminalView, reached mark: TerminalMark) {
                seen.append((mark.arg, core?.rowText(0) ?? ""))
            }
        }
        let sink = Sink()
        sink.core = core
        surface.surfaceDelegate = sink
        for n in 1...200 {
            surface.feed(Data("\r\u{1b}[KL\(n)".utf8))
            surface.feed(mark: TerminalMark(code: 4, arg: Int64(n)))
        }
        #expect(settle(surface) { sink.seen.count == 200 }, "\(sink.seen.count) of 200 marks")
        #expect(sink.seen.map(\.arg) == Array(1...200).map(Int64.init), "out of order or lost")
        for (arg, row) in sink.seen {
            let shown = Int64(row.dropFirst()) ?? 0
            #expect(shown >= arg, "mark \(arg) came with a frame showing \(row)")
        }
    }

    // MARK: The coordinator

    @Test("output that came before the terminal reaches it, in order")
    func earlyOutputIsNotLost() {
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.feed(data: Data("ear".utf8))
        coordinator.feed(data: Data("ly".utf8))
        #expect(coordinator.unattachedFeed.count == 2)
        let surface = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 20, rows: 3))
        coordinator.attach(surface)
        #expect(coordinator.unattachedFeed.isEmpty)
        #expect(settle(surface) { surface.rowText(0) == "early" }, "got \(surface.rowText(0))")
    }

    @Test("a predictor set before the terminal is made is the terminal's")
    func predictorReachesALateTerminal() throws {
        let coordinator = CoreTerminalHost.Coordinator()
        let engine = MoshPrediction(mode: .always, trail: false)
        coordinator.attachPrediction(engine)
        let surface = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 20, rows: 3))
        coordinator.attach(surface)
        let core = try #require(surface as? CoreTerminalView)
        #expect(core.prediction === engine)
    }

    @Test("modes, the grid, the cursor and a paste read through the seam")
    func seamReads() {
        let surface = TerminalSurfaces.make(font: Self.font, grid: TerminalGrid(cols: 30, rows: 5))
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.attach(surface)
        var sent: [Data] = []
        coordinator.onInput = { sent.append($0) }
        #expect(surface.grid == TerminalGrid(cols: 30, rows: 5))
        coordinator.feed(data: Data("\u{1b}[?2004h\u{1b}[?1h\u{1b}[?1000h\u{1b}[3;5Hhi".utf8))
        #expect(settle(surface) { surface.rowText(2) == "    hi" })
        #expect(coordinator.modes == TerminalModes(applicationCursorKeys: true, bracketedPaste: true,
                                                   mouseReporting: true))
        #expect(surface.cursor == TerminalCell(col: 6, row: 2))
        coordinator.paste("a\nb\u{1b}[201~c")
        #expect(sent == [Data("\u{1b}[200~a\nbc\u{1b}[201~".utf8)], "bracketed, the marker inside taken out")
        let cell = surface.cellSize
        #expect(surface.viewportCell(at: CGPoint(x: cell.width * 3.5, y: cell.height * 1.5))
                == TerminalCell(col: 3, row: 1))
        #expect(surface.grid(fitting: CGSize(width: cell.width * 12.5, height: cell.height * 4.2))
                == TerminalGrid(cols: 12, rows: 4))
    }

    // MARK: Gone panes (B736)

    /// The core's list of panes with a channel: `ids`, the first active.
    private func list(_ ids: [String]) -> [PaneValue] {
        ids.enumerated().map { n, id in PaneValue(pane: id, stream: UInt64(90_000 + n), active: n == 0) }
    }

    @Test("a pane tmux reports gone lets its terminal go; the live ones keep theirs (B736)")
    func gonePanesLetTheirTerminalsGo() {
        let (tmux, driver, _) = tmux()
        // Four panes in two windows, every one shown once.
        tmux.updatePanes(list(["%1", "%2", "%3", "%4"]))
        for id in ["%1", "%2", "%3", "%4"] { _ = tmux.surface(for: id) }
        let first = tmux.surface(for: "%1")
        #expect(tmux.paneTerminalCount == 4)
        // The second window closed: the core lists the first's panes. Every
        // pane once shown kept its terminal, its history with it, for the
        // attach's life.
        tmux.updatePanes(list(["%1", "%2"]))
        #expect(tmux.paneTerminalCount == 2, "the closed window's terminals stayed")
        #expect(tmux.existingSurface(for: "%3") == nil)
        #expect(tmux.surface(for: "%1") === first, "a live pane keeps its terminal")
        // A pane hidden, zoomed or in another window is still listed: kept.
        tmux.updatePanes(list(["%2", "%1"]))
        #expect(tmux.surface(for: "%1") === first)
        #expect(tmux.paneTerminalCount == 2)
        _ = driver
    }

    @Test("a pane a landing waits on keeps its terminal until the landing ends (B736, B468)")
    func aLandingHoldsItsPane() {
        let (tmux, driver, _) = tmux()
        tmux.updatePanes(list(["%1", "%2"]))
        let held = tmux.surface(for: "%2")
        tmux.holdPane("%2")
        // %2 closed while its notification's landing waits.
        tmux.updatePanes(list(["%1"]))
        #expect(tmux.existingSurface(for: "%2") === held, "let go while the landing waited")
        tmux.holdPane(nil)
        #expect(tmux.existingSurface(for: "%2") == nil, "kept once the landing ended")
        #expect(tmux.paneTerminalCount == 1)
        _ = driver
    }

    @Test("a terminal made before its pane's channel is listed stays for it (B736)")
    func aPaneOnItsWayKeepsItsTerminal() {
        let (tmux, driver, _) = tmux()
        tmux.updatePanes(list(["%1"]))
        // The stage shows %5 from the tree before the core lists its channel.
        let early = tmux.surface(for: "%5")
        #expect(tmux.existingSurface(for: "%5") === early)
        tmux.updatePanes(list(["%1", "%5"]))
        #expect(tmux.surface(for: "%5") === early, "the pane on its way lost its terminal")
        // One asked for after the latest list but never listed after it:
        // a pane already gone, let go with the next list.
        _ = tmux.surface(for: "%7")
        tmux.updatePanes(list(["%5", "%1"]))
        #expect(tmux.existingSurface(for: "%7") == nil)
        #expect(tmux.paneTerminalCount == 2)
        _ = driver
    }
}
