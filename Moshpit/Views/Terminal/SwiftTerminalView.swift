import SwiftUI
import UIKit
import SwiftTerm

/// What a hosted terminal should do about first-responder status.
///
/// Three states rather than a Bool because "should hold the keyboard" and "may
/// hold the keyboard" are different questions, and answering both with one flag
/// made tapping the terminal do nothing: `false` didn't merely decline to
/// focus, it *resigned* on every `updateUIView`, so a tap that focused the view
/// was undone by the very next repaint.
enum TerminalFocusPolicy {
    /// Take first responder now, and take it back if something steals it.
    case take
    /// Neither take nor surrender. A tap on the terminal focuses it, and
    /// nothing drags it back down — this is what "the keyboard hasn't been
    /// asked for yet" has to mean.
    case allow
    /// Give it up: a sheet is covering the screen, or the user put the
    /// keyboard away on purpose.
    case resign
}

/// Hosts a ``TerminalView`` and owns the ONLY code allowed to set its frame.
///
/// ### Why the terminal's frame is managed manually
///
/// During a keyboard show/hide/height change, SwiftUI re-lays-out the host on
/// every notification — and it runs BEFORE any observer this file could
/// register, so by the time the coordinator hears about the keyboard the
/// terminal has already been squeezed through one or more *partial* sizes.
/// Each partial size makes SwiftTerm resize + reflow its buffer and paint a
/// garbled intermediate frame (rows duplicated / overlapped — the classic
/// "键盘一动画面就乱" report, and what an IME switch triggers when the CN/EN
/// keyboards differ in height). A snapshot cover can't fix this: taken in the
/// notification handler it *captures the garble* (see git history).
///
/// So instead of masking the garble, prevent it: while a keyboard transition
/// is in flight (``SwiftTerminalView/Coordinator/lockFrame(for:)``) the
/// terminal keeps its pre-transition frame — content stays pixel-stable,
/// merely clipped by this container — and when the keyboard settles the
/// terminal is resized ONCE to the final bounds. One resize, one reflow, one
/// remote SIGWINCH (instead of a resize storm), no visible intermediate state.
final class TerminalHostContainer: UIView {
    /// The hosted terminal. Weak: SwiftTerm views are persistent objects owned
    /// by the session (tmux panes outlive any single host container).
    private(set) weak var terminalView: TerminalView?

    /// While true, ``layoutSubviews`` leaves the terminal's frame alone.
    /// Toggled by the coordinator around keyboard transitions.
    var frameLocked = false {
        didSet {
            if !frameLocked { setNeedsLayout() }
        }
    }

    func host(_ terminal: TerminalView) {
        guard terminal.superview !== self else { return }
        terminalView = terminal
        addSubview(terminal)
        terminal.frame = bounds
        clipsToBounds = true
    }

    /// Called once the hosted terminal has been given this container's real
    /// bounds. This is the app's OWN layout talking, which is the only reliable
    /// answer to "how big is the grid actually": SwiftTerm reports a size only
    /// when the grid *changes* (`processSizeChange` returns early otherwise), so
    /// a terminal that was minted at the right grid never reports at all — and
    /// the owner is left believing whatever it estimated before connecting.
    /// Installed by the ``SwiftTerminalView/Coordinator`` that owns this
    /// container.
    var onTerminalLaidOut: ((TerminalView) -> Void)?

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let terminalView, !frameLocked else { return }
        if terminalView.frame != bounds {
            terminalView.frame = bounds
        }
        guard bounds.width > 0, bounds.height > 0 else { return }
        onTerminalLaidOut?(terminalView)
    }
}

/// SwiftUI wrapper around SwiftTerm's ``TerminalView``.
///
/// ### Persistence contract (DO NOT BREAK)
///
/// The wrapped ``TerminalView`` is **persistent**: SwiftUI may invoke
/// ``UIViewRepresentable/updateUIView(_:context:)`` many times across re-renders,
/// but the underlying ``TerminalView`` instance — including its scrollback
/// buffer, cursor position, and the data we have already fed into it — must
/// survive every re-render.
///
/// As a result, `updateUIView` is **only** allowed to mutate cosmetic state
/// (theme, font, alpha, etc.). It MUST NOT:
///
///   - call ``TerminalView/feed(text:)`` / ``TerminalView/feed(byteArray:)``,
///   - clear the buffer,
///   - swap or replace ``TerminalView/terminalDelegate``.
///
/// The single legitimate input path is via the ``Coordinator`` — a parent
/// view holds the coordinator instance (e.g. as ``@State``) and calls
/// ``Coordinator/feed(data:)`` whenever new output arrives from SSH or the
/// tmux control client.
struct SwiftTerminalView: UIViewRepresentable {
    /// Active theme. Re-applied on every `updateUIView` (cheap, idempotent).
    var theme: TerminalTheme

    /// Monospaced font point size. Re-applied on every `updateUIView`.
    var fontSize: Double

