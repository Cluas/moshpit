import MoshpitKit
import UIKit

// The seam between the app and the view that draws a terminal: the shared
// Rust core's (``CoreTerminalView``; SwiftTerm's adapter went with batch B).
// App code — the session's coordinator, the tmux controllers, the hub, the
// hosts, the debug probes — asks a ``TerminalSurface`` and hears from it
// through a ``TerminalSurfaceDelegate``; it never names the view's own types
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
        /// The coordinator's own reveal (``SwiftTerminalView/Coordinator/
        /// revealWhenShown()``): never a session's code, never forwarded.
        static let revealWhenShown: UInt32 = 0x4000_0001
    }
}

/// What app code asks of a terminal, whichever view draws it.
///
/// **Bytes** go in through ``feed(_:)`` in the order they came, never
/// dropped; ``feed(mark:)`` puts a mark among them, and the surface hands it
/// back (``TerminalSurfaceDelegate/surface(_:reached:)``) once every byte
/// fed before it is on screen and before any byte fed after it is: with the
/// frame that has those bytes (the core parses on a thread of its own).
@MainActor
protocol TerminalSurface: AnyObject {
    /// The view to host.
    var view: UIView { get }
    var surfaceDelegate: TerminalSurfaceDelegate? { get set }

    // MARK: Bytes

    /// The remote's bytes, after everything fed before.
    func feed(_ data: Data)
    /// A mark after the bytes fed so far (see the type's doc).
    func feed(mark: TerminalMark)
    /// A reconnect: the next bytes are not the dead stream's continuation
    /// (a half-read UTF-8 or escape sequence is forgotten, after the bytes
    /// fed so far). Screen, history and modes stay.
    func discardPendingInput()
    /// This terminal is a tmux pane's: a `clear` keeps in its history what
    /// tmux's own history keeps (TERMINAL-VIEW.md §5.9).
    func followTmuxClears()

    // MARK: The grid

    var font: UIFont { get }
    /// A new font: new cells, a new grid for the same bounds. Nothing when
    /// it is the one the surface has.
    func apply(font: UIFont)
    /// The theme and the user's cursor; only what changed is applied.
    func apply(look: TerminalLook)
    /// The theme and the cursor again, whatever was applied last: colours
    /// the remote set since (OSC 10/11) give way to the theme.
    func reassert(look: TerminalLook)
    /// One cell in points.
    var cellSize: CGSize { get }
    /// The grid the surface has now.
    var grid: TerminalGrid { get }
    /// The grid bounds of `size` hold (what the host reports before the
    /// view has taken them).
    func grid(fitting size: CGSize) -> TerminalGrid
    /// The host gave the view its real bounds.
    func laidOut()
    /// The viewport cell under `point` (the view's coordinates), clamped to
    /// the grid.
    func viewportCell(at point: CGPoint) -> TerminalCell
    /// Where the remote's cursor is, in the viewport.
    var cursor: TerminalCell { get }
    var modes: TerminalModes { get }

    // MARK: History

    /// Scrolled back into the history.
    var isReadingScrollback: Bool { get }
    /// Lines of history kept.
    var historyLines: Int { get }
    /// Whole lines of the local history (positive: older).
    func scrollLocal(lines: Int)
    /// The local history by `dy` points (positive: older), sub-row offsets
    /// kept; the distance moved.
    func scrollLocal(pixels dy: CGFloat) -> CGFloat
    /// A drag came to rest: within half a row of the live screen docks onto
    /// it (V19).
    func settleLocalScroll()
    /// A drag starts (sub-row travel not yet turned into rows is dropped).
    func dragWillBegin()
    /// A drag of `dy` points (positive: older) with no owner of its own
    /// (TERMINAL-VIEW.md V20): the host's rows (returned, when `hostRows`),
    /// wheel ticks for a program that wants the mouse (sent by the surface),
    /// or the local history. Returns the points used and the host's rows.
    func drag(_ dy: CGFloat, hostRows: Bool) -> (moved: CGFloat, rows: Int)
    /// Back on the live screen: output held while history was read goes on
    /// screen (the core never holds any).
    func releaseScrollHold()
    /// Drops what was held without showing it (a reconnect on a reused
    /// terminal: those bytes belong to a dead screen).
    func discardScrollHold()

