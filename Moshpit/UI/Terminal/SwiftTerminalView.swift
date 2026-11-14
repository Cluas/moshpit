import SwiftUI
import UIKit
import MoshpitKit

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
    /// Neither take nor surrender: the keyboard stays wherever the user left
    /// it. (A tap no longer focuses the terminal — `focusOnTap` is off — so
    /// "allow" is about not resigning focus the toggle granted, not about
    /// granting any.)
    case allow
    /// Give it up: a sheet is covering the screen, or the user put the
    /// keyboard away on purpose.
    case resign
}

/// Hosts a terminal (a ``TerminalSurface``'s view) and owns the ONLY code
/// allowed to set its frame.
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
    /// The hosted terminal, whichever view draws it. Weak: terminals are
    /// persistent objects owned by the session (tmux panes outlive any single
    /// host container).
    private(set) weak var surface: TerminalSurface?

    /// The hosted terminal's view.
    var hostedView: UIView? { surface?.view }

    /// While true, ``layoutSubviews`` leaves the terminal's frame alone.
    /// Toggled by the coordinator around keyboard transitions.
    var frameLocked = false {
        didSet {
            if !frameLocked { setNeedsLayout() }
        }
    }

    /// A second, independent hold on the frame, for a modal that is going to
    /// give the space back.
    ///
    /// Presenting one of the navigation sheets collapses the keyboard, and
    /// the terminal behind the sheet grows into the space it left: a full
    /// buffer reflow and a resize sent to the remote, both undone again the
    /// moment the sheet closes and the keyboard comes back. Nobody ever saw
    /// either size — the sheet covered the screen for both — but the pane
    /// came back re-flowed, and the user's grid changed twice for a switch
    /// that was never about the size ("切 window 会自动重画布局", report).
    /// Separate from ``frameLocked`` because that one is a *timed* lock owned
    /// by the keyboard transition; this one lasts as long as the modal does.
    var geometryHeld = false {
        didSet {
            if !geometryHeld { setNeedsLayout() }
        }
    }

    /// Asked before a size change is applied to the terminal. Returning true
    /// means "hold it — I will call ``applyDeferredResize()`` when the repaint
    /// that belongs with it has arrived."
    ///
    /// Why anyone would want that: shrinking a buffer is LOSSY. SwiftTerm
    /// removes rows by popping the lines below the cursor first
    /// (`Buffer.resize`), and a full-screen TUI puts its own furniture down
    /// there — Claude Code's separator and its "bypass permissions" line sit
    /// under the input box, and the keyboard coming up simply deleted them
    /// (user screenshots). They come back only when the remote repaints, one
    /// round trip later, and that gap is the jitter. Nothing local can fix
    /// it: those rows are gone and only the server knows what was in them.
    /// So don't take the loss until the replacement is in hand.
    var shouldDeferResize: ((_ cols: Int, _ rows: Int) -> Bool)?
    private var deferredResize: CGRect?

    /// Apply the size change that ``shouldDeferResize`` held back. Call this
    /// immediately BEFORE feeding the repaint that goes with it, so the two
    /// land in one turn and the lossy moment is never drawn. Returns whether
    /// there was one to apply.
    @discardableResult
    func applyDeferredResize() -> Bool {
        guard let target = deferredResize, let hostedView else { return false }
        deferredResize = nil
        hostedView.frame = target
        onTerminalLaidOut?(hostedView)
        return true
    }

    /// Whether the terminal has yet to take this container's size: a
    /// keyboard move is still in flight (``frameLocked``), or a resize is
    /// held for the repaint that goes with it (``shouldDeferResize``). The
    /// keyboard cover stays up until it has.
    var resizePending: Bool { frameLocked || deferredResize != nil }

    func host(_ surface: TerminalSurface) {
        let terminal = surface.view
        guard terminal.superview !== self else { return }
        self.surface = surface
        addSubview(terminal)
        // Only adopt the container's bounds when the container HAS bounds. A
        // freshly created container is .zero until its first layout pass, and
        // framing a REUSED terminal to .zero makes SwiftTerm reflow the whole
        // buffer to a one-column grid — an xterm.js-style reflow that does not
        // round-trip, so when the real size lands the screen stays garbled
        // (2-character-wide shards of Claude Code's status line down the left
        // edge; user screenshot, 2026-08-17). mosh then patches diffs against
        // the server's un-resized model and the mess never self-corrects.
        // Keeping the terminal's previous frame until layoutSubviews delivers
        // real bounds means the common case (same device, same size) never
        // reflows at all.
        if bounds != .zero {
            terminal.frame = bounds
        }
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
    var onTerminalLaidOut: ((UIView) -> Void)?

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let terminalView = hostedView, !frameLocked, !geometryHeld else { return }
        // Same guard as host(): a transient zero-bounds pass (sheet
        // transitions, remounts) must not squeeze a live buffer through the
        // minimum grid — that reflow doesn't round-trip.
        guard bounds.width > 0, bounds.height > 0 else { return }
        if terminalView.frame != bounds {
            if terminalView.frame.size != bounds.size, let shouldDeferResize {
                let cell = surface?.cellSize ?? .zero
                let grid = TerminalCellGeometry.grid(for: bounds.size, cell: cell)
                if grid.cols > 0, grid.rows > 0, shouldDeferResize(grid.cols, grid.rows) {
                    deferredResize = bounds
                    return
                }
            }
            terminalView.frame = bounds
        }
        onTerminalLaidOut?(terminalView)
    }
}

