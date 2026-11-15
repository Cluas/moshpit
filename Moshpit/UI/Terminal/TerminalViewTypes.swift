import MoshpitKit
import UIKit

// What the app and the view that draws a terminal (the shared Rust core's,
// ``CoreTerminalView``) say to each other: the values they pass, what the
// view tells its owner (``CoreTerminalViewDelegate``), and how terminals are
// made (``TerminalSurfaces``). The protocol that let SwiftTerm's view stand
// in went with E6a, its one conformer left being the core's
// (docs/nsh/TERMINAL-VIEW.md §1).

/// A viewport cell, 0-based, row 0 at the top of what shows.
struct TerminalCell: Equatable, Sendable {
    var col: Int
    var row: Int
}

/// A grid in cells.
struct TerminalGrid: Equatable, Sendable {
    var cols: Int
    var rows: Int
}

/// The modes the remote program set that the app routes input by.
struct TerminalModes: Equatable, Sendable {
    /// DECCKM (`ESC[?1h`): arrows send their application form.
    var applicationCursorKeys = false
    /// Mode 2004: a paste goes between markers.
    var bracketedPaste = false
    /// Mouse reporting (1000, 1002, 1003): taps and wheels are the program's.
    var mouseReporting = false
}

/// The user's look for a terminal (Settings): the theme and the cursor. The
/// user's cursor is authoritative over the remote's DECSCUSR and OSC 12.
struct TerminalLook: Equatable {
    var theme: TerminalTheme
    var cursorShape: CursorShape = .block
    var cursorColorId: String = "teal"
    var cursorBlink: Bool = true
    /// The app theme the cursor's colour resolves under
    /// (``TerminalCursor/color(forId:theme:)``: Accent is its accent): a new
    /// theme is a new look, so a terminal already open takes it.
    var appTheme: AppTheme = AppThemeCatalog.current
}

/// A mark among a terminal's bytes: what the view does at that point of the
/// output (crates/stream's marks: a session's code and its argument; the
/// codes are `RustTmuxController.Mark`'s).
struct TerminalMark: Equatable, Sendable {
    var code: UInt32
    var arg: Int64

    /// The view marks of crates/session's `view_marks` (tmux_leg.rs), and
    /// the coordinator's own.
    enum Code {
        static let veil: UInt32 = 1
        static let veilUnlessCovered: UInt32 = 2
        /// `arg`: milliseconds.
        static let extendCover: UInt32 = 3
        static let reveal: UInt32 = 4
        static let releaseScrollHold: UInt32 = 5
        /// `arg`: rows, positive = older.
        static let scrollLocal: UInt32 = 6
        static let settleLocalScroll: UInt32 = 7
        static let applyDeferredResize: UInt32 = 8
        /// mosh's echo acknowledgement (the shell's channel): `arg` is how
        /// many bytes of the input written into the channel the server has
        /// echoed.
        static let echoAck: UInt32 = 9
    }
}

/// What a surface tells its owner (``CoreTerminalHost/Coordinator``): the
/// bytes it sends, its grid, what the remote asked for, the marks it
/// reached, and the gestures whose meaning depends on the session (wheel or
/// copy-mode or local history; clicks; pane switches).
@MainActor
protocol CoreTerminalViewDelegate: AnyObject {
    /// Keys, a paste, the terminal's replies: in order, one call each.
    func surface(_ surface: CoreTerminalView, send data: Data)
    /// What the person typed (a key, a character, an input method's
    /// commit), apart from what the terminal sends of its own (B570: the
    /// bar's sticky Ctrl takes typing only).
    func surface(_ surface: CoreTerminalView, typed data: Data)
    /// What the terminal answers of its own: a query's reply (cursor
    /// position, a colour), a focus report. Apart from typing, so a view
    /// whose program the terminal does not emulate can drop them (B575).
    func surface(_ surface: CoreTerminalView, reply data: Data)
    /// The grid changed (not reported for the grid the surface was born at).
    func surface(_ surface: CoreTerminalView, sizeChanged cols: Int, rows: Int)
    func surface(_ surface: CoreTerminalView, titleChanged title: String)
    /// OSC 7.
    func surface(_ surface: CoreTerminalView, directoryChanged directory: String?)
    func surfaceBell(_ surface: CoreTerminalView)
    func surface(_ surface: CoreTerminalView, openLink link: String)
    /// The edit menu's Paste and ⌘V.
    func surface(_ surface: CoreTerminalView, paste text: String)
    /// OSC 52: the remote asks for the clipboard
    /// (``CoreTerminalView/answerClipboard(_:)``).
    func surfaceClipboardRequest(_ surface: CoreTerminalView)
    /// A mark fed with ``CoreTerminalView/feed(mark:)`` is due.
    func surface(_ surface: CoreTerminalView, reached mark: TerminalMark)
    /// A drag's first move.
    func surfaceScrollWillBegin(_ surface: CoreTerminalView)
    /// Finger travel in points, positive toward older output; the distance
    /// used.
    func surface(_ surface: CoreTerminalView, scrollPixels dy: CGFloat) -> CGFloat
    func surfaceScrollDidEnd(_ surface: CoreTerminalView)
    /// A tap at a viewport cell, for a program that wants the mouse.
    func surface(_ surface: CoreTerminalView, click col: Int, row: Int)
    /// A drag may scroll even without local history (tmux, herdr, a mouse
    /// app).
    var surfaceScrollsElsewhere: Bool { get }
    /// A drag's rows go to the host's own scrollback (herdr, mosh's tmux),
    /// so a coast goes there paced (B523). A tmux pane's own scrollback is
    /// this terminal's, by the pixel.
    var surfaceScrollsHostRows: Bool { get }
    /// A horizontal swipe switches panes or windows.
    var surfaceCanSwitch: Bool { get }
    /// The shortcut bar's Ctrl is armed: the next character typed is a
    /// control chord (B570), even one an input method would compose (B691).
    var surfaceCtrlArmed: Bool { get }
    func surface(_ surface: CoreTerminalView, switchForward forward: Bool)
    /// The sideways drag is the host's to move as it goes
    /// (``TerminalSwipe``), not a switch on the lift.
    var surfaceTracksSwipe: Bool { get }
    func surface(_ surface: CoreTerminalView, swipe: TerminalSwipe)
    /// The host shows every pane of the session (VoiceOver's Show all
    /// panes: the NSH page's overview).
    var surfaceCanShowAll: Bool { get }
    func surfaceShowAll(_ surface: CoreTerminalView)
    /// A pinch ended at this font size.
    func surface(_ surface: CoreTerminalView, fontSizeCommitted size: Double)
    /// What lies outside the grid changed (``CoreTerminalView/groundColor``,
    /// B496).
    func surfaceGroundChanged(_ surface: CoreTerminalView)
}