    /// Font family id from Settings ("system" / "Menlo" / …).
    var fontName: String = "system"

    /// Cursor appearance from Settings (shape / colour id / blink). Applied
    /// on create and whenever it changes.
    var cursorShape: CursorShape = .block
    var cursorColorId: String = "teal"
    var cursorBlink: Bool = true

    /// Caller-owned coordinator. Held as a property (not via
    /// `makeCoordinator`) so a parent SwiftUI view can keep a stable
    /// reference and route output through ``Coordinator/feed(data:)``.
    var coordinator: Coordinator

    /// What to do about the keyboard. See ``TerminalFocusPolicy``.
    var focusPolicy: TerminalFocusPolicy = .take

    func makeCoordinator() -> Coordinator {
        // SwiftUI calls this once per representable identity. Return the
        // caller-owned coordinator so SwiftUI's internal bookkeeping matches
        // the instance the parent view is holding onto.
        coordinator
    }

    func makeUIView(context: Context) -> TerminalHostContainer {
        let font = TerminalFont.font(id: fontName, size: CGFloat(fontSize))
        let terminalView = TerminalView(frame: .zero, font: font)
        terminalView.inputAccessoryView = nil   // the app renders its own shortcut bar
        TerminalKeyboard.enableComposingInput(on: terminalView)
        TerminalScrollback.enlarge(terminalView)
        // Only underline/open REAL hyperlinks the program declared via OSC-8.
        // SwiftTerm's default `.implicit` also runs a heuristic regex that
        // mis-underlines bare file/relative paths (src/foo, ./build, ~/x) and
        // can truncate real URLs at certain chars — so it's off.
        terminalView.linkReporting = .explicit
        terminalView.linkHighlightMode = .always         // OSC-8 hyperlinks open on a plain tap
        // We own scrolling (gestures + scroll thumb), so don't let SwiftTerm
        // report touches as mouse drags — they leak to the remote during a
        // scroll and desync mosh copy-mode (see mintTerminal).
        terminalView.allowMouseReporting = false

        // Wire delegate + back-reference so the coordinator can push data in
        // and the terminal can route user input back out. attach(to:) also
        // installs the one-finger scroll + pinch-zoom gesture.
        terminalView.terminalDelegate = coordinator
        coordinator.attach(to: terminalView)

        theme.apply(to: terminalView)
        coordinator.enforcedCursor = TerminalCursor.apply(
            shape: cursorShape, colorId: cursorColorId, blink: cursorBlink, to: terminalView)
        // Pop the system keyboard as soon as the terminal is on screen — the
        // app's shortcut bar rides above it via safeAreaInset.
        if focusPolicy == .take {
            DispatchQueue.main.async { [weak terminalView] in
                terminalView?.becomeFirstResponder()
            }
        }
        let container = TerminalHostContainer()
        container.host(terminalView)
        coordinator.hostContainer = container
        return container
    }

    func updateUIView(_ uiView: TerminalHostContainer, context: Context) {
        // === COSMETIC ONLY ===
        // No feed(), no buffer clear, no delegate swap. See type doc.
        guard let terminal = uiView.terminalView else { return }

        let targetSize = CGFloat(fontSize)
        let desired = TerminalFont.font(id: fontName, size: targetSize)
        if terminal.font.pointSize != targetSize || terminal.font.fontName != desired.fontName {
            terminal.font = desired
        }

        theme.apply(to: terminal)
        // Re-assert cursor after theme (theme also sets caretColor), and
        // refresh the coordinator's enforced copy for the post-feed guard.
        coordinator.enforcedCursor = TerminalCursor.apply(
            shape: cursorShape, colorId: cursorColorId, blink: cursorBlink, to: terminal)

        // Honor the keyboard-dismiss toggle.
        switch focusPolicy {
        case .take:
            if !terminal.isFirstResponder, terminal.window != nil {
                DispatchQueue.main.async { [weak terminal] in terminal?.becomeFirstResponder() }
            }
        case .allow:
            break
        case .resign:
            if terminal.isFirstResponder {
                DispatchQueue.main.async { [weak terminal] in terminal?.resignFirstResponder() }
            }
        }
    }

    // MARK: - Coordinator

    /// Bridges between the SwiftUI side (theme/font bindings) and the
    /// imperative SwiftTerm ``TerminalView``. Owns the data-input path
    /// (``feed(data:)`` / ``feed(text:)``) and forwards user input + size
    /// changes back to the caller via closures.
    ///
    /// This is a plain reference type — no Combine, no Observation — so a
    /// parent view can hold it via `@State` (reference identity is stable
    /// across re-renders) without dragging in property-wrapper machinery
    /// that the representable doesn't need.
    final class Coordinator: NSObject, TerminalViewDelegate {
        /// Weak so the coordinator never extends the terminal view's
        /// lifetime past the SwiftUI view tree.
        private(set) weak var terminalView: TerminalView?