    // MARK: The program's mouse and paste

    /// Wheel ticks at the grid's centre for a program that wants the mouse
    /// (positive: older), at most six.
    func sendWheel(lines: Int)
    /// A left click at a viewport cell, encoded as the program asked;
    /// nothing when it asked for no reports.
    func clickLocal(col: Int, row: Int)
    /// What a paste of `text` sends: bracketed as the program asked, any
    /// marker inside taken out (V30).
    func pastePayload(_ text: String) -> Data
    /// The answer to the remote's OSC 52 read query (nil: an empty
    /// clipboard, the conventional refusal).
    func answerClipboard(_ text: String?)

    // MARK: Text

    /// The text of viewport row `row`, trailing blanks trimmed.
    func rowText(_ row: Int) -> String
    /// Paths the host confirmed, tagged as links (CoreTerminalView+Paths).
    var pathLinks: RemotePathLinks? { get set }

    // MARK: Pictures

    /// A still picture of the terminal as it shows, framed like the view.
    func snapshotView() -> UIView?
    /// Draws every row again on the canvas there (B445): the terminal
    /// stage asks it whenever it puts the terminal in a slot, so a picture
    /// lost on the way never waits for the next byte, or for the screen to
    /// be left and entered again, to come back.
    func redrawAll()
    /// Frames drawn since the terminal got its canvas, for the stage's
    /// check (a terminal on screen with text on it and nothing drawn); nil
    /// when the surface doesn't count them.
    var framesSinceCanvas: Int? { get }
    /// What lies outside the grid when it isn't the theme's background: the
    /// colour the grid shows at its edges (B495), which the page's chrome
    /// takes too (B496). nil: the theme's.
    var groundColor: UInt32? { get }

    // MARK: Find

    /// Its find bar shows: the bar's text field has the keyboard, and the
    /// terminal must not take it back (``TerminalFocusPolicy/take``).
    var isFinding: Bool { get }

    // MARK: mosh's predictive echo

    /// The session's engine (nil: none), whatever the previous one drew
    /// taken down.
    func attachPrediction(_ engine: MoshPrediction?)
    /// Keystrokes on their way to the server: their likely echo shows now.
    /// The batch for ``predictionSent(batch:userState:)``, nil when none.
    func predict(_ data: Data) -> Int?
    func predictionSent(batch: Int, userState: UInt64?)
    /// One applied mosh diff: its bytes, then its echo ack, judged after
    /// them.
    func feed(moshFrame frame: MoshHostFrame)
    /// Every guess down; the next keystroke starts a new epoch.
    func dropPredictions()
    /// A guess is waiting on the clock (shown past 250 ms, marked past 5 s).
    var predictionNeedsTick: Bool { get }
    /// The clock's turn; false: stop ticking (nothing to show now).
    func predictionTick() -> Bool
}