/// SwiftUI host of the session's terminal (a ``TerminalSurface``: SwiftTerm's
/// view or the core's, as ``CoreTermSwitch`` decides when it is made).
///
/// ### Persistence contract (DO NOT BREAK)
///
/// The terminal is **persistent**: SwiftUI may invoke
/// ``UIViewRepresentable/updateUIView(_:context:)`` many times across re-renders,
/// but the terminal — including its scrollback, cursor position, and the data
/// we have already fed into it — must survive every re-render.
///
/// As a result, `updateUIView` is **only** allowed to mutate cosmetic state
/// (theme, font, alpha, etc.). It MUST NOT feed the terminal, clear it, or
/// swap its delegate.
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

    /// Hold the terminal at the size it has right now — see
    /// ``TerminalHostContainer/geometryHeld``. True while a navigation sheet
    /// is up, so the pane keeps the grid it had across the switch.
    var geometryHeld: Bool = false

    func makeCoordinator() -> Coordinator {
        // SwiftUI calls this once per representable identity. Return the
        // caller-owned coordinator so SwiftUI's internal bookkeeping matches
        // the instance the parent view is holding onto.
        coordinator
    }

    private var look: TerminalLook {
        TerminalLook(theme: theme, cursorShape: cursorShape, cursorColorId: cursorColorId, cursorBlink: cursorBlink)
    }

    func makeUIView(context: Context) -> TerminalHostContainer {
        // Reuse the coordinator's own terminal when it has one: this is a
        // REMOUNT of a live session (left the screen, came back), and the
        // existing terminal is the only complete copy of the screen — mosh
        // in particular never resends what it believes the client already
        // has. See `Coordinator.surface`.
        let surface = Self.mintOwnedSurface(coordinator: coordinator, fontName: fontName, fontSize: fontSize)
        surface.view.removeFromSuperview()
        let container = TerminalHostContainer()
        container.host(surface)
        coordinator.hostContainer = container
        container.geometryHeld = geometryHeld
        surface.apply(look: look)
        // Pop the system keyboard as soon as the terminal is on screen — the
        // app's shortcut bar rides above it via safeAreaInset.
        if focusPolicy == .take {
            let view = surface.view
            DispatchQueue.main.async { [weak view] in _ = view?.becomeFirstResponder() }
        }
        return container
    }

    /// The plain path's terminal (mosh, plain SSH, herdr) for `coordinator`:
    /// its own when it has one (re-wired: a remount), else a new one, owned
    /// by it (``Coordinator/surface``). Whatever hosts the session's terminal
    /// — this representable, or the NSH page's stage — gets it here.
    ///
    /// A new one is born at the estimated phone grid, not `.zero`. A `.zero`
    /// frame derives a two-column grid, and everything fed before the first
    /// layout pass — mosh's first screen diffs, herdr's raw-attach repaint,
    /// both beat `makeUIView` by design (see `pendingFeed`) — gets wrapped at
    /// two columns. Those shards sink into the scrollback, and a reflow does
    /// not round-trip (see TerminalHostContainer.host), so scrolling up
    /// shows a column of 1–2 character fragments forever while the visible
    /// screen self-corrects (user screenshot, 2026-08-18).
    ///
    /// Specifically `estimateGrid`, because that is the grid the session
    /// seeds the transport with (`lastReportedSize ?? estimateGrid`, and
    /// lastReportedSize is nil this early): birth grid == seed grid means the
    /// pre-layout feed lands in the shape the server drew it for, and the
    /// first real layout — if it disagrees — fires `sizeChanged`, which
    /// corrects BOTH sides. A birth grid that guesses better than the seed
    /// would break that pact: whenever it happened to match the final
    /// layout, `sizeChanged` never fires and the server stays at the seed
    /// forever.
    @MainActor
    static func mintOwnedSurface(coordinator: Coordinator, fontName: String,
                                 fontSize: Double) -> TerminalSurface {
        if let existing = coordinator.surface {
            coordinator.attach(existing)   // idempotent
            return existing
        }
        let font = TerminalFont.font(id: fontName, size: CGFloat(fontSize))
        let seed = SessionHub.ActiveSession.estimateGrid(fontSize: fontSize)
        let surface = TerminalSurfaces.make(font: font, grid: TerminalGrid(cols: seed.cols, rows: seed.rows))
        coordinator.attach(surface)
        return surface
    }

    func updateUIView(_ uiView: TerminalHostContainer, context: Context) {
        // === COSMETIC ONLY ===
        // No feed, no buffer clear, no delegate swap. See type doc.
        guard let surface = uiView.surface else { return }
        if uiView.geometryHeld != geometryHeld { uiView.geometryHeld = geometryHeld }
        surface.apply(font: TerminalFont.font(id: fontName, size: CGFloat(fontSize)))
        // Only what changed since the last push: SwiftTerm's colour setters
        // drop its attribute cache and repaint the whole screen, and SwiftUI
        // calls `updateUIView` on every re-render of the parent (each SRTT
        // tick on a mosh session included).
        surface.apply(look: look)
        Self.applyFocus(focusPolicy, to: surface)
    }

    /// ``TerminalFocusPolicy`` on any terminal: take the keyboard (now, and
    /// back if something stole it — not from its own find bar), leave it,
    /// or give it up.
    @MainActor
    static func applyFocus(_ policy: TerminalFocusPolicy, to surface: TerminalSurface) {
        let view = surface.view
        switch policy {
        case .take:
            if !view.isFirstResponder, view.window != nil, !surface.isFinding {
                DispatchQueue.main.async { [weak view] in _ = view?.becomeFirstResponder() }
            }
        case .allow:
            // Nobody has asked for the keyboard: leave first responder
            // exactly as the user left it, so a tap on the pane can raise it.
            break
        case .resign:
            if view.isFirstResponder {
                // Keyboard put away (toggle) or a sheet is up — drop it.
                DispatchQueue.main.async { [weak view] in _ = view?.resignFirstResponder() }
            }
        }
    }

    // MARK: - Coordinator

    /// The session's terminal host: owns its terminal (a ``TerminalSurface``,
    /// whichever view draws it), owns the data-input path (``feed(data:)``),
    /// routes what the terminal and its gestures ask for (input upstream,
    /// size changes, scrolling, clicks, pane switches) to the closures its
    /// owner installs, and holds the transition covers (keyboard frame lock,
    /// pane-switch veil). The same for the plain path (mosh, plain SSH,
    /// herdr) and every tmux pane; nothing here knows which view draws.
    ///
    /// This is a plain reference type — no Combine, no Observation — so a
    /// parent view can hold it via `@State` (reference identity is stable
    /// across re-renders) without dragging in property-wrapper machinery
    /// that the representable doesn't need.
    final class Coordinator: NSObject {
        /// The terminal, owned HERE so it survives the screen unmounting. It
        /// has to: mosh's screen model lives server-side and only diffs
        /// arrive, so a freshly minted empty terminal after re-entering the
        /// same session stays empty — the server has no idea the client
        /// discarded its framebuffer. Leaving the terminal screen and tapping
        /// back into the SAME pane showed a bare cursor (user report,
        /// 2026-08-17). Keeping it alive keeps its screen current (feeds
        /// continue while unmounted), so a remount re-parents a fully
        /// painted, up-to-date screen. A tmux pane's coordinator is owned by
        /// its controller, and the pane's terminal with it.
        private(set) var surface: TerminalSurface?

        /// Output that arrived before the terminal attached (e.g. mosh's
        /// first screen diffs land within ~50ms, before `makeUIView` runs).
        /// Flushed in order once ``attach(_:)`` wires up the terminal.
        private var pendingFeed: [Data] = []

        /// Everything fed so far while no terminal is attached, in order —
        /// what a terminal would be handed on attach. Unit tests run without
        /// a terminal and read its input here.
        var unattachedFeed: [Data] { pendingFeed }

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
        ///
        /// Deliberately fed ONLY by `sizeChanged`, never by the layout
        /// report: recording reportGrid's value here shipped in build 368
        /// and pinned tmux windows to a junk grid from a transient layout
        /// pass (499×62 — "SSH+tmux 满屏乱码" regression, reverted in 369).
        /// The plain path instead keeps the birth grid and the seed grid
        /// equal by construction — see `makeUIView` — so a terminal that
        /// never fires `sizeChanged` is one whose seed was already right.
        private(set) var lastReportedSize: (cols: Int, rows: Int)?

        /// Called for OSC 0/2 title changes (e.g. shell prompts that set
        /// `\e]0;…\a`). Optional.
        var onTitleChange: ((String) -> Void)?

        /// Called when OSC 7 reports a new working directory. Optional.
        var onHostDirectoryChange: ((String?) -> Void)?

        /// Which paths on this terminal the host has confirmed, so the link
        /// pass can make them tappable (`PathLinkDetector`). nil = paths stay
        /// plain text. An answer from the host schedules another pass.
        var pathLinks: RemotePathLinks? {
            didSet { withSurface { $0.pathLinks = pathLinks } }
        }

        /// A tapped host-path link — one `pathLinks` confirmed, or Claude
        /// Code's own OSC-8 `file://` link. Receives the absolute host path.
        var onOpenRemotePath: ((String) -> Void)?

        /// Called when the remote rings the terminal bell (BEL). The Vibe
        /// Island agent monitor uses this as a "needs attention" signal.
        var onBell: (() -> Void)?

        /// A mark fed with ``feed(mark:)`` is due: every byte fed before it
        /// is on screen, none after it is (``TerminalSurface``).
        var onMark: ((TerminalMark) -> Void)?

        /// Set for tmux panes: scroll requests (swipe / thumb) drive tmux
        /// copy-mode server-side instead of the local SwiftTerm scrollback (the
        /// only correct scrollback for a tmux pane). nil for a plain shell, where
        /// ``scroll(pixels:)`` pages the local buffer. Positive = older, negative
        /// = newer.
        var onScroll: ((Int) -> Void)?

        /// Set for tmux panes alongside ``onScroll``: pixel-precise scroll
        /// requests from the pan gesture (positive = older). Returns the
        /// distance consumed — 0 when the route drops the tick or the buffer
        /// is at its edge. When nil, ``scroll(pixels:)`` folds the pixels into
        /// whole rows for ``onScroll`` (mosh / herdr: server-side rows are all
        /// there is).
        var onScrollPixels: ((CGFloat) -> CGFloat)?

        /// Set alongside ``onScrollPixels``: the drag came to rest.
        var onScrollEnd: (() -> Void)?

        /// Fired when a scroll gesture/drag BEGINS (before the first
        /// ``scroll(pixels:)``). tmux panes use it to refresh whether the active
        /// pane's app wants the mouse (`#{mouse_any_flag}`) — the signal that
        /// decides wheel-vs-copy-mode — so the decision is fresh for this burst.
        var onScrollBegin: (() -> Void)?

        /// One text row in points — the unit that turns finger travel into rows.
        var cellHeight: CGFloat {
            let height = withSurface { $0.cellSize.height } ?? 0
            return height > 0 ? height : 16
        }

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
            withSurface { $0.modes.mouseReporting } ?? false
        }

        /// The modes the terminal's program set; nil without a terminal.
        var modes: TerminalModes? {
            withSurface { $0.modes }
        }

        override init() {
            super.init()
        }

        /// `body` with the terminal, on the main actor (every caller is on the
        /// main thread; the coordinator itself predates the annotations).
        @discardableResult
        func withSurface<R>(_ body: @MainActor (TerminalSurface) -> R) -> R? {
            guard let surface else { return nil }
            return MainActor.assumeIsolated { body(surface) }
        }

        // MARK: Input plumbing (parent → terminal)

        #if DEBUG
        /// Every chunk fed, with its coordinator: the echo probe's view of
        /// output on either session path (`RustSessionSeed`).
        nonisolated(unsafe) static var debugFeedTap: ((Coordinator, Data) -> Void)?
        #endif

        /// Feed remote output bytes into the terminal, after everything fed
        /// before (main thread).
        func feed(data: Data) {
            noteFed(data)
            guard let surface else {
                // Buffer until a terminal attaches so no early output is lost.
                TmuxSessionController.ccTap("cc-pairing.log",
                    Data("FEED-PENDING bytes=\(data.count)\n".utf8))
                pendingFeed.append(data)
                return
            }
            MainActor.assumeIsolated { surface.feed(data) }
            if prediction != nil { schedulePredictionTick() }
        }

        private func noteFed(_ data: Data) {
            #if DEBUG
            Self.debugFeedTap?(self, data)
            #endif
            lastFedAt = CACurrentMediaTime()
        }

        /// A mark after the bytes fed so far: ``onMark`` hears it once they
        /// are on screen (a view that parses as it is fed: at once; the
        /// core's: with the frame that has them). Without a terminal yet, at
        /// once.
        func feed(mark: TerminalMark) {
            guard let surface else { reached(mark); return }
            MainActor.assumeIsolated { surface.feed(mark: mark) }
        }

        /// Lifts the pane-switch veil once every byte fed so far is on
        /// screen: at once on SwiftTerm's view, with the frame that has them
        /// on the core's (a reveal right after a feed would lift the veil
        /// over a repaint the core has not parsed yet).
        func revealWhenShown() {
            feed(mark: TerminalMark(code: TerminalMark.Code.revealWhenShown, arg: 0))
        }

        /// A mark is due: the coordinator's own, or its owner's.
        fileprivate func reached(_ mark: TerminalMark) {
            if mark.code == TerminalMark.Code.revealWhenShown {
                reveal()
                return
            }
            onMark?(mark)
        }

        // MARK: Predictive echo (mosh)

        /// mosh's local echo for this terminal (see ``MoshPrediction``). Set
        /// by a mosh session for the life of its transport; nil everywhere
        /// else, which leaves every path here a no-op. The engine object
        /// keeps the mode and the round trip; the terminal draws the guesses
        /// (SwiftTerm into its cells, the core over its frames).
        private(set) var prediction: MoshPrediction?
        private var predictionTick: DispatchWorkItem?

        /// Install (or remove, with nil) the session's predictor, taking down
        /// whatever the previous one had drawn.
        func attachPrediction(_ engine: MoshPrediction?) {
            predictionTick?.cancel()
            predictionTick = nil
            prediction = engine
            withSurface { $0.attachPrediction(engine) }
        }

        /// Keystrokes on their way to the mosh server: draw their likely echo
        /// now. Returns the batch to report back through
        /// ``predictionSent(batch:userState:)`` once the transport has queued
        /// the bytes, or nil when nothing was predicted.
        func predict(_ data: Data) -> Int? {
            guard let prediction else { return nil }
            guard let surface else {
                // Nothing to draw on: start over once there is.
                prediction.reset()
                return nil
            }
            let batch = MainActor.assumeIsolated { surface.predict(data) }
            schedulePredictionTick()
            return batch
        }

        /// Bytes the engine can't model are about to move the screen: take
        /// every guess down; the next keystroke starts a new epoch.
        func dropPredictions() {
            guard let prediction else { return }
            guard let surface else { prediction.reset(); return }
            MainActor.assumeIsolated { surface.dropPredictions() }
        }

        func predictionSent(batch: Int, userState: UInt64?) {
            guard let surface else { prediction?.sent(batch: batch, as: userState); return }
            MainActor.assumeIsolated { surface.predictionSent(batch: batch, userState: userState) }
        }

        /// One applied mosh diff: its bytes, then its echo ack, judged after
        /// them (a frame that only moved the ack still settles the guesses it
        /// now covers).
        func feed(moshFrame frame: MoshHostFrame) {
            if !frame.bytes.isEmpty { noteFed(frame.bytes) }
            guard let surface else {
                prediction?.noteEchoAck(frame.echoAck)
                if !frame.bytes.isEmpty { pendingFeed.append(frame.bytes) }
                return
            }
            MainActor.assumeIsolated { surface.feed(moshFrame: frame) }
            schedulePredictionTick()
        }

        /// mosh wakes every 50ms while a guess is pending, so one stuck past
        /// 250ms gets shown (and marked past 5s) without waiting for the next
        /// keystroke or frame. Clock-only, so it redraws only on a change.
        private func schedulePredictionTick() {
            guard predictionTick == nil, prediction != nil,
                  withSurface({ $0.predictionNeedsTick }) == true else { return }
            let work = DispatchWorkItem { [weak self] in
                guard let self else { return }
                self.predictionTick = nil
                guard self.prediction != nil else { return }
                if self.withSurface({ $0.predictionTick() }) == true {
                    self.schedulePredictionTick()
                }
            }
            predictionTick = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: work)
        }

        // MARK: Scrolling

        /// Notify the scroll owner that a gesture/drag is starting. Lets tmux
        /// panes refresh the active pane's mouse flag before the first tick.
        func scrollWillBegin() {
            withSurface { $0.dragWillBegin() }
            onScrollBegin?()
        }

        /// Scroll the terminal's history from the pan gesture: finger travel
        /// in points, positive = toward older output.
        ///
        /// Routing (see the gesture architecture):
        ///  - tmux pane (`onScrollPixels` set): hand off to the controller,
        ///    which decides wheel (mouse app) vs copy-mode (shell) from the
        ///    `-CC` controller's `#{mouse_any_flag}` and drives the right one.
        ///  - otherwise the terminal routes it (TERMINAL-VIEW.md V20): rows
        ///    for the host when `onScroll` is set (mosh, herdr: server-side
        ///    rows are all there is); a wheel for a program on screen that
        ///    wants the mouse; else the LOCAL history.
        ///
        /// Local history moves by the exact distance (sub-row offsets
        /// included, so the content tracks the finger); wheel and server-side
        /// routes get whole rows with the remainder carried to the next tick.
        /// Returns the distance consumed (0 = nothing moved).
        @discardableResult
        func scroll(pixels dy: CGFloat) -> CGFloat {
            guard dy != 0, dy.isFinite else { return 0 }
            if let onScrollPixels {
                return onScrollPixels(dy)
            }
            guard let routed = withSurface({ $0.drag(dy, hostRows: onScroll != nil) }) else { return 0 }
            if routed.rows != 0 { onScroll?(routed.rows) }
            return routed.moved
        }

        /// The drag came to rest.
        func scrollDidEnd() {
            if let onScrollEnd {
                onScrollEnd()
                return
            }
            if onScroll == nil, !localAppWantsMouse {
                settleLocalScroll()
            }
        }

        /// Wheel events into the LOCAL terminal's app (non-tmux mouse apps),
        /// encoded per the app's negotiated mouse protocol and sent the same
        /// way as a typed key, so no copy-mode is involved and typing
        /// afterwards just works. Clamped so a fast flick doesn't fire
        /// dozens. No output-hold — the app repaints itself.
        func sendWheel(lines: Int) {
            guard lines != 0 else { return }
            withSurface { $0.sendWheel(lines: lines) }
        }

        /// Report a left click at a 0-based, viewport-relative cell — "put the
        /// cursor here" to a program that grabbed the mouse.
        ///
        /// Routing mirrors ``scroll(pixels:)``: a tmux pane hands off to the
        /// controller (which consults the pane's `#{mouse_any_flag}`), and a
        /// plain shell lets the terminal encode the press/release in whatever
        /// mouse protocol the local app negotiated — the same path a typed
        /// key takes.
        ///
        /// Does nothing when nothing on screen asked for the mouse: a bare shell
        /// would print the report as text (`0;12;3M` into the command line).
        func click(col: Int, row: Int) {
            if let onClick {
                onClick(col, row)
                return
            }
            clickLocal(col: col, row: row)
        }

        /// The local half of ``click(col:row:)`` — the terminal encodes the
        /// press/release itself. Never consults `onClick`, so an onClick
        /// closure whose routing falls back here can't recurse. That is not
        /// hypothetical: the mosh onClick wiring routes to
        /// `clickActiveTerminal`, whose no-multiplexer fallback used to call
        /// `click` — with a herdr frame channel that failed to attach, one
        /// tap on the terminal recursed 9000 frames deep and segfaulted
        /// (TestFlight 351 crash, 2026-08-17). `scrollLocal` learned this
        /// same lesson first; the click path just hadn't caught up.
        func clickLocal(col: Int, row: Int) {
            guard localAppWantsMouse else { return }
            withSurface { $0.clickLocal(col: col, row: row) }
        }

        /// Page the LOCAL history (plain non-tmux shell, or a mosh session
        /// degraded to a bare shell). Never consults `onScroll`, so an
        /// onScroll closure that falls back here for the no-tmux case can't
        /// recurse.
        func scrollLocal(lines: Int) {
            guard lines != 0 else { return }
            withSurface { $0.scrollLocal(lines: lines) }
        }

        /// Pixel-precise ``scrollLocal(lines:)``: move the viewport `dy` points
        /// (positive = older), keeping sub-row offsets so the content follows
        /// the finger instead of lurching a row at a time. Returns the
        /// distance actually travelled (0 at an edge).
        @discardableResult
        func scrollLocal(pixels dy: CGFloat) -> CGFloat {
            guard dy != 0 else { return 0 }
            return withSurface { $0.scrollLocal(pixels: dy) } ?? 0
        }

        /// A local drag came to rest. A viewport parked within half a
        /// row of the live bottom docks onto it: the reader is done, and a
        /// half-row of history left on screen would keep output held and make
        /// taps read as "scrolled up".
        func settleLocalScroll() {
            withSurface { $0.settleLocalScroll() }
        }

        // MARK: Output hold (scrollback reading)

        /// Back on the live screen: whatever output was held while history
        /// was read goes on screen, in order. Idempotent.
        func releaseScrollHold() {
            withSurface { $0.releaseScrollHold() }
        }

        /// Drop the hold AND the buffer without replaying — for a reconnect on
        /// a REUSED coordinator (the mosh path): bytes held from the previous
        /// connection belong to a dead screen, and replaying them would garble
        /// the fresh one (the remote repaints from scratch anyway).
        func discardScrollHold() {
            withSurface { $0.discardScrollHold() }
        }

        /// Forget the dead connection's unfinished input. A weak-network
        /// disconnect routinely cuts the stream mid-CJK-character or — worse
        /// — mid-OSC (coding agents retitle constantly), and this coordinator
        /// and its terminal are REUSED across reconnects. Without this, the
        /// fresh connection's first bytes are parsed as the dead stream's
        /// continuation: a poisoned putback mangles the first character; a
        /// dangling OSC swallows the whole first repaint up to the next BEL
        /// ("重连后满屏乱码"). Screen, scrollback and modes stay untouched.
        /// Call at every (re)connect boundary, before the new stream feeds.
        func resetInputParser() {
            withSurface { $0.discardPendingInput() }
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
        /// When the veil that is up went up (B386: a veil still up long
        /// after its safety cap is a stuck one).
        private var veiledAt: CFTimeInterval = 0

        /// What a pane-switch veil is called in the view tree: a veil left
        /// in a terminal no coordinator lifts any more can still be found.
        static let veilIdentifier = "terminal-switch-veil"

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
                hostContainer?.onTerminalLaidOut = { [weak self] hosted in
                    guard let self else { return }
                    self.reportGrid(bounds: hosted.bounds.size)
                    self.withSurface { $0.laidOut() }
                }
                // Container-level wiring belongs HERE, not at the assignment
                // sites: a coordinator is configured when its pane is minted,
                // long before SwiftUI hands it a container, and the container
                // is re-created on every remount. Setting it anywhere else
                // silently loses whichever side happened to come second — the
                // deferred-resize gate was installed and never propagated for
                // exactly that reason.
                hostContainer?.shouldDeferResize = shouldDeferResize
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
        /// ``onGridReport``, measured as the terminal measures (its cell box,
        /// then truncating division) so the two agree. Must NOT write
        /// ``lastReportedSize`` — see its doc for the 368 junk-grid
        /// regression that taught us that.
        private func reportGrid(bounds: CGSize) {
            guard let onGridReport, let grid = withSurface({ $0.grid(fitting: bounds) }),
                  grid.cols > 0, grid.rows > 0 else { return }
            onGridReport(grid.cols, grid.rows)
        }
        private var frameLockTimeout: DispatchWorkItem?

        // MARK: Keyboard cover

        /// A still image of the pane, laid over it for the length of a
        /// keyboard transition.
        ///
        /// The flicker a keyboard resize produces is NOT ours to fix in the
        /// renderer. Measured server-side against a real Claude Code pane: on
        /// resize, tmux's OWN copy of the pane loses its footer for about
        /// 50ms while the app redraws — and our immediate capture-pane lands
        /// inside that window, so we faithfully paint a screen the app had
        /// half-erased, and the corrected one arrives with the settled pass
        /// ~700ms later. Every attempt to hold or reorder the local resize
        /// left that intact, because the repaint we were waiting for was
        /// itself the broken frame.
        ///
        /// So stop showing the transition. A snapshot costs one frame to
        /// take, holds still (see ``coverForKeyboard(duration:options:)``),
        /// and comes off once the terminal has been resized for the move and
        /// the pane has stopped repainting underneath it. What happens in
        /// between is nobody's business.
        private var keyboardCover: UIView?
        private var coverRelease: DispatchWorkItem?
        /// When output last landed (`CACurrentMediaTime`).
        private var lastFedAt: CFTimeInterval = 0
        /// When the terminal last took a new size from its container: at the
        /// end of a keyboard move (``unlockFrame()``), or when a resize held
        /// for its repaint is applied (``applyDeferredResize()``). The remote
        /// repaints for the new size after it, so the cover's quiet beat
        /// counts from here too.
        private var resizedAt: CFTimeInterval = 0
        /// When the cover that is up comes off whatever the pane does:
        /// ``coverCap`` after it went up. Fixed then, not recomputed by each
        /// later check — a deadline that moved with every check never came
        /// while output kept the pane from going quiet, and an agent's spinner
        /// repaints every ~100 ms: the frozen picture stayed up for as long as
        /// the agent ran.
        private var coverDeadline: CFTimeInterval = 0

        /// Quiet this long under the cover and the pane is done repainting.
        private static let coverQuiet: TimeInterval = 0.15
        /// …but never hold the cover longer than this, counted from when it
        /// went up, whatever the pane does (an agent that streams forever must
        /// not be hidden forever).
        private static let coverCap: TimeInterval = 1.6

        /// Put the cover up (or keep the existing one) for a keyboard move.
        ///
        /// The snapshot does not MOVE, and that is a deliberate correction.
        /// Sliding it to track the container's bottom edge — the Messages
        /// motion, and the one that was asked for — assumes the content is
        /// bottom-anchored. A TUI's is not: Claude Code pins its footer to
        /// the bottom but keeps the conversation at the top with blank filler
        /// between, so bottom-aligning the old picture slid that blank filler
        /// into view and looked exactly like the content vanishing (verified
        /// frame by frame — the first attempt at this made the flash worse,
        /// not better). No rigid transform can stand in for a reflow, so the
        /// cover holds still and cross-fades to the settled screen instead:
        /// no flicker, and no motion that lies about what happened.
        func coverForKeyboard(duration: TimeInterval, options: UIView.AnimationOptions) {
            coverRequests += 1
            guard let terminalView = withSurface({ $0.view }), let container = hostContainer
            else { return }
            if keyboardCover == nil {
                // The terminal's own picture (the core draws one: a Metal
                // layer's snapshot is not to be trusted).
                guard let snapshot = withSurface({ $0.snapshotView() }) ?? nil else { return }
                // A backing plate under the snapshot, filling the container.
                // When the keyboard LEAVES, the container grows past what the
                // snapshot covers, and the strip below it would show the live
                // pane mid-redraw — the very thing the cover exists to hide.
                // Flat background there instead.
                let plate = UIView(frame: container.bounds)
                plate.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                plate.backgroundColor = terminalView.backgroundColor ?? .black
                plate.isUserInteractionEnabled = false
                snapshot.frame = terminalView.frame
                plate.addSubview(snapshot)
                container.addSubview(plate)
                keyboardCover = plate
                coverDeadline = CACurrentMediaTime() + Self.coverCap
            }
            _ = (duration, options)
            // Not when the keyboard's animation ends. The terminal takes its
            // new size only when the move is over — `unlockFrame()`, at
            // didChangeFrame or the lock's timeout, 0.4–0.52 s in on an iPad
            // after a 0.25 s animation — and the remote repaints after that.
            // A cover that came off at `duration` showed the reflow and the
            // repaint it was there to hide (spike G5: 24 of 24 transitions
            // jumped in the open; 0 of 24 with the cover waiting for the
            // resize, off ~670 ms in). The checks wait for the resize; a
            // further move keeps the deadline the cover went up with.
            scheduleCoverRelease(after: Self.coverQuiet)
        }

        /// Take the cover down once the terminal has its size for the move
        /// and the pane has been quiet for a beat since, or at
        /// ``coverDeadline``, whichever comes first.
        private func scheduleCoverRelease(after delay: TimeInterval) {
            coverRelease?.cancel()
            guard keyboardCover != nil else { return }
            let work = DispatchWorkItem { [weak self] in
                guard let self, self.keyboardCover != nil else { return }
                let now = CACurrentMediaTime()
                let left = self.coverDeadline - now
                guard left > 0 else { return self.releaseKeyboardCover() }
                // Not before the terminal has the size the move left it…
                if self.hostContainer?.resizePending == true {
                    return self.scheduleCoverRelease(after: min(Self.coverQuiet, left))
                }
                // …and not before a quiet beat after that resize and after the
                // last output: the repaint the resize asks for is in hand.
                let quietFor = now - max(self.lastFedAt, self.resizedAt)
                if quietFor < Self.coverQuiet {
                    return self.scheduleCoverRelease(after: min(Self.coverQuiet - quietFor, left))
                }
                self.releaseKeyboardCover()
            }
            coverRelease = work
            DispatchQueue.main.asyncAfter(deadline: .now() + max(delay, 0.01), execute: work)
        }

        /// How many keyboard moves have been judged worth covering. The
        /// DECISION is what matters and what the test pins; whether a
        /// snapshot could actually be taken depends on having a rendered
        /// window, which a test host does not.
        private(set) var coverRequests = 0

        func releaseKeyboardCover() {
            coverRelease?.cancel()
            coverRelease = nil
            guard let cover = keyboardCover else { return }
            keyboardCover = nil
            UIView.animate(withDuration: 0.18, animations: { cover.alpha = 0 },
                           completion: { _ in cover.removeFromSuperview() })
        }

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

        /// See ``TerminalHostContainer/shouldDeferResize``. Installed by the
        /// owner of the transport; nil means resizes apply immediately.
        var shouldDeferResize: ((_ cols: Int, _ rows: Int) -> Bool)? {
            didSet { hostContainer?.shouldDeferResize = shouldDeferResize }
        }

        /// Apply a held-back resize, immediately before feeding its repaint.
        func applyDeferredResize() {
            if hostContainer?.applyDeferredResize() == true {
                resizedAt = CACurrentMediaTime()
            }
        }

        /// Let the terminal adopt the container's (now settled) bounds — one
        /// resize, one reflow, one remote size report. Idempotent.
        func unlockFrame() {
            frameLockTimeout?.cancel()
            frameLockTimeout = nil
            guard let hostContainer else { return }
            hostContainer.frameLocked = false
            // The resize now rather than at the next layout pass, so the
            // cover's quiet beat starts after it and the repaint it asks for.
            hostContainer.layoutIfNeeded()
            resizedAt = CACurrentMediaTime()
            if keyboardCover != nil {
                scheduleCoverRelease(after: Self.coverQuiet)
            }
        }

        /// Cover the terminal with its background colour — used when switching
        /// TO this pane, whose buffer may hold stale content until the resync
        /// frame arrives. A deliberate clean veil reads as a transition; a beat
        /// of garbled stale text reads as a bug.
        func veilForSwitch() {
            #if DEBUG
            // The director's positive control: a switch with nothing covering
            // it, so its repaint shows (`-MOSHPIT_DEBUG_NO_VEIL 1`).
            if UserDefaults.standard.bool(forKey: "MOSHPIT_DEBUG_NO_VEIL") { return }
            #endif
            guard let tv = withSurface({ $0.view }) else { return }
            if transitionCover == nil {
                let veil = UIView()
                veil.backgroundColor = tv.backgroundColor ?? .black
                veil.frame = tv.bounds
                veil.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                veil.isUserInteractionEnabled = false
                veil.accessibilityIdentifier = Self.veilIdentifier
                tv.addSubview(veil)
                transitionCover = veil
                veiledAt = CACurrentMediaTime()
            }
            scheduleCoverTimeout()
        }

        /// The app came back (B386): a veil up longer than `age` is stuck
        /// (its safety cap is under a second, each resync pass extends it a
        /// little), and so is one in this terminal that this coordinator
        /// does not hold (another coordinator's, gone): both come off, and
        /// the log says so. Whether one came off.
        @discardableResult
        func liftStaleVeil(olderThan age: TimeInterval, now: CFTimeInterval = CACurrentMediaTime()) -> Bool {
            var lifted = false
            if transitionCover != nil, now - veiledAt > age {
                NSHDiagnostics.error("terminal: a pane-switch veil was still up \(Int(now - veiledAt)) s after it went up; lifted")
                reveal()
                lifted = true
            }
            guard let tv = withSurface({ $0.view }) else { return lifted }
            for orphan in tv.subviews where orphan.accessibilityIdentifier == Self.veilIdentifier
                && orphan !== transitionCover {
                NSHDiagnostics.error("terminal: a pane-switch veil no coordinator holds covered the terminal; removed")
                orphan.removeFromSuperview()
                lifted = true
            }
            return lifted
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
            // Told whether or not a cover was up: a reveal means "the frame
            // on this pane is the one to look at", and the tmux controller
            // reports the first of those per attach to the hub.
            defer { onReveal?() }
            guard let cover = transitionCover else { return }
            transitionCover = nil
            UIView.animate(withDuration: 0.12, animations: { cover.alpha = 0 },
                           completion: { _ in cover.removeFromSuperview() })
        }

        /// Called on every ``reveal()`` — see there.
        var onReveal: (() -> Void)?

        /// Makes `surface` this coordinator's terminal: owned here, wired
        /// both ways (its delegate, the keyboard's moves, the path links and
        /// predictor it should have), and handed whatever output arrived
        /// before it, in order. Idempotent: a remount re-wires the same one.
        /// The representable and the NSH stage attach the plain path's; the
        /// tmux controllers attach each pane's when they mint it.
        func attach(_ surface: TerminalSurface) {
            let isNew = self.surface !== surface
            self.surface = surface
            MainActor.assumeIsolated {
                surface.surfaceDelegate = self
                surface.pathLinks = pathLinks
                if isNew, prediction != nil { surface.attachPrediction(prediction) }
                observeKeyboard(of: surface.view)
                if !pendingFeed.isEmpty {
                    // One feed of everything, in order: the same bytes as
                    // the chunks one by one, one pass of the fix-ups after.
                    let held = pendingFeed.reduce(into: Data()) { $0.append($1) }
                    pendingFeed.removeAll()
                    surface.feed(held)
                }
            }
        }

        /// The input-method switch and the keyboard's moves, for whichever
        /// view is this coordinator's terminal (SwiftTerm's or the core's).
        func observeKeyboard(of terminalView: UIView) {
            // Switching the input method (globe key) is INVISIBLE to every
            // path we had: the CN/EN keyboards are the same height, so no
            // sizeChanged fires. Listen for the switch itself and let the owner
            // resync from the authoritative model (tmux re-captures the pane),
            // which repaints whatever cells really diverged. No full repaint
            // of our own: a composition is drawn in a subview of its own (fork
            // patch 5), not into the terminal's rows, so redrawing the same
            // rows from the same buffer changed nothing but the time it took.
            if let inputModeObserver {
                NotificationCenter.default.removeObserver(inputModeObserver)
            }
            inputModeObserver = NotificationCenter.default.addObserver(
                forName: UITextInputMode.currentInputModeDidChangeNotification,
                object: nil, queue: .main
            ) { [weak self, weak terminalView] _ in
                guard let terminalView, terminalView.isFirstResponder else { return }
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
                let curve = (note.userInfo?[UIResponder.keyboardAnimationCurveUserInfoKey] as? Int)
                    .flatMap(UIView.AnimationCurve.init(rawValue:)) ?? .easeInOut
                let options = UIView.AnimationOptions(rawValue: UInt(curve.rawValue) << 16)
                self?.lockFrame(for: duration)
                // Only cover a move that actually changes how much room the
                // terminal has. Switching input method (globe key) fires this
                // notification with the SAME frame on both ends — CN and EN
                // are the same height — and covering that costs the user
                // half a second of frozen picture to hide a transition that
                // never happens ("切换输入法有点慢", report). A begin frame is
                // not always supplied; when it isn't, assume it changed.
                let begin = (note.userInfo?[UIResponder.keyboardFrameBeginUserInfoKey]
                             as? NSValue)?.cgRectValue
                let end = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey]
                           as? NSValue)?.cgRectValue
                if let begin, let end, abs(begin.minY - end.minY) < 1,
                   abs(begin.height - end.height) < 1 {
                    return
                }
                self?.coverForKeyboard(duration: duration, options: options)
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
        }

        // MARK: Upstream: keys, a paste, the terminal's replies

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

        /// What the terminal sends upstream (keys, a paste, its replies), from
        /// either view.
        func sendInput(_ data: ArraySlice<UInt8>) {
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
            // No repaint of our own: what a key changes on screen arrives as its
            // echo, and SwiftTerm repaints just the rows that changed (fork
            // patch 27). This used to force a FULL repaint on every key so a
            // stale composing overlay couldn't linger after an IME commit; the
            // composition is a subview of its own now (fork patch 5), gone the
            // moment it ends, and the forced repaint cost a whole-screen draw
            // per keystroke (~5 ms on a 70×72 iPhone grid).
            let isLoneEraseByte = data.count == 1 && (data.first == 0x7f || data.first == 0x08)
            if isLoneEraseByte { noteEraseByte() }
            if data.contains(0x0d) || data.contains(0x0a) {
                onReturnSend?()
            }
            guard let onInput else { return }
            onInput(Data(data))
        }

        /// Where the terminal's own paste goes — the edit menu's Paste and
        /// ⌘V on a hardware keyboard: the session's paste path
        /// (`ActiveSession.sendPaste`), which decides bracketing per
        /// transport. With none, the paste is bracketed by the local
        /// emulator's flag, as SwiftTerm would — never set over SSH+tmux or
        /// SSH+herdr, so a multi-line paste ran line by line.
        var onPaste: ((String) -> Void)?

        /// The terminal's own paste (its edit menu, ⌘V): pasting means "I'm
        /// done reading", like typing does; then the session's paste path,
        /// or bracketed by the terminal's own mode with any marker inside
        /// taken out (``BracketedPaste``, V30) — never a view's own paste,
        /// which wraps the text as it is, so a marker inside ends it early.
        func paste(_ text: String) {
            releaseScrollHold()
            if let onPaste {
                onPaste(text)
                return
            }
            guard let payload = withSurface({ $0.pastePayload(text) }), !payload.isEmpty else { return }
            sendInput(ArraySlice([UInt8](payload)))
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

        /// The terminal's grid changed.
        func gridChanged(cols: Int, rows: Int) {
            lastReportedSize = (cols, rows)
            onSizeChange?(cols, rows)
        }

        /// A tapped link (OSC 8, or a URL the terminal recognised).
        func openLink(_ link: String) {
            // A tapped terminal hyperlink (OSC-8, or a plain URL when the build
            // surfaces one). Open only safe web/mail schemes — never arbitrary
            // custom schemes from remote output.
            let trimmed = link.trimmingCharacters(in: .whitespacesAndNewlines)
            // A host path is opened in the app, never handed to the system:
            // `file://` here names a file on the REMOTE machine.
            if let path = RemotePathLink.path(from: trimmed) {
                Log.input.info("path open \(path, privacy: .private)")
                let open = onOpenRemotePath
                Task { @MainActor in open?(path) }
                return
            }
            guard let url = URL(string: trimmed),
                  let scheme = url.scheme?.lowercased(),
                  ["http", "https", "mailto", "ftp", "ftps"].contains(scheme) else { return }
            // The observability seam the touch-matrix S-layer asserts on: the
            // EXACT address handed to the system, so a harness can prove a
            // wrapped link opened whole without scraping Safari.
            Log.input.info("link open \(url.absoluteString, privacy: .public)")
            Task { @MainActor in
                UIApplication.shared.open(url)
            }
        }

        /// OSC 52's read query — the remote is asking for the phone's
        /// clipboard. Gated on an off-by-default setting because this is an
        /// exfiltration channel: with it off we still ANSWER, with an empty
        /// clipboard (the conventional refusal), so a remote program waiting
        /// on the reply unblocks without learning anything.
        ///
        /// Text only for now; images wait on the ecosystem picking a
        /// protocol (kitty OSC 5522 — see Claude Code #42712). Pasteboard
        /// reads can block the main thread for over a second (see the paste
        /// chip), so the read happens off it.
        func answerClipboardRequest() {
            let allowed = AppSettings.shared.remoteClipboardReadEnabled
            Task { [weak self] in
                let text: String? = allowed ? await Task.detached { UIPasteboard.general.string }.value : nil
                await MainActor.run { _ = self?.withSurface { $0.answerClipboard(text) } }
            }
        }
    }
}