        /// Output that arrived before the terminal view attached (e.g. mosh's
        /// first screen diffs land within ~50ms, before `makeUIView` runs).
        /// Flushed in order once `attach(to:)` wires up the view.
        private var pendingFeed: [Data] = []

        /// Output-hold for reading scrollback (see ``TerminalScrollGesture``).
        /// While the user is scrolled up, new output would otherwise snap the
        /// viewport to the bottom (SwiftTerm follows the tail on every line), so
        /// we buffer it here and replay in order when they return to the bottom
        /// or type. Capped so a long read can't grow memory without bound — past
        /// the cap we give up the hold and resume live output.
        private var scrollHeld = false
        private var heldFeed: [Data] = []
        private var heldBytes = 0
        private let heldByteLimit = 1_000_000

        /// The user's cursor style + colour (Settings), re-asserted after
        /// every feed. Remote output carries DECSCUSR / OSC 12 (vim mode
        /// switches, zsh plugins, coding agents) and SwiftTerm applies them
        /// to the caret just like ours — without enforcement the user's
        /// choice only survives until the next such sequence. Installed by
        /// whoever calls ``TerminalCursor.apply``; nil skips enforcement.
        var enforcedCursor: (style: CursorStyle, color: UIColor)?

        /// Re-assert ``enforcedCursor`` if the fed bytes changed the caret.
        /// Cheap when nothing changed (two comparisons), so it runs after
        /// every feed.
        private func enforceCursor(on terminalView: TerminalView) {
            guard let enforcedCursor else { return }
            let terminal = terminalView.getTerminal()
            if terminal.options.cursorStyle != enforcedCursor.style {
                terminal.setCursorStyle(enforcedCursor.style)
            }
            if terminalView.caretColor != enforcedCursor.color {
                terminalView.caretColor = enforcedCursor.color
            }
        }

        /// Everything that must re-run after any bytes land: re-assert the
        /// cursor style, and re-scan visible text for bare http(s) URLs to
        /// make tappable (``PlainLinkDetector``) — the remote often only
        /// wraps a URL in a real OSC-8 hyperlink when it's alone on its own
        /// line, not one mentioned inline or inside a summary box.
        private func postFeedFixups(on terminalView: TerminalView) {
            enforceCursor(on: terminalView)
            PlainLinkDetector.linkify(terminalView)
        }

        /// Called when the terminal wants to send bytes upstream (typed
        /// keystrokes, paste, control sequences). Hook this up to
        /// `SSHService.write` (single pane) or `TmuxControlClient` (split).
        var onInput: ((Data) -> Void)?

        /// Called when the visible grid changes size — forward to the
        /// remote side via `SSHService.resize` / `tmux refresh-client -C`.
        var onSizeChange: ((_ cols: Int, _ rows: Int) -> Void)?

        /// Last grid size the view reported via `sizeChanged`. SwiftTerm only
        /// fires that on actual bounds changes, so a transport that comes up
        /// AFTER layout has settled (mosh bootstrap, reconnects) must read
        /// this and push it explicitly — waiting for the next event means
        /// waiting forever, leaving the remote PTY at its 80×24 default.
        private(set) var lastReportedSize: (cols: Int, rows: Int)?

        /// Called for OSC 0/2 title changes (e.g. shell prompts that set
        /// `\e]0;…\a`). Optional.
        var onTitleChange: ((String) -> Void)?

        /// Called when OSC 7 reports a new working directory. Optional.
        var onHostDirectoryChange: ((String?) -> Void)?

        /// Called when the remote rings the terminal bell (BEL). The Vibe
        /// Island agent monitor uses this as a "needs attention" signal.
        var onBell: (() -> Void)?

        /// Set for tmux panes: scroll requests (swipe / thumb) drive tmux
        /// copy-mode server-side instead of the local SwiftTerm scrollback (the
        /// only correct scrollback for a tmux pane). nil for a plain shell, where
        /// ``scroll(lines:)`` pages the local buffer. Positive = older, negative
        /// = newer.
        var onScroll: ((Int) -> Void)?

        /// Fired when a scroll gesture/drag BEGINS (before the first
        /// ``scroll(lines:)``). tmux panes use it to refresh whether the active
        /// pane's app wants the mouse (`#{mouse_any_flag}`) — the signal that
        /// decides wheel-vs-copy-mode — so the decision is fresh for this burst.
        var onScrollBegin: (() -> Void)?

        /// A horizontal swipe asked to switch pane/window. `forward == true` =
        /// next (swipe left), false = previous (swipe right). Set for tmux panes;
        /// nil for a plain shell (nothing to switch). The handler cycles panes in
        /// the active window when it has more than one, else cycles windows.
        var onSwitch: ((_ forward: Bool) -> Void)?