/// What a surface tells its owner (``SwiftTerminalView/Coordinator``): the
/// bytes it sends, its grid, what the remote asked for, the marks it
/// reached, and the gestures whose meaning depends on the session (wheel or
/// copy-mode or local history; clicks; pane switches).
@MainActor
protocol TerminalSurfaceDelegate: AnyObject {
    /// Keys, a paste, the terminal's replies: in order, one call each.
    func surface(_ surface: TerminalSurface, send data: Data)
    /// The grid changed (not reported for the grid the surface was born at).
    func surface(_ surface: TerminalSurface, sizeChanged cols: Int, rows: Int)
    func surface(_ surface: TerminalSurface, titleChanged title: String)
    /// OSC 7.
    func surface(_ surface: TerminalSurface, directoryChanged directory: String?)
    func surfaceBell(_ surface: TerminalSurface)
    func surface(_ surface: TerminalSurface, openLink link: String)
    /// The edit menu's Paste and ⌘V.
    func surface(_ surface: TerminalSurface, paste text: String)
    /// OSC 52: the remote asks for the clipboard
    /// (``TerminalSurface/answerClipboard(_:)``).
    func surfaceClipboardRequest(_ surface: TerminalSurface)
    /// A mark fed with ``TerminalSurface/feed(mark:)`` is due.
    func surface(_ surface: TerminalSurface, reached mark: TerminalMark)
    /// A drag's first move.
    func surfaceScrollWillBegin(_ surface: TerminalSurface)
    /// Finger travel in points, positive toward older output; the distance
    /// used.
    func surface(_ surface: TerminalSurface, scrollPixels dy: CGFloat) -> CGFloat
    func surfaceScrollDidEnd(_ surface: TerminalSurface)
    /// A tap at a viewport cell, for a program that wants the mouse.
    func surface(_ surface: TerminalSurface, click col: Int, row: Int)
    /// A drag may scroll even without local history (tmux, herdr, a mouse
    /// app).
    var surfaceScrollsElsewhere: Bool { get }
    /// A horizontal swipe switches panes or windows.
    var surfaceCanSwitch: Bool { get }
    func surface(_ surface: TerminalSurface, switchForward forward: Bool)
    /// A pinch ended at this font size.
    func surface(_ surface: TerminalSurface, fontSizeCommitted size: Double)
    /// What lies outside the grid changed (``TerminalSurface/groundColor``,
    /// B496).
    func surfaceGroundChanged(_ surface: TerminalSurface)
}

/// The quiet defaults: a test's sink (or any owner) implements what it
/// cares about.
extension TerminalSurfaceDelegate {
    func surfaceGroundChanged(_ surface: TerminalSurface) {}
    func surface(_ surface: TerminalSurface, sizeChanged cols: Int, rows: Int) {}
    func surface(_ surface: TerminalSurface, titleChanged title: String) {}
    func surface(_ surface: TerminalSurface, directoryChanged directory: String?) {}
    func surfaceBell(_ surface: TerminalSurface) {}
    func surface(_ surface: TerminalSurface, openLink link: String) {}
    func surface(_ surface: TerminalSurface, paste text: String) {}
    func surfaceClipboardRequest(_ surface: TerminalSurface) { surface.answerClipboard(nil) }
    func surface(_ surface: TerminalSurface, reached mark: TerminalMark) {}
    func surfaceScrollWillBegin(_ surface: TerminalSurface) {}
    func surface(_ surface: TerminalSurface, scrollPixels dy: CGFloat) -> CGFloat {
        surface.drag(dy, hostRows: false).moved
    }
    func surfaceScrollDidEnd(_ surface: TerminalSurface) { surface.settleLocalScroll() }
    func surface(_ surface: TerminalSurface, click col: Int, row: Int) { surface.clickLocal(col: col, row: row) }
    var surfaceScrollsElsewhere: Bool { false }
    var surfaceCanSwitch: Bool { false }
    func surface(_ surface: TerminalSurface, switchForward forward: Bool) {}
    func surface(_ surface: TerminalSurface, fontSizeCommitted size: Double) {}
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

    static func make(font: UIFont, grid: TerminalGrid) -> TerminalSurface {
        let cell = TerminalCellGeometry.measuredCell(font: font, scale: UITraitCollection.current.displayScale)
        let birth = CGRect(x: 0, y: 0, width: cell.width * CGFloat(max(1, grid.cols)),
                           height: cell.height * CGFloat(max(1, grid.rows)))
        let view = CoreTerminalView(frame: birth, font: font)
        Log.ui.info("core term: minted \(view.grid.cols, privacy: .public)×\(view.grid.rows, privacy: .public) on \(view.canvasKind, privacy: .public)")
        NotificationCenter.default.post(name: minted, object: view)
        return view
    }
}