/// The terminal's events, routed as the session wants them.
@MainActor
extension SwiftTerminalView.Coordinator: TerminalSurfaceDelegate {
    func surface(_ surface: TerminalSurface, send data: Data) {
        sendInput(ArraySlice([UInt8](data)))
    }

    func surface(_ surface: TerminalSurface, sizeChanged cols: Int, rows: Int) {
        gridChanged(cols: cols, rows: rows)
    }

    func surface(_ surface: TerminalSurface, titleChanged title: String) {
        onTitleChange?(title)
    }

    func surface(_ surface: TerminalSurface, directoryChanged directory: String?) {
        onHostDirectoryChange?(directory)
    }

    func surfaceBell(_ surface: TerminalSurface) {
        onBell?()
    }

    func surface(_ surface: TerminalSurface, openLink link: String) {
        openLink(link)
    }

    func surface(_ surface: TerminalSurface, paste text: String) {
        paste(text)
    }

    func surfaceClipboardRequest(_ surface: TerminalSurface) {
        answerClipboardRequest()
    }

    func surface(_ surface: TerminalSurface, reached mark: TerminalMark) {
        reached(mark)
    }

    func surfaceScrollWillBegin(_ surface: TerminalSurface) {
        scrollWillBegin()
    }

    func surface(_ surface: TerminalSurface, scrollPixels dy: CGFloat) -> CGFloat {
        scroll(pixels: dy)
    }

    func surfaceScrollDidEnd(_ surface: TerminalSurface) {
        scrollDidEnd()
    }

    func surface(_ surface: TerminalSurface, click col: Int, row: Int) {
        click(col: col, row: row)
    }

    var surfaceScrollsElsewhere: Bool { onScroll != nil || onScrollPixels != nil }

    var surfaceCanSwitch: Bool { onSwitch != nil }

    func surface(_ surface: TerminalSurface, switchForward forward: Bool) {
        onSwitch?(forward)
    }

    func surface(_ surface: TerminalSurface, fontSizeCommitted size: Double) {
        onFontSizeCommit?(size)
    }
}