        /// Set for tmux panes: a tap asked for the cursor to move to a cell, and
        /// the controller forwards a click to the pane's program when that
        /// program wants the mouse. nil for a plain shell / local terminal,
        /// where ``click(col:row:)`` reports it through SwiftTerm's own encoder.
        /// Cells are 0-based, viewport-relative.
        var onClick: ((_ col: Int, _ row: Int) -> Void)?

        /// True when the LOCAL terminal's app has mouse reporting on (DECSET
        /// 1000/1002/1003). Only meaningful when there's no tmux indirection
        /// (plain SSH / mosh degraded to a bare shell): tmux's own `mouse on`
        /// would mask the pane app's state, so tmux sessions consult the `-CC`
        /// controller's per-pane `#{mouse_any_flag}` instead.
        var localAppWantsMouse: Bool {
            (terminalView?.getTerminal().mouseMode ?? .off) != .off
        }

        override init() {
            super.init()
        }

        // MARK: Input plumbing (parent → terminal)

        /// Feed remote output bytes into the terminal. Safe to call from
        /// any thread — SwiftTerm explicitly documents `feed` as
        /// background-thread safe.
        func feed(data: Data) {
            guard let terminalView else {
                // Buffer until a view attaches so no early output is lost.
                pendingFeed.append(data)
                return
            }
            // While the user reads scrollback, hold new output instead of
            // letting it yank the viewport to the bottom. Replay on release.
            if scrollHeld {
                heldFeed.append(data)
                heldBytes += data.count
                if heldBytes > heldByteLimit { releaseScrollHold() }
                return
            }
            terminalView.feed(byteArray: ArraySlice([UInt8](data)))
            postFeedFixups(on: terminalView)
        }

        // MARK: Output hold (scrollback reading)

        /// Begin holding output — the user has scrolled up off the bottom.
        func engageScrollHold() {
            scrollHeld = true
        }

        /// Notify the scroll owner that a gesture/drag is starting. Lets tmux
        /// panes refresh the active pane's mouse flag before the first tick.
        func scrollWillBegin() {
            onScrollBegin?()
        }

        /// Scroll the attached terminal's scrollback (drag thumb / swipe share
        /// this). Positive = toward older output, negative = newer.
        ///
        /// Routing (see the gesture architecture):
        ///  - tmux pane (`onScroll` set): hand off to the controller/session,
        ///    which decides wheel (mouse app) vs copy-mode (shell) from the
        ///    `-CC` controller's `#{mouse_any_flag}` and drives the right one.
        ///  - plain shell with a mouse app on screen: synthesize wheel events to
        ///    it (vim/less --mouse scroll themselves; copy-mode/local don't apply).
        ///  - plain shell, no mouse: page the LOCAL SwiftTerm scrollback,
        ///    holding live output while scrolled up so it doesn't snap back.
        func scroll(lines: Int) {
            guard lines != 0 else { return }
            if let onScroll {
                onScroll(lines)
                return
            }
            if localAppWantsMouse {
                sendWheel(lines: lines)
            } else {
                scrollLocal(lines: lines)
            }
        }

        /// Synthesize scroll-wheel events into the LOCAL terminal's app
        /// (non-tmux mouse apps). SwiftTerm's encoder emits them per the app's
        /// negotiated mouse protocol and routes them out through ``send`` →
        /// ``onInput`` — the same path as a typed key, so no copy-mode is
        /// involved and typing afterwards just works. Button 64 = wheel-up
        /// (older), 65 = wheel-down (newer); clamped so a fast flick doesn't
        /// fire dozens. No output-hold — the app repaints itself.
        func sendWheel(lines: Int) {
            guard lines != 0, let terminal = terminalView?.getTerminal() else { return }
            let button = lines > 0 ? 64 : 65
            let count = min(abs(lines), 6)
            let x = max(1, terminal.cols / 2), y = max(1, terminal.rows / 2)
            for _ in 0..<count {
                terminal.sendEvent(buttonFlags: button, x: x, y: y, pixelX: x, pixelY: y)
            }
        }

        /// Report a left click at a 0-based, viewport-relative cell — "put the
        /// cursor here" to a program that grabbed the mouse.
        ///
        /// Routing mirrors ``scroll(lines:)``: a tmux pane hands off to the
        /// controller (which consults the pane's `#{mouse_any_flag}`), and a
        /// plain shell lets SwiftTerm encode the press/release in whatever mouse
        /// protocol the local app negotiated — the same path a typed key takes.
        ///
        /// Does nothing when nothing on screen asked for the mouse: a bare shell
        /// would print the report as text (`0;12;3M` into the command line).
        func click(col: Int, row: Int) {
            if let onClick {
                onClick(col, row)
                return
            }
            guard localAppWantsMouse, let terminal = terminalView?.getTerminal() else { return }
            // 0 = left button press, 3 = release (SwiftTerm's `encodeButton`
            // convention, which its own tap handler uses).
            terminal.sendEvent(buttonFlags: 0, x: col, y: row, pixelX: col, pixelY: row)
            terminal.sendEvent(buttonFlags: 3, x: col, y: row, pixelX: col, pixelY: row)
        }

