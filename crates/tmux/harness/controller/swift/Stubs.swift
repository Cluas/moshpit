// Harness-only stand-ins for the UIKit / terminal-view collaborators of
// Moshpit/Services/Tmux/TmuxSessionController.swift (the TerminalSurface seam
// of Moshpit/UI/Terminal/TerminalSurface.swift), so the controller
// compiles unchanged (but for the substitutions generate.py lists) for a
// macOS command-line build. Never part of the app.
//
// The coordinator stub records what the controller asks of a pane's view —
// feeds, veils, reveals, scroll holds, held resizes — in the harness log, in
// the order asked; it draws nothing. `isCoverPresented` is logged as a
// question ("cover?") and answered "no": the Rust controller leaves that
// decision to the view (`ViewAction::VeilUnlessCovered`), so the logs compare
// the asking, not a modelled cover.

import CoreGraphics
import Foundation

struct HarnessFont {}

final class HarnessTerminal {
    var cols: Int
    var rows: Int
    init(cols: Int, rows: Int) {
        self.cols = cols
        self.rows = rows
    }
}

/// A grid in cells (the app's, verbatim).
struct TerminalGrid: Equatable, Sendable {
    var cols: Int
    var rows: Int
}

/// The app's `TerminalSurface`, whichever view draws it: only its grid
/// matters here (the wheel's position). Born at the grid the controller
/// asks for, as `TerminalSurfaces.make` makes it.
final class TerminalSurface {
    let terminal: HarnessTerminal

    init(grid: TerminalGrid) {
        terminal = HarnessTerminal(cols: max(1, grid.cols), rows: max(1, grid.rows))
    }

    var grid: TerminalGrid { TerminalGrid(cols: terminal.cols, rows: terminal.rows) }
    func apply(font: HarnessFont) {}
    func reassert(look: TerminalLook) {}
    /// A tmux pane's `clear` keeps what tmux keeps (W23, e720b48d): a view
    /// setting, nothing the controller's log shows (the Rust controller's
    /// host sets it outside the controller too).
    func followTmuxClears() {}
}

/// Which view a new terminal gets: irrelevant here.
enum TerminalSurfaceKind {
    case core, swiftTerm
    static var current: TerminalSurfaceKind { .swiftTerm }
}

enum TerminalSurfaces {
    static func make(_ kind: TerminalSurfaceKind = .current, font: HarnessFont,
                     grid: TerminalGrid) -> TerminalSurface {
        TerminalSurface(grid: grid)
    }
}

enum TerminalFont {
    static func font(id: String, size: CGFloat) -> HarnessFont { HarnessFont() }
}

struct TerminalTheme: Equatable {
    static let dracula = TerminalTheme()
}

enum CursorShape { case block }

/// The user's look (the app's, verbatim).
struct TerminalLook: Equatable {
    var theme: TerminalTheme
    var cursorShape: CursorShape = .block
    var cursorColorId: String = "teal"
    var cursorBlink: Bool = true
}

struct UIDevice {
    enum Idiom { case phone, pad }
    static let current = UIDevice()
    let userInterfaceIdiom: Idiom = .phone
}

/// What `pathLinksFactory` hands each new pane terminal: here only its pane
/// id, which is how the coordinator stub learns which pane it belongs to.
final class RemotePathLinks {
    let paneId: String
    init(paneId: String) { self.paneId = paneId }
}

enum Multiplexer: String {
    case none, tmux, herdr
}

/// Stands in for the type that owns `Coordinator` in the app.
enum SwiftTerminalView {
    @MainActor
    final class Coordinator {
        private(set) var pane = "?"
        weak var terminal: TerminalSurface?
        /// A grid the view is holding back for its repaint.
        var heldGrid: (cols: Int, rows: Int)?

        var pathLinks: RemotePathLinks? {
            didSet {
                if let links = pathLinks, pane == "?" {
                    pane = links.paneId
                    HarnessLog.item("open \(pane)")
                }
            }
        }
        var onReveal: (() -> Void)?
        var onInput: ((Data) -> Void)?
        var onSizeChange: ((_ cols: Int, _ rows: Int) -> Void)?
        var onOpenRemotePath: ((String) -> Void)?
        var onBell: (() -> Void)?
        var onScroll: ((Int) -> Void)?
        var onScrollPixels: ((CGFloat) -> CGFloat)?
        var onScrollEnd: (() -> Void)?
        var onScrollBegin: (() -> Void)?
        var onSwitch: ((_ forward: Bool) -> Void)?
        var onClick: ((_ col: Int, _ row: Int) -> Void)?
        var onInputModeChange: (() -> Void)?
        var onGridReport: ((_ cols: Int, _ rows: Int) -> Void)?
        var shouldDeferResize: ((_ cols: Int, _ rows: Int) -> Bool)?
        var onFontSizeCommit: ((Double) -> Void)?
        var onReturnSend: (() -> Void)?

        nonisolated init() {}

        deinit {
            let pane = self.pane
            MainActor.assumeIsolated { HarnessLog.item("close \(pane)") }
        }

        func attach(_ surface: TerminalSurface) { terminal = surface }

        var cellHeight: CGFloat { HarnessView.cellHeight }

        var isCoverPresented: Bool {
            HarnessLog.item("view \(pane) cover?")
            return false
        }

        func feed(data: Data) { HarnessLog.item("pane \(pane) \(harnessEscape(data))") }
        func veilForSwitch() { HarnessLog.item("view \(pane) veil") }
        func extendCoverTimeout(by seconds: TimeInterval) {
            HarnessLog.item("view \(pane) extend \(String(format: "%.3f", seconds))")
        }
        func reveal() {
            HarnessLog.item("view \(pane) reveal")
            onReveal?()
        }
        /// The app reveals once the bytes fed before are on screen (a mark
        /// after them); here every feed is on screen when it is made.
        func revealWhenShown() { reveal() }
        func releaseScrollHold() { HarnessLog.item("view \(pane) release-hold") }
        func scrollLocal(lines: Int) { HarnessLog.item("view \(pane) scroll-local \(lines)") }
        func scrollLocal(pixels dy: CGFloat) -> CGFloat {
            HarnessLog.item("view \(pane) scroll-local-px \(String(format: "%.3f", Double(dy)))")
            return dy
        }
        func settleLocalScroll() { HarnessLog.item("view \(pane) settle") }
        func applyDeferredResize() {
            HarnessLog.item("view \(pane) apply-resize")
            if let grid = heldGrid, let terminal {
                terminal.terminal.cols = grid.cols
                terminal.terminal.rows = grid.rows
            }
            heldGrid = nil
        }
    }
}