/// The quiet defaults: a test's sink (or any owner) implements what it
/// cares about.
extension CoreTerminalViewDelegate {
    func surface(_ surface: CoreTerminalView, typed data: Data) { self.surface(surface, send: data) }
    func surface(_ surface: CoreTerminalView, reply data: Data) { self.surface(surface, send: data) }
    func surfaceGroundChanged(_ surface: CoreTerminalView) {}
    func surface(_ surface: CoreTerminalView, sizeChanged cols: Int, rows: Int) {}
    func surface(_ surface: CoreTerminalView, titleChanged title: String) {}
    func surface(_ surface: CoreTerminalView, directoryChanged directory: String?) {}
    func surfaceBell(_ surface: CoreTerminalView) {}
    func surface(_ surface: CoreTerminalView, openLink link: String) {}
    func surface(_ surface: CoreTerminalView, paste text: String) {}
    func surfaceClipboardRequest(_ surface: CoreTerminalView) { surface.answerClipboard(nil) }
    func surface(_ surface: CoreTerminalView, reached mark: TerminalMark) {}
    func surfaceScrollWillBegin(_ surface: CoreTerminalView) {}
    var surfaceScrollsHostRows: Bool { false }
    func surface(_ surface: CoreTerminalView, scrollPixels dy: CGFloat) -> CGFloat {
        surface.drag(dy, hostRows: false).moved
    }
    func surfaceScrollDidEnd(_ surface: CoreTerminalView) { surface.settleLocalScroll() }
    func surface(_ surface: CoreTerminalView, click col: Int, row: Int) { surface.clickLocal(col: col, row: row) }
    var surfaceScrollsElsewhere: Bool { false }
    var surfaceCanSwitch: Bool { false }
    var surfaceCtrlArmed: Bool { false }
    func surface(_ surface: CoreTerminalView, switchForward forward: Bool) {}
    var surfaceTracksSwipe: Bool { false }
    func surface(_ surface: CoreTerminalView, swipe: TerminalSwipe) {}
    var surfaceCanShowAll: Bool { false }
    func surfaceShowAll(_ surface: CoreTerminalView) {}
    func surface(_ surface: CoreTerminalView, fontSizeCommitted size: Double) {}
}

/// A sideways drag on the terminal, as its host moves it (the NSH stage:
/// the terminal follows the finger and uncovers the pane it would go to).
/// Distances are points along the x axis from where the drag started.
enum TerminalSwipe: Equatable {
    /// The drag locked sideways.
    case began
    case moved(CGFloat)
    /// The finger lifted, moving at `velocity` points a second.
    case ended(CGFloat, velocity: CGFloat)
    /// The system took the touch.
    case cancelled
}

/// Makes terminals: born at a grid (what the remote is told before the
/// first layout, so bytes that land before it are parsed in the shape they
/// were drawn for), configured as every terminal of the app is.
@MainActor
enum TerminalSurfaces {
    /// Posted (on the main thread) for every terminal minted, before its
    /// owner has it (B445): the terminal stage looks again on the next turn
    /// whether the terminal it shows is still the one its target names, so
    /// a new one is in the slot as soon as it exists, not when the screen is
    /// next entered.
    static let minted = Notification.Name("moshpit.terminal-surface-minted")

    static func make(font: UIFont, grid: TerminalGrid) -> CoreTerminalView {
        let cell = TerminalCellGeometry.measuredCell(font: font, scale: UITraitCollection.current.displayScale)
        let birth = CGRect(x: 0, y: 0, width: cell.width * CGFloat(max(1, grid.cols)),
                           height: cell.height * CGFloat(max(1, grid.rows)))
        let view = CoreTerminalView(frame: birth, font: font)
        Log.ui.info("core term: minted \(view.grid.cols, privacy: .public)×\(view.grid.rows, privacy: .public) on \(view.canvasKind, privacy: .public)")
        NotificationCenter.default.post(name: minted, object: view)
        return view
    }
}