        /// Page the LOCAL SwiftTerm scrollback (plain non-tmux shell, or a mosh
        /// session degraded to a bare shell), holding live output while scrolled
        /// up so it doesn't snap back. Never consults `onScroll`, so an onScroll
        /// closure that falls back here for the no-tmux case can't recurse.
        func scrollLocal(lines: Int) {
            guard let terminalView, lines != 0 else { return }
            if lines > 0 {
                terminalView.scrollUp(lines: lines)
            } else {
                terminalView.scrollDown(lines: -lines)
            }
            if terminalView.scrollPosition >= 0.999 {
                releaseScrollHold()
            } else {
                engageScrollHold()
            }
        }

        /// Stop holding and replay everything buffered since the hold began, in
        /// order, so the viewport catches up to live output. Idempotent.
        func releaseScrollHold() {
            guard scrollHeld else { return }
            scrollHeld = false
            guard let terminalView else { heldFeed.removeAll(); heldBytes = 0; return }
            for chunk in heldFeed {
                terminalView.feed(byteArray: ArraySlice([UInt8](chunk)))
            }
            heldFeed.removeAll()
            heldBytes = 0
            postFeedFixups(on: terminalView)
        }

        /// Drop the hold AND the buffer without replaying — for a reconnect on
        /// a REUSED coordinator (the mosh path): bytes held from the previous
        /// connection belong to a dead screen, and replaying them would garble
        /// the fresh one (the remote repaints from scratch anyway).
        func discardScrollHold() {
            scrollHeld = false
            heldFeed.removeAll()
            heldBytes = 0
        }

        /// Feed a string (for seeding banners, status text, etc.). Same
        /// thread-safety note applies.
        func feed(text: String) {
            guard let terminalView else { return }
            terminalView.feed(text: text)
            postFeedFixups(on: terminalView)
        }

        /// Programmatic resize — typically driven by the parent in response
        /// to remote window-size requests rather than UI layout.
        func resize(cols: Int, rows: Int) {
            terminalView?.resize(cols: cols, rows: rows)
        }

        // MARK: Internal attach hook

        /// Fired when the user switches the input method (the globe key).
        /// Optional: tmux panes use it to resync from the server's model.
        var onInputModeChange: (() -> Void)?
        private var inputModeObserver: NSObjectProtocol?
        private var keyboardObserver: NSObjectProtocol?
        private var keyboardDidObserver: NSObjectProtocol?

        deinit {
            if let inputModeObserver {
                NotificationCenter.default.removeObserver(inputModeObserver)
            }
            if let keyboardObserver {
                NotificationCenter.default.removeObserver(keyboardObserver)
            }
            if let keyboardDidObserver {
                NotificationCenter.default.removeObserver(keyboardDidObserver)
            }
        }

        // MARK: Transition covers (keyboard frame-lock / pane-switch veil)

        /// Overlay masking the terminal through a pane/window switch, whose
        /// intermediate frames would look broken (the target pane's persistent
        /// terminal still shows its stale previous content). Revealed when the
        /// authoritative resync frame is fed (``reveal``) or by the safety
        /// timeout. Keyboard transitions do NOT use this — they lock the
        /// terminal's frame instead (see ``TerminalHostContainer``).
        private var transitionCover: UIView?
        private var coverTimeout: DispatchWorkItem?

        /// Whether a pane-switch cover is currently up.
        /// Internal (not private) purely so `@testable import` can assert on
        /// the resize/resync reveal-timing contract without UIKit-snooping.
        var isCoverPresented: Bool { transitionCover != nil }

        // MARK: Keyboard-transition frame lock

        /// The container whose layout we may freeze. Set by whichever host
        /// embeds the terminal (`SwiftTerminalView.makeUIView` /
        /// `PaneTerminalHost.makeUIView`). Assigning it also wires
        /// ``onGridReport``.
        weak var hostContainer: TerminalHostContainer? {
            didSet {
                hostContainer?.onTerminalLaidOut = { [weak self] terminal in
                    self?.reportGrid(of: terminal)
                }
            }
        }

        /// The grid the app's own layout implies, reported every time the host
        /// has given the terminal its real bounds.
        ///
        /// This exists because ``TerminalViewDelegate/sizeChanged`` cannot be
        /// used to learn the size — only to learn about a *change*. SwiftTerm
        /// returns early when the recomputed grid matches what the terminal
        /// already has, so a terminal minted at the right grid never reports at
        /// all, and an owner that seeded itself from an estimate keeps believing
        /// the estimate. (`SessionHub.estimateGrid` is a rough
        /// `fontSize × 0.6 / × 1.2` guess with a fudge factor for the chrome; it
        /// is routinely a column and a good ten rows off.) A tmux window pinned
        /// to that estimate renders the pane at a width this view does not have,
        /// and every line comes out wrapped short.
        var onGridReport: ((_ cols: Int, _ rows: Int) -> Void)?

        /// Compute the grid from the laid-out bounds and hand it to
        /// ``onGridReport``. Measured the same way SwiftTerm measures (the cell
        /// box from the font, then truncating division) so the two agree.
        private func reportGrid(of terminal: TerminalView) {
            guard let onGridReport else { return }
            let scale = terminal.window?.screen.scale ?? UITraitCollection.current.displayScale
            let cell = TerminalCellGeometry.measuredCell(font: terminal.font, scale: scale)
            guard cell.width > 0, cell.height > 0 else { return }
            let cols = Int(terminal.bounds.width / cell.width)
            let rows = Int(terminal.bounds.height / cell.height)
            guard cols > 0, rows > 0 else { return }
            onGridReport(cols, rows)
        }
        private var frameLockTimeout: DispatchWorkItem?

        /// Keep the terminal at its current size through a keyboard
        /// show/hide/height transition. SwiftUI shoves the host through
        /// PARTIAL sizes during the animation; letting each one hit SwiftTerm
        /// means several buffer reflows painting garbled intermediate frames
        /// (rows duplicated/overlapped — the "输入法/键盘切换画面乱码" bug) plus a
        /// resize storm at the remote. Locked, the content stays pixel-stable
        /// (clipped by the container) and ``unlockFrame()`` applies the final
        /// size in ONE step.
        ///
        /// `duration` comes from the keyboard notification's animation
        /// duration; the safety timeout adds headroom for the settle. An
        /// interactive drag-dismiss keeps re-arming this until its final
        /// notification, so the lock follows the gesture.
        func lockFrame(for duration: TimeInterval) {
            guard let hostContainer else { return }
            hostContainer.frameLocked = true
            frameLockTimeout?.cancel()
            let work = DispatchWorkItem { [weak self] in self?.unlockFrame() }
            frameLockTimeout = work
            DispatchQueue.main.asyncAfter(deadline: .now() + max(duration, 0.1) + 0.15,
                                          execute: work)
        }

        /// Let the terminal adopt the container's (now settled) bounds — one
        /// resize, one reflow, one remote size report. Idempotent.
        func unlockFrame() {
            frameLockTimeout?.cancel()
            frameLockTimeout = nil
            guard let hostContainer else { return }
            hostContainer.frameLocked = false
        }

        /// Cover the terminal with its background colour — used when switching
        /// TO this pane, whose buffer may hold stale content until the resync
        /// frame arrives. A deliberate clean veil reads as a transition; a beat
        /// of garbled stale text reads as a bug.
        func veilForSwitch() {
            guard let tv = terminalView else { return }
            if transitionCover == nil {
                let veil = UIView()
                veil.backgroundColor = tv.backgroundColor ?? .black
                veil.frame = tv.bounds
                veil.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                veil.isUserInteractionEnabled = false
                tv.addSubview(veil)
                transitionCover = veil
            }
            scheduleCoverTimeout()
        }

        /// Push the cover's auto-reveal deadline `seconds` further out, IF a
        /// cover is currently up. No-op otherwise — this must never itself
        /// start a veil that wasn't already requested by ``veilForSwitch()``.
        ///
        /// The default 0.8s safety cap assumes a near-instant resync
        /// round-trip; the tmux resize/resync schedule (debounce + a settled
        /// pass a further 700ms out) easily outlasts it over a real
        /// connection's RTT, so the cap itself was firing and revealing a
        /// mid-redraw frame (a duplicated or blank region flashing on
        /// screen) before the correct capture arrived. The controller calls
        /// this as it schedules each resync pass so the cap tracks the
        /// ACTUAL schedule instead of a fixed guess.
        func extendCoverTimeout(by seconds: TimeInterval) {
            guard transitionCover != nil else { return }
            scheduleCoverTimeout(after: seconds)
        }

        private func scheduleCoverTimeout(after seconds: TimeInterval = 0.8) {
            coverTimeout?.cancel()
            // Safety cap: if no resync ever lands (plain SSH/mosh, or the remote
            // is quiet), reveal the live view — a beat of garble beats a stuck
            // cover hiding live output.
            let work = DispatchWorkItem { [weak self] in self?.reveal() }
            coverTimeout = work
            DispatchQueue.main.asyncAfter(deadline: .now() + seconds, execute: work)
        }

        /// Fade the cover out (resync frame arrived, or timeout). Idempotent.
        func reveal() {
            coverTimeout?.cancel()
            coverTimeout = nil
            guard let cover = transitionCover else { return }
            transitionCover = nil
            UIView.animate(withDuration: 0.12, animations: { cover.alpha = 0 },
                           completion: { _ in cover.removeFromSuperview() })
        }

        /// Attach the coordinator to a live ``TerminalView``. Normally called
        /// automatically by ``SwiftTerminalView/makeUIView(context:)``; the
        /// tmux controller calls this explicitly when minting persistent
        /// terminals outside of the SwiftUI representable lifecycle.
        func attach(to terminalView: TerminalView) {
            self.terminalView = terminalView
            // Install scroll/zoom here (not in makeUIView) so it covers tmux
            // panes too — those are minted by the controller and only ever go
            // through this attach, never through the SwiftUI representable.
            TerminalScrollGesture.attach(to: terminalView, coordinator: self)
            // Switching the input method (globe key) is INVISIBLE to every
            // path we had: the CN/EN keyboards are the same height, so no
            // sizeChanged fires, and neither SwiftTerm nor UIKit repaints the
            // terminal — stale composing artifacts ("乱码") just sit there.
            // Listen for the switch itself: repaint the full local screen, and
            // let the owner resync from the authoritative model.
            if let inputModeObserver {
                NotificationCenter.default.removeObserver(inputModeObserver)
            }
            inputModeObserver = NotificationCenter.default.addObserver(
                forName: UITextInputMode.currentInputModeDidChangeNotification,
                object: nil, queue: .main
            ) { [weak self, weak terminalView] _ in
                guard let terminalView, terminalView.isFirstResponder else { return }
                terminalView.getTerminal().updateFullScreen()
                terminalView.setNeedsDisplay()
                self?.onInputModeChange?()
            }
            // Keyboard show/hide/height changes: hold the terminal at its
            // current size for the transition and resize once when it settles
            // (see TerminalHostContainer's doc — a snapshot cover taken here is
            // already too late, SwiftUI has re-laid-out before this observer
            // runs). willChange re-arms the lock (interactive dismiss fires it
            // repeatedly); didChange applies the final size.
            if let keyboardObserver {
                NotificationCenter.default.removeObserver(keyboardObserver)
            }
            keyboardObserver = NotificationCenter.default.addObserver(
                forName: UIResponder.keyboardWillChangeFrameNotification,
                object: nil, queue: .main
            ) { [weak self, weak terminalView] note in
                guard let terminalView, terminalView.isFirstResponder || terminalView.window != nil
                else { return }
                let duration = (note.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey]
                                as? TimeInterval) ?? 0.25
                self?.lockFrame(for: duration)
            }
            if let keyboardDidObserver {
                NotificationCenter.default.removeObserver(keyboardDidObserver)
            }
            keyboardDidObserver = NotificationCenter.default.addObserver(
                forName: UIResponder.keyboardDidChangeFrameNotification,
                object: nil, queue: .main
            ) { [weak self] _ in
                self?.unlockFrame()
            }
            if !pendingFeed.isEmpty {
                for chunk in pendingFeed {
                    terminalView.feed(byteArray: ArraySlice([UInt8](chunk)))
                }
                pendingFeed.removeAll()
                postFeedFixups(on: terminalView)
            }
        }

        // MARK: TerminalViewDelegate

        /// Sticky-Ctrl: armed by the bar's ctrl chip; the NEXT typed key is
        /// sent as its control code (ctrl-b, ctrl-l, ctrl-r… — otherwise
        /// unreachable on the software keyboard, and the mosh degrade banner
        /// literally tells the user to press one). One-shot.
        var pendingCtrl = false
        /// Lets the UI un-highlight the chip the moment the ctrl fires.
        var onPendingCtrlConsumed: (() -> Void)?

        /// Pinch-zoom ended — commit the final font size so it can persist
        /// (the next appearance pass used to silently snap it back).
        var onFontSizeCommit: ((Double) -> Void)?

        /// Fired after the user submits a line (the sent bytes contained CR/LF).
        /// tmux panes resync from the server shortly after: the IME commit +
        /// submit sequence (marked-text teardown, echo, the app clearing its
        /// input box) is where local cells most often diverge — "the text I
        /// sent is still sitting in the input bar".
        var onReturnSend: (() -> Void)?

        func send(source: TerminalView, data: ArraySlice<UInt8>) {
            // Typing means "I'm done reading" — resume live output and snap to
            // the bottom (the flush makes SwiftTerm follow the tail again).
            releaseScrollHold()
            // Sticky-Ctrl: fold the next single printable key into its control
            // code (standard mapping: byte & 0x1F covers @A-Z[\]^_ and a-z).
            if pendingCtrl, data.count == 1, let byte = data.first,
               (0x40...0x7e).contains(byte) {
                pendingCtrl = false
                onPendingCtrlConsumed?()
                onInput?(Data([byte & 0x1f]))
                return
            }
            // Committing IME input routes through here; force a FULL repaint so a
            // stale composing overlay (the "？？？" ghost) or leftover marked-text
            // cells can't linger until the next output. Skip it for a lone
            // backspace/delete byte: that's deleteBackward()'s hold-to-repeat
            // path, firing many times a second while the key is held, and a
            // full-screen redraw on every single one is what breaks the "holds
            // down = keeps deleting" feel (the OS's repeat cadence outruns the
            // redraw, so it stutters instead of deleting continuously).
            let isLoneEraseByte = data.count == 1 && (data.first == 0x7f || data.first == 0x08)
            if isLoneEraseByte { noteEraseByte() }
            if !isLoneEraseByte {
                source.getTerminal().updateFullScreen()
                source.setNeedsDisplay()
            }
            if data.contains(0x0d) || data.contains(0x0a) {
                onReturnSend?()
            }
            guard let onInput else { return }
            onInput(Data(data))
        }

        /// Erase bytes seen in the current burst, and when the last one landed.
        private var eraseBurstCount = 0
        private var lastEraseAt: ContinuousClock.Instant?

        /// Record one backspace/delete byte on its way to the PTY.
        ///
        /// This separates two failures that look identical on screen —
        /// "holding backspace stops deleting":
        ///
        /// - the log goes **quiet** while the key is still held ⇒ UIKit stopped
        ///   calling `deleteBackward()`, so the fault is upstream of us (a
        ///   `hasText` gate, an empty shadow document, or a marked-text
        ///   composition swallowing the repeats before they become bytes);
        /// - the log keeps **ticking** but the screen doesn't move ⇒ the bytes
        ///   left the app, so the fault is downstream — tmux `send-keys` is one
        ///   control-mode round trip PER BYTE through a serialized write chain,
        ///   or the local screen is out of sync with what the server holds.
        ///
        /// It was the first: a three-second hold over 20 pasted characters sent
        /// one byte, and over 10 characters typed on the same line sent exactly
        /// ten and then stopped — UIKit measures the shadow document between
        /// ticks and quits when nothing sits in front of the caret. Fixed in the
        /// SwiftTerm fork (patch 12, `docs/PATCHES.md`); the same hold now sends
        /// 25. Keep the counter: it is the cheapest way to tell a regression
        /// here from a tmux stall, which look the same to a tester.
        ///
        /// A gap longer than a second starts a new burst, so each hold reads as
        /// its own run of counts rather than one number climbing forever.
        private func noteEraseByte() {
            let now = ContinuousClock.now
            if let last = lastEraseAt, now - last > .seconds(1) { eraseBurstCount = 0 }
            lastEraseAt = now
            eraseBurstCount += 1
            Log.input.info("erase byte #\(self.eraseBurstCount, privacy: .public) in this burst")
        }

        func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            lastReportedSize = (newCols, newRows)
            onSizeChange?(newCols, newRows)
        }

        func setTerminalTitle(source: TerminalView, title: String) {
            onTitleChange?(title)
        }

        func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
            onHostDirectoryChange?(directory)
        }

        // MARK: TerminalViewDelegate — stubs (no-op for M1)

        func scrolled(source: TerminalView, position: Double) {
            // no-op: scroll position is owned by SwiftTerm internally.
        }

        func requestOpenLink(source: TerminalView, link: String, params: [String : String]) {
            // A tapped terminal hyperlink (OSC-8, or a plain URL when the build
            // surfaces one). Open only safe web/mail schemes — never arbitrary
            // custom schemes from remote output.
            let trimmed = link.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let url = URL(string: trimmed),
                  let scheme = url.scheme?.lowercased(),
                  ["http", "https", "mailto", "ftp", "ftps"].contains(scheme) else { return }
            Task { @MainActor in
                UIApplication.shared.open(url)
            }
        }

        func bell(source: TerminalView) {
            onBell?()
        }

        func clipboardCopy(source: TerminalView, content: Data) {
            // no-op for M1; OSC 52 paste-from-remote disabled by default.
        }

        func clipboardRequest(source: TerminalView) {
            // OSC 52 read query — the remote is asking for the phone's
            // clipboard. Gated on an off-by-default setting because this is
            // an exfiltration channel: with it off we still ANSWER, with an
            // empty clipboard (the conventional refusal), so a remote program
            // waiting on the reply unblocks without learning anything.
            //
            // Text only for now; images wait on the ecosystem picking a
            // protocol (kitty OSC 5522 — see Claude Code #42712). Pasteboard
            // reads can block the main thread for over a second (see the
            // paste chip), so the read happens off it.
            let allowed = AppSettings.shared.remoteClipboardReadEnabled
            Task { [weak source] in
                let content: Data?
                if allowed {
                    let text = await Task.detached { UIPasteboard.general.string }.value
                    content = text?.data(using: .utf8)
                } else {
                    content = nil
                }
                await MainActor.run { [weak source] in
                    source?.getTerminal().sendClipboardResponse(content: content)
                }
            }
        }

        func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {
            // no-op: iTerm2-specific OSC 1337 escapes are out of scope.
        }

        func rangeChanged(source: TerminalView, startY: Int, endY: Int) {
            // no-op: only fires when `notifyUpdateChanges` is enabled.
        }
    }
}
