import MoshpitFFI
import MoshpitKit
import QuartzCore
import Synchronization
import UIKit

/// A terminal view on the shared Rust terminal core (crates/term, our
/// wezterm-term fork; docs/nsh/TERMINAL-CORE.md): the T2 replacement for
/// the SwiftTerm fork's `TerminalView` (gone with batch B). App code
/// reaches it as a ``TerminalSurface`` (CoreTerminalView+Surface) and hears
/// from it as the surface's delegate.
///
/// - **Bytes** are pushed (``push(_:)``) into a byte channel of the core's,
///   parsed on the core's own thread in the order pushed, never dropped
///   (the iron rule of the byte path). A wake from that thread makes one
///   hop to the main thread, which takes a frame: copies of the rows that
///   changed. Resizes and input resets travel in the same queue, so each
///   lands between the bytes it came between; so do the owner's marks
///   (``feed(mark:)``), which come back with the frame that has the bytes
///   before them.
/// - **Drawing**: Metal (``CoreTermMetalCanvas``) re-encodes only the rows a
///   frame changed; CoreGraphics (``CoreTermCGCanvas``) is the fallback.
/// - **Input** (CoreTerminalView+TextInput, +Keys): the SwiftTerm fork's
///   input-method and keyboard rules, keys encoded by the core.
/// - **Scrolling and selection** (+Gestures): local history by the pixel,
///   the core's selection, links and search.
/// - **Accessibility** (+Accessibility): each row an element.
@MainActor
final class CoreTerminalView: UIView {
    weak var surfaceDelegate: TerminalSurfaceDelegate?

    /// The core's terminal (`MpTerm *`); nil only if the library failed.
    private(set) var term: OpaquePointer?
    let rows = CoreTermRows()
    private(set) var metrics: CoreTermMetrics
    var style: CoreTermStyle {
        didSet {
            guard style != oldValue else { return }
            canvas.setStyle(style)
            canvas.update(rows, changed: [], all: true)
            requestRender()
        }
    }
    private var canvas: CoreTermCanvas
    private let waker: Unmanaged<CoreTermWake>

    /// The grid the view asked the core for.
    private(set) var cols: Int
    private(set) var gridRows: Int
    let scrollbackLines: Int

    // Frame pacing.
    /// When the last frame a wake or the pacer took was applied (the end
    /// of its turn), and what it cost the main thread. The view's own
    /// frames (a scroll back to the live screen as a key goes out) do not
    /// count.
    private var lastFrameAt: CFTimeInterval = 0
    private var lastFrameCost: CFTimeInterval = 0
    private var lastRenderAt: CFTimeInterval = 0
    private var pacer: CADisplayLink?
    private var frameDue = false
    private var renderDue = false
    /// Frames the GPU has not finished.
    private let inFlight = CoreTermCounter()
    /// Whether the frames drawn reached the screen (B445).
    private let presentation = CoreTermPresentation()
    /// This view, for a frame's fate told on Metal's thread.
    private lazy var presentBox = CoreTermViewBox(self)
    /// The last frame's fate, looked for ``CoreTermPresentation/fateTimeout``
    /// after it was drawn.
    private var fateWatch: DispatchWorkItem?

    // The cursor's blink.
    private var blinkTimer: Timer?
    private(set) var cursorOn = true
    private var lastCursor: (col: UInt16, row: UInt16) = (0, 0)

    // Text input (CoreTerminalView+TextInput): the core's input model's
    // shape, as last read.
    var imeState = MpImeState(len: 0, sel_start: 0, sel_end: 0, marked_start: -1, marked_end: -1)
    weak var inputDelegate: UITextInputDelegate?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
    var markedLabel: UILabel?
    /// The cell the composition started at.
    var markedAnchor: (col: Int, row: Int)?
    /// The caret at the composition's end.
    lazy var compositionCaret: UIView = {
        let v = UIView()
        v.isUserInteractionEnabled = false
        return v
    }()
    /// The caret a commit leaves until the echo comes (patch 9).
    var pendingCaret: UIView?
    var pendingCaretFrom: (col: UInt16, row: UInt16) = (0, 0)
    /// The composition (or a commit's caret) owns the caret: the canvas
    /// draws none.
    var canvasCursorHidden = false {
        didSet {
            guard canvasCursorHidden != oldValue else { return }
            if !canvasCursorHidden {
                compositionCaret.removeFromSuperview()
                markedAnchor = nil
            }
            canvas.setCursorShown(cursorOn && !canvasCursorHidden)
            redrawCursorRow()
        }
    }
    var lastFloatingCursorLocation: CGPoint?
    var markedTextStyle: [NSAttributedString.Key: Any]?

    // Keys (CoreTerminalView+Keys).
    var keyRepeat: Timer?
    /// Keys whose press went out under the kitty protocol, for their release.
    var keysDown: [UIKeyboardHIDUsage: (key: UInt32, scalar: UInt32, mods: UInt8)] = [:]
    var commandKeys = Set<UIKeyboardHIDUsage>()

    // Gestures (CoreTerminalView+Gestures).
    let gestures = CoreTermGestureState()
    var handles: (start: CoreTermHandle, end: CoreTermHandle)?
    lazy var editMenu = UIEditMenuInteraction(delegate: self)
    /// The system's find bar over the core's find (CoreTerminalView+Find).
    private(set) var finder: UIFindInteraction?

    // The look last applied (CoreTerminalBinding).
    var appliedPalette: ([UInt32], UInt32, UInt32)?
    var appliedLook: TerminalLook?

    // The surface's state (CoreTerminalView+Surface).
    /// mosh's engine object: its mode and round trip (the guesses are the
    /// core's).
    var prediction: MoshPrediction?
    /// Which paths on this terminal the host has confirmed: they draw and
    /// tap as links (CoreTerminalView+Paths). nil: paths stay text.
    var pathLinks: RemotePathLinks? {
        didSet { pathLinksChanged() }
    }
    /// The next path pass, debounced.
    var pathPass: DispatchWorkItem?
    /// A frame the pass takes for its own links starts no pass.
    var pathPassFraming = false
    /// The terminal's own answers are on their way out
    /// (CoreTerminalView+Surface's `sendReply`).
    var sendingReply = false

    // Accessibility (CoreTerminalView+Accessibility).
    var accessibilityRows: [CoreTermRowElement] = []
    var accessibilityStale = true
    var lastAccessibilityPost: CFTimeInterval = 0
    /// The core asked to be asked again (what appeared in its quiet).
    var announceAgain: DispatchWorkItem?

    // Measurements: the debug probes read these.
    private(set) var framesTaken = 0
    private(set) var rowsEncoded = 0
    private(set) var renders = 0
    /// The longest a frame took on the main thread, take + encode (ms).
    private(set) var longestFrameMs: Double = 0
    /// Where the main thread's time per frame goes (ms, longest each):
    /// the core's events, one frame's rows applied and encoded, a render's
    /// submission; and the most frames one turn applied.
    private(set) var frameCosts = CoreTermFrameCosts()
    /// Called after every frame taken (debug probes: echo timing).
    var onFrame: (() -> Void)?

    /// A terminal of `font` whose grid fits `frame` (the birth grid: what
    /// the session seeds the remote with), `scrollback` lines of history.
    init(frame: CGRect, font: UIFont, scrollback: Int = TerminalScrollback.lines) {
        let scale = UITraitCollection.current.displayScale
        let metrics = CoreTermMetrics(font: font, scale: scale)
        self.metrics = metrics
        self.scrollbackLines = scrollback
        cols = max(1, Int(frame.width / metrics.width))
        gridRows = max(1, Int(frame.height / metrics.height))
        let style = CoreTermStyle()
        // CoreGraphics only where Metal is not.
        if let metal = CoreTermMetalCanvas(metrics: metrics, style: style) {
            canvas = metal
        } else {
            canvas = CoreTermCGCanvas(metrics: metrics, style: style)
        }
        self.style = style
        let wake = CoreTermWake()
        waker = Unmanaged.passRetained(wake)
        super.init(frame: frame)
        wake.view = self
        isOpaque = true
        // The terminal's own ground from birth, never black (B473): what
        // shows beside the grid before a theme is applied.
        backgroundColor = UIColor(cgColor: coreCGColor(style.background))
        clipsToBounds = true
        isMultipleTouchEnabled = true
        let canvasView = canvas.view
        canvasView.frame = canvasFrame
        addSubview(canvasView)
        rows.reset(cols: cols, rows: gridRows, background: style.background)
        term = mp_term_open_local(UInt16(clamping: cols), UInt16(clamping: gridRows), UInt32(clamping: scrollback),
                                  coreTermWake, waker.toOpaque())
        if term == nil { Log.ui.error("core term: the core refused a terminal") }
        applyCellPixels()
        canvas.update(rows, changed: [], all: true)
        CoreTermMemory.register(self)
        installGestures()
        let finder = UIFindInteraction(sessionDelegate: self)
        addInteraction(finder)
        self.finder = finder
        // The app draws its own shortcut bar, and no assistant bar either
        // (TerminalMint's rule for the SwiftTerm view).
        inputAssistantItem.leadingBarButtonGroups = []
        inputAssistantItem.trailingBarButtonGroups = []
        accessibilityTraits = [.allowsDirectInteraction]
    }

    required init?(coder: NSCoder) { fatalError("CoreTerminalView is made in code") }

    deinit {
        // The core's thread stops and calls the waker no more once this
        // returns; only then may the waker's box go.
        mp_term_close(term)
        waker.release()
    }

    /// Stops the frame pacer and the cursor's blink (they hold the view
    /// weakly; this only stops them early).
    override func removeFromSuperview() {
        super.removeFromSuperview()
        pacer?.isPaused = true
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        // Where the terminal is when it comes and goes, and whether the app
        // is up to draw then (B386: a blank terminal after the background).
        Log.ui.notice("core term: \(self.window == nil ? "out of" : "into", privacy: .public) a window, app \(Self.appState, privacy: .public), \(self.drawableSummary, privacy: .public)")
        if window == nil {
            blinkTimer?.invalidate()
            blinkTimer = nil
            pacer?.invalidate()
            pacer = nil
        } else {
            restartBlink()
            if let why = layerSuspect, canvas is CoreTermMetalCanvas,
               !CoreTermMemory.inBackground, UIApplication.shared.applicationState == .active {
                // B469: a layer with a reason not to show what it is given
                // (its drawables let go in the background) coming on screen
                // after the app is already active: the return's new canvas
                // (``cameToForeground()``) passed it by, off screen then.
                // Refitted, it dropped every frame until eight in a row made
                // a new canvas (Canary 124, 145 ms); a new one now.
                remakeCanvas(why: "into a window; \(why); \(drawableSummary)")
            } else {
                // Drawables the size of the view again, if they were let go.
                (canvas as? CoreTermMetalCanvas)?.fitDrawables()
                // Every row, whatever was drawn before it left (B445).
                redrawAll()
            }
            // Back on screen: its path links were held back while it was not.
            schedulePathPass()
        }
    }

    // MARK: Bytes in

    /// The remote's bytes: parsed on the core's thread after everything
    /// pushed before.
    func push(_ data: Data) {
        guard let term, !data.isEmpty else { return }
        data.withUnsafeBytes { raw in
            _ = mp_term_push(term, raw.baseAddress?.assumingMemoryBound(to: UInt8.self), raw.count)
        }
    }

    /// A reconnect: the next bytes are not the dead stream's continuation.
    func discardPendingInput() {
        guard let term else { return }
        mp_term_discard_pending_input(term)
    }

    /// A tmux pane's terminal: a `clear` keeps what tmux keeps (its
    /// `scroll-on-clear`, on by default, which this assumes).
    func followTmuxClears() {
        guard let term else { return }
        #if DEBUG
        // The director's A/B: `-MOSHPIT_TMUX_CLEARS 0` keeps a terminal's own rule.
        if UserDefaults.standard.string(forKey: "MOSHPIT_TMUX_CLEARS") == "0" { return }
        #endif
        mp_term_set_tmux_clears(term, true)
    }

    /// The modes the remote program set (mouse, bracketed paste, DECCKM,
    /// kitty keyboard), as the core has them.
    var termModes: MpModes {
        guard let term else { return MpModes() }
        return mp_term_modes(term)
    }

    // MARK: Frames

    /// The core's thread has a frame: called on the main thread, at most one
    /// hop outstanding.
    fileprivate func woke() {
        waker.takeUnretainedValue().queued.store(false, ordering: .sequentiallyConsistent)
        // A frame that comes before the main thread has rested as long as
        // the last one took (full screens back to back: a burst, a
        // backlog) waits for the display's refresh, one a refresh: the
        // main thread applied them back to back and missed refreshes
        // (TERMINAL-VIEW.md 5.6); what the core parses meanwhile is in the
        // next. A cheap one (a key's echo, a prompt) is taken at once, even
        // right after another (on a tmux pane every echo came within a
        // refresh of a frame before it, and waited a refresh).
        if CACurrentMediaTime() - lastFrameAt < min(lastFrameCost, refreshInterval) {
            frameDue = true
            resumePacer()
            return
        }
        takeFrame(fresh: false)
    }

    /// Takes what the core has: its replies and events, then its frames,
    /// each applied (only its changed rows re-encoded) and its marks met,
    /// in order; draws.
    ///
    /// The core's thread frames what it parses itself (at the end of each
    /// pass, and about every display refresh while a backlog catches up)
    /// and publishes it; a wake takes only that (`fresh` false), so the
    /// main thread never waits for a parse. A change of the view's own (a
    /// scroll, a selection, a new look, typed keys) is framed now
    /// (`fresh`): the next frame in order, then the change.
    func takeFrame(fresh: Bool = true) {
        guard let term else { return }
        let start = CACurrentMediaTime()
        frameDue = false
        let replies = mp_term_take_replies(term)
        if replies.len > 0, let ptr = replies.ptr {
            sendReply(Data(bytes: ptr, count: replies.len))
        }
        mp_bytes_free(replies)
        let eventsAt = CACurrentMediaTime()
        takeEvents()
        frameCosts.note(events: CACurrentMediaTime() - eventsAt)
        var next = fresh ? mp_term_frame(term) : mp_term_take_frame(term)
        guard next != nil else { return }
        var changedAny = false
        var taken = 0
        while let frame = next {
            let applyAt = CACurrentMediaTime()
            if apply(frame) { changedAny = true }
            frameCosts.note(apply: CACurrentMediaTime() - applyAt)
            taken += 1
            // One at a time, so a mark's handler that frames a change of its
            // own takes the frames after this one in their order. A wake
            // applies one frame (one published meanwhile wakes the view
            // again, for the next refresh); a change of the view's own shows
            // now: the frame published before it, then it.
            next = fresh && taken < 2 ? mp_term_take_frame(term) : nil
        }
        let cursor = rows.info.cursor
        if cursor.col != lastCursor.col || cursor.row != lastCursor.row {
            lastCursor = (cursor.col, cursor.row)
            // A cursor that moves is shown solid until it rests.
            if !cursorOn { cursorOn = true; canvas.setCursorShown(!canvasCursorHidden) }
            restartBlink()
            // The echo of a commit: the real cursor takes over (patch 9).
            if pendingCaret != nil, (cursor.col, cursor.row) != pendingCaretFrom { dropPendingCaret() }
        }
        if changedAny {
            accessibilityStale = true
            announceChanges()
            if pathLinks != nil, !pathPassFraming { schedulePathPass() }
        }
        if markedLabel != nil { positionMarkedText() }
        if handles != nil { positionHandles() }
        frameCosts.note(turn: taken)
        let end = CACurrentMediaTime()
        if !fresh {
            lastFrameAt = end
            lastFrameCost = end - start
        }
        let took = (end - start) * 1000
        if took > longestFrameMs { longestFrameMs = took }
        if changedAny { updateGround() }
        requestRender()
        onFrame?()
    }

    // MARK: What lies outside the grid (B495)

    /// The margins the grid leaves (its columns centred, B473; its rows
    /// from the top) take what the grid shows at its edges: the colour most
    /// of its first and last column's cells have, else the core's live
    /// default background. They were the theme's background, while the
    /// grid's own could differ (a remote's OSC 11, a program painting every
    /// cell): dark strips either side of the grid.
    private func updateGround() {
        var edges: [UInt32] = []
        edges.reserveCapacity(rows.gridRows * 2)
        for row in rows.rows.prefix(rows.gridRows) where !row.cells.isEmpty {
            edges.append(row.cells[0].bg)
            edges.append(row.cells[row.cells.count - 1].bg)
        }
        let fallback = framesTaken > 0 ? rows.info.background : style.background
        let next = Self.ground(edges: edges, fallback: fallback)
        guard next != style.outside else { return }
        var s = style
        s.ground = next == s.background ? nil : next
        style = s
        backgroundColor = UIColor(cgColor: coreCGColor(next))
        // The page's chrome takes it too (B496).
        surfaceDelegate?.surfaceGroundChanged(self)
    }

    /// The colour most of `edges` have, if more than half do; else
    /// `fallback`.
    nonisolated static func ground(edges: [UInt32], fallback: UInt32) -> UInt32 {
        guard !edges.isEmpty else { return fallback }
        var counts: [UInt32: Int] = [:]
        for colour in edges { counts[colour, default: 0] += 1 }
        guard let (top, count) = counts.max(by: { $0.value < $1.value }), count * 2 > edges.count else {
            return fallback
        }
        return top
    }

    /// One frame into the rows and the canvas, then the marks its bytes
    /// reached: before it is drawn (a veil goes up over it, a reveal starts
    /// with it). Whether it changed rows.
    private func apply(_ frame: OpaquePointer) -> Bool {
        let decodeAt = CACurrentMediaTime()
        let (changed, shift) = rows.apply(frame)
        let marks = Self.marks(of: frame, count: rows.info.marks)
        mp_frame_free(frame)
        framesTaken += 1
        let encodeAt = CACurrentMediaTime()
        frameCosts.note(decode: encodeAt - decodeAt)
        canvas.update(rows, changed: changed, all: false, shift: shift)
        frameCosts.note(encode: CACurrentMediaTime() - encodeAt)
        rowsEncoded += changed.count
        for mark in marks { surfaceDelegate?.surface(self, reached: mark) }
        return !changed.isEmpty
    }

    /// The debug probes' per-run measurements start over.
    func resetFrameCosts() {
        frameCosts = CoreTermFrameCosts()
        longestFrameMs = 0
    }

    private static func marks(of frame: OpaquePointer, count: UInt32) -> [TerminalMark] {
        guard count > 0 else { return [] }
        var out: [TerminalMark] = []
        out.reserveCapacity(Int(count))
        var code: UInt32 = 0
        var arg: Int64 = 0
        for i in 0..<count where mp_frame_mark(frame, i, &code, &arg) {
            out.append(TerminalMark(code: code, arg: arg))
        }
        return out
    }

    /// A mark among the bytes: back with the frame that has every byte
    /// pushed before it (the core keeps it in the byte queue). Without a
    /// core, at once.
    func feed(mark: TerminalMark) {
        guard let term, mp_term_push_mark(term, mark.code, mark.arg) else {
            surfaceDelegate?.surface(self, reached: mark)
            return
        }
    }

    private func takeEvents() {
        guard let term else { return }
        let box = Unmanaged.passRetained(CoreTermEvents())
        mp_term_take_events(term, { ctx, kind, text in
            guard let ctx else { return }
            let events = Unmanaged<CoreTermEvents>.fromOpaque(ctx).takeUnretainedValue()
            events.list.append((kind, String(cString: text)))
        }, box.toOpaque())
        let events = box.takeRetainedValue().list
        for (kind, text) in events {
            switch Int(kind) {
            case Int(MP_TERM_EVENT_TITLE): surfaceDelegate?.surface(self, titleChanged: text)
            case Int(MP_TERM_EVENT_BELL): surfaceDelegate?.surfaceBell(self)
            case Int(MP_TERM_EVENT_CLIPBOARD_STORE):
                // OSC 52 writes: off, as on the SwiftTerm path (an app that
                // writes the phone's clipboard behind the user's back).
                break
            case Int(MP_TERM_EVENT_CLIPBOARD_LOAD): surfaceDelegate?.surfaceClipboardRequest(self)
            case Int(MP_TERM_EVENT_DIRECTORY):
                surfaceDelegate?.surface(self, directoryChanged: text.isEmpty ? nil : text)
            default: break
            }
        }
    }

    /// Answers the remote's OSC 52 query (nil: an empty clipboard, the
    /// conventional refusal); the reply goes with the terminal's others.
    func answerClipboard(_ text: String?) {
        guard let term else { return }
        if let text {
            text.withCString { mp_term_answer_clipboard(term, $0) }
        } else {
            mp_term_answer_clipboard(term, nil)
        }
        takeFrame()
    }

    // MARK: Drawing

    /// Draws now, or at the next display refresh when a picture drawn this
    /// refresh is not on screen yet: at most one a refresh, so the main
    /// thread never waits for a drawable (it did: pixel scrolling faster
    /// than a 60 Hz iPad mini refreshes spent 7 ms a step in nextDrawable).
    /// A key's echo after a pause draws at once.
    func requestRender() {
        guard window != nil || canvas is CoreTermCGCanvas else { renderDue = true; return }
        // Not while the app is in the background: the GPU refuses the frame
        // and the picture on screen is gone with it; coming back draws the
        // whole picture (``CoreTermMemory/foreground()``, B386).
        guard !CoreTermMemory.inBackground || canvas is CoreTermCGCanvas else { renderDue = true; return }
        let now = CACurrentMediaTime()
        // A picture that never reached the screen (the view left it) does
        // not hold drawing up.
        if inFlight.value > 0, now - lastRenderAt > 0.1 { inFlight.reset() }
        if now - lastRenderAt < refreshInterval * 0.75 || inFlight.value >= 1 {
            renderDue = true
            resumePacer()
            return
        }
        render()
    }

    /// The display's refresh interval.
    private var refreshInterval: CFTimeInterval {
        1.0 / Double(max(window?.screen.maximumFramesPerSecond ?? 60, 30))
    }

    private func render() {
        renderDue = false
        lastRenderAt = CACurrentMediaTime()
        defer { frameCosts.note(render: CACurrentMediaTime() - lastRenderAt) }
        canvas.yOffset = -renderShift
        if let metal = canvas as? CoreTermMetalCanvas {
            let counter = inFlight
            counter.add(1)
            if let refused = metal.takeGPUFailure() {
                // A frame before this one never reached the screen.
                layerSuspect = "the GPU refused a frame: \(refused)"
                if CoreTermMemory.shouldReportRefusal() {
                    NSHDiagnostics.error("core term: the GPU refused a frame (app \(Self.appState)): \(refused)")
                }
            }
            let frame = presentation.issue()
            let presentation = self.presentation, box = presentBox
            let drawn = metal.render(rows: rows, completion: { shown in
                counter.add(-1)
                if shown { return presentation.shown(frame) }
                DispatchQueue.main.async { MainActor.assumeIsolated { box.view?.frameNeverShown(frame) } }
            })
            if !drawn {
                counter.add(-1)
                cannotPresent(metal)
                renderDue = true
                resumePacer()
                return
            }
            fateWatch?.cancel()
            let watch = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, self.presentation.unresolved(frame) else { return }
                    self.frameNeverShown(frame)
                }
            }
            fateWatch = watch
            DispatchQueue.main.asyncAfter(deadline: .now() + CoreTermPresentation.fateTimeout, execute: watch)
            if failedRenders > 0 {
                Log.ui.notice("core term: presents again after \(self.failedRenders, privacy: .public) frames it could not")
                failedRenders = 0
            }
        } else {
            canvas.render(rows: rows)
        }
        renders += 1
    }

    /// Renders in a row that drew nothing (B386).
    private(set) var failedRenders = 0

    /// A frame drawn never reached the screen (B445): the layer dropped it
    /// or the GPU refused it. With nothing newer on its way, it is drawn
    /// again; an idle pane sends nothing that would, and its rows stayed
    /// unseen until something unrelated drew. After
    /// ``CoreTermPresentation/retryCap`` in a row, a new canvas.
    func frameNeverShown(_ frame: Int) {
        let active = window != nil && !CoreTermMemory.inBackground
            && UIApplication.shared.applicationState == .active
        switch presentation.neverReached(frame, active: active) {
        case .nothingLost:
            return
        case .drawAgain:
            if presentation.currentStreak == 1 {
                Log.ui.notice("core term: a frame never reached the screen (app \(Self.appState, privacy: .public), \(self.drawableSummary, privacy: .public)): drawing it again")
            }
            renderDue = true
            resumePacer()
        case .giveUp:
            NSHDiagnostics.error("core term: \(CoreTermPresentation.retryCap) frames in a row never reached the screen (app \(Self.appState), \(drawableSummary)): a new canvas")
            remakeCanvas(why: "frames never reached the screen")
        }
    }

    /// Frames that reached the screen (B445; tests, the stage's check).
    var framesShown: Int { presentation.shownFrames }

    /// Frames that never did (B445).
    var framesNeverShown: Int { presentation.framesNeverShown }

    /// A render drew nothing: said once a streak (and again every 120
    /// tries), and after a few the drawables are made anew, whatever held
    /// them (B386).
    private func cannotPresent(_ metal: CoreTermMetalCanvas) {
        failedRenders += 1
        if failedRenders == 1 || failedRenders.isMultiple(of: 120) {
            NSHDiagnostics.error("core term: can't present (\(metal.lastFailure ?? "?"), try \(failedRenders), app \(Self.appState), \(drawableSummary))")
        }
        // A layer whose drawables never come back: a new one.
        if metal.lastFailure?.hasPrefix("no drawable") == true, failedRenders.isMultiple(of: 3) {
            remakeCanvas(why: "no drawable for \(failedRenders) frames")
        }
    }

    /// Canvases this view has had (tests: a foreground makes a new one).
    private(set) var canvasesMade = 1

    /// ``renders`` and ``framesShown`` when the canvas there was made.
    private var rendersAtCanvas = 0
    private var shownAtCanvas = 0

    /// Frames on the screen from the canvas there (B445: the stage's
    /// check): those that reached it on Metal, those drawn on CoreGraphics.
    var framesSinceCanvas: Int? {
        canvas is CoreTermMetalCanvas ? framesShown - shownAtCanvas : renders - rendersAtCanvas
    }

    /// Every row encoded again and drawn (B445): what the stage asks
    /// whenever it puts this terminal in a slot. Nothing is drawn while the
    /// app is in the background (``requestRender()``); the return draws.
    func redrawAll() {
        canvas.update(rows, changed: [], all: true)
        fullRedraws += 1
        requestRender()
    }

    /// Times every row was encoded again for a whole redraw (tests).
    private(set) var fullRedraws = 0

    /// A new Metal canvas in place of the one there, everything encoded
    /// again from the rows (B386). What a redial cured, without one: a
    /// terminal blank after the background had its rows right (the keyboard
    /// cover, drawn from them by CoreGraphics, showed them) but a layer
    /// that showed nothing it was given, and a new terminal's new layer did.
    /// The old layer keeps its last picture until the new one has drawn.
    private func remakeCanvas(why: String) {
        guard canvas is CoreTermMetalCanvas, let fresh = CoreTermMetalCanvas(metrics: metrics, style: style) else { return }
        let old = canvas.view
        fresh.frame = canvasFrame
        insertSubview(fresh, belowSubview: old)
        canvas = fresh
        canvasesMade += 1
        rendersAtCanvas = renders
        shownAtCanvas = framesShown
        layerSuspect = nil
        canvas.setCursorShown(cursorOn && !canvasCursorHidden)
        canvas.update(rows, changed: [], all: true)
        fresh.fitDrawables()
        failedRenders = 0
        inFlight.reset()
        render()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { old.removeFromSuperview() }
        Log.ui.notice("core term: a new canvas (\(why, privacy: .public)), \(self.drawableSummary, privacy: .public)")
    }

    /// The app's state, for the log.
    static var appState: String {
        switch UIApplication.shared.applicationState {
        case .active: "active"
        case .inactive: "inactive"
        case .background: "background"
        @unknown default: "unknown"
        }
    }

    /// The drawables against the bounds, for the log.
    var drawableSummary: String {
        guard let metal = canvas as? CoreTermMetalCanvas else { return "cg canvas" }
        let have = metal.drawableSize, want = metal.fittedSize
        return "drawables \(Int(have.width))×\(Int(have.height)) for \(Int(want.width))×\(Int(want.height))"
            + (metal.released ? ", let go" : "")
    }

    /// The drawables fit the view and were made since they were last let
    /// go (tests, the foreground check).
    var drawablesFit: Bool { (canvas as? CoreTermMetalCanvas)?.drawablesFit ?? true }

    /// The drawables' size now, in pixels; nil on the CoreGraphics canvas
    /// (tests).
    var drawablePixelSize: CGSize? { (canvas as? CoreTermMetalCanvas)?.drawableSize }

    /// The app came back (B386): a terminal on screen draws again. On a new
    /// canvas (``remakeCanvas(why:)``) when its layer has a reason not to
    /// show what it is given (``layerSuspect``: drawables let go while the
    /// app was away, a frame the GPU refused, frames it could not present);
    /// otherwise on its own layer, one render: a layer swap on every return
    /// costs power and risks a flash for nothing.
    func cameToForeground() {
        guard window != nil, let metal = canvas as? CoreTermMetalCanvas else { return }
        if let refused = metal.takeGPUFailure() { layerSuspect = "the GPU refused a frame: \(refused)" }
        if failedRenders > 0 { layerSuspect = "\(failedRenders) frames it could not present" }
        guard let why = layerSuspect else {
            // A layer that never failed: its picture is still its own; one
            // render puts on it whatever came while the app was away.
            Log.ui.info("core term: foreground, layer fine, \(self.drawableSummary, privacy: .public)")
            return requestRender()
        }
        remakeCanvas(why: "foreground; \(why); \(drawableSummary)")
    }

    /// Why this view's layer may not show what it is given (B386): its
    /// drawables were let go while the app was in the background, the GPU
    /// refused one of its frames, or it could not present. A new canvas
    /// clears it (``remakeCanvas(why:)``); the app's return makes one only
    /// for a layer with a reason, the others just draw.
    private(set) var layerSuspect: String?

    /// Lets the drawables go wherever the view is (tests: what the
    /// background does to a view off screen, without moving it).
    func releaseDrawablesForTesting() {
        (canvas as? CoreTermMetalCanvas)?.releaseDrawables()
    }

    /// Draws now, past the pacer (tests).
    func renderNowForTesting() {
        inFlight.reset()
        render()
    }

    private func resumePacer() {
        if pacer == nil {
            let link = CADisplayLink(target: CoreTermPacerTarget(self), selector: #selector(CoreTermPacerTarget.tick))
            link.add(to: .main, forMode: .common)
            pacer = link
        }
        pacer?.isPaused = false
    }

    fileprivate func tick() {
        if frameDue { takeFrame(fresh: false) }
        if renderDue, inFlight.value < 1 || CACurrentMediaTime() - lastRenderAt > 0.1 {
            if inFlight.value > 0 { inFlight.reset() }
            render()
        }
        if !frameDue, !renderDue { pacer?.isPaused = true }
    }

    // MARK: The cursor's blink

    private func restartBlink() {
        blinkTimer?.invalidate()
        blinkTimer = nil
        guard style.cursorBlink, window != nil else {
            if !cursorOn { cursorOn = true; canvas.setCursorShown(!canvasCursorHidden); redrawCursorRow() }
            return
        }
        let timer = Timer(timeInterval: 0.53, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.cursorOn.toggle()
                self.canvas.setCursorShown(self.cursorOn && !self.canvasCursorHidden)
                self.redrawCursorRow()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        blinkTimer = timer
    }

    func redrawCursorRow() {
        let r = Int(rows.info.cursor.row)
        guard r < rows.rows.count else { return }
        canvas.update(rows, changed: [r], all: false)
        requestRender()
    }

    // MARK: Geometry

    /// The cell in points.
    var cellSize: CGSize { CGSize(width: metrics.width, height: metrics.height) }

    /// The grid this view's bounds hold.
    var grid: TerminalGrid { TerminalGrid(cols: cols, rows: gridRows) }

    /// A new size takes effect at once, not at the next layout pass: a
    /// resize held for its repaint is applied by setting the frame right
    /// before the repaint's bytes are pushed, and the core must resize
    /// between the two (SwiftTerm's view does the same in its frame setter).
    override var frame: CGRect {
        didSet { if frame.size != oldValue.size { syncGrid() } }
    }

    override var bounds: CGRect {
        didSet { if bounds.size != oldValue.size { syncGrid() } }
    }

    /// The grid the bounds hold, to the core (after the bytes pushed so far)
    /// and to the owner.
    private func syncGrid() {
        guard bounds.width > 0, bounds.height > 0, term != nil else { return }
        let c = max(1, Int(bounds.width / metrics.width))
        let r = max(1, Int(bounds.height / metrics.height))
        guard c != cols || r != gridRows else { return }
        cols = c
        gridRows = r
        if let term { mp_term_resize(term, UInt16(clamping: c), UInt16(clamping: r)) }
        applyCellPixels()
        canvas.view.frame = canvasFrame
        surfaceDelegate?.surface(self, sizeChanged: c, rows: r)
    }

    // MARK: Where the grid sits across the view (B473)

    /// Where the grid starts across the view: the width its columns leave
    /// over, split evenly on both sides (to a pixel), so the grid
    /// isn't shoved left with the whole leftover on the right. Every cell
    /// to point mapping goes through it (``cell(at:)``, ``edge(at:)``,
    /// ``x(ofColumn:)``, the canvas, the snapshot).
    var gridOriginX: CGFloat {
        Self.gridOrigin(width: bounds.width, cols: cols, cellWidth: metrics.width, scale: metrics.scale)
    }

    /// The grid's left edge in a view `width` wide holding `cols` columns
    /// of `cellWidth`: half what they leave over, to the nearest pixel at
    /// `scale` (the two sides differ by a pixel at most).
    nonisolated static func gridOrigin(width: CGFloat, cols: Int, cellWidth: CGFloat, scale: CGFloat) -> CGFloat {
        let spare = width - CGFloat(cols) * cellWidth
        guard spare > 0, scale > 0 else { return 0 }
        return min(spare, ((spare / 2) * scale).rounded() / scale)
    }

    /// The column under `x` (the view's coordinates) for a grid starting at
    /// `originX`, clamped to the grid.
    nonisolated static func column(atX x: CGFloat, originX: CGFloat, cellWidth: CGFloat, cols: Int) -> Int {
        guard cellWidth > 0 else { return 0 }
        return min(max(0, Int(((x - originX) / cellWidth).rounded(.down))), max(0, cols - 1))
    }

    /// Where column `col` starts, in the view.
    func x(ofColumn col: Int) -> CGFloat { gridOriginX + CGFloat(col) * metrics.width }

    /// The canvas: from the grid's left edge to the view's right one (the
    /// gutter left of it is the view's own ground, the theme's background).
    var canvasFrame: CGRect {
        let origin = gridOriginX
        return CGRect(x: origin, y: 0, width: max(0, bounds.width - origin), height: bounds.height)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard bounds.width > 0, bounds.height > 0 else {
            canvas.view.frame = bounds
            return
        }
        syncGrid()
        canvas.view.frame = canvasFrame
        if markedLabel != nil { positionMarkedText() }
        if handles != nil { positionHandles() }
        accessibilityStale = true
        if bounds.size != laidOutSize {
            // A new size (the first layout, the keyboard, a rotation): every
            // row encoded again and drawn into drawables of that size, not
            // only the rows the next bytes change (B445: a first open's one
            // full frame came before the first layout, and an idle pane
            // sends no other).
            laidOutSize = bounds.size
            redrawAll()
        } else {
            requestRender()
        }
    }

    /// The size the last layout gave (B445).
    private var laidOutSize: CGSize = .zero

    private func applyCellPixels() {
        guard let term else { return }
        mp_term_set_cell_pixels(term, UInt16(clamping: metrics.widthPx), UInt16(clamping: metrics.heightPx))
    }

    /// A new font: new cells, a new grid for the same bounds.
    func apply(font: UIFont) {
        guard font.fontName != metrics.font.fontName || font.pointSize != metrics.font.pointSize else { return }
        let scale = window?.screen.scale ?? UITraitCollection.current.displayScale
        metrics = CoreTermMetrics(font: font, scale: scale)
        canvas.setMetrics(metrics)
        // The core's cells take the new pixels even when the grid keeps its
        // columns and rows; a history reader stays on its line (B479: the
        // view went to the bottom at every step of a pinch).
        applyCellPixels()
        canvas.update(rows, changed: [], all: true)
        takeFrame()
        setNeedsLayout()
        layoutIfNeeded()
        requestRender()
    }

    var font: UIFont { metrics.font }

    /// The viewport cell under `point` (this view's coordinates), clamped.
    func cell(at point: CGPoint) -> (col: Int, row: Int) {
        let y = point.y + renderShift
        let col = Self.column(atX: point.x, originX: gridOriginX, cellWidth: metrics.width, cols: rows.cols)
        let row = min(max(0, Int(y / metrics.height)), max(0, rows.gridRows - 1))
        return (col, row)
    }

    /// The nearest edge between cells to `point` (a drag's selection end,
    /// fork patch 55): a cell, and whether the edge is after it.
    func edge(at point: CGPoint) -> (col: Int, row: Int, after: Bool) {
        let y = point.y + renderShift
        let row = min(max(0, Int(y / metrics.height)), max(0, rows.gridRows - 1))
        let e = min(max(0, Int(((point.x - gridOriginX) / metrics.width).rounded())), rows.cols)
        return e >= rows.cols ? (rows.cols - 1, row, true) : (e, row, false)
    }

    /// Where viewport row `row` (negative: above) starts, in this view.
    func y(ofRow row: Int) -> CGFloat { CGFloat(row) * metrics.height - renderShift }

    // MARK: Scrolling: the core's scroll model

    /// Rows are drawn this many points up: the part of a row a pixel scroll
    /// leaves above the view (the core's `pixel_shift`).
    private var renderShift: CGFloat { CGFloat(rows.info.pixel_shift) / metrics.scale }

    /// Lines of history the core keeps.
    var historyLines: Int {
        guard let term else { return 0 }
        var history: UInt32 = 0
        mp_term_scroll_state(term, nil, &history)
        return Int(history)
    }

    var isReadingScrollback: Bool { rows.info.display_offset > 0 }

    /// A drag of `dy` points (positive: older). The core decides where it
    /// goes (V20): the host's rows (returned, when `hostRows`), wheel ticks
    /// for a program that wants the mouse (sent from here), or this
    /// terminal's history by the pixel. Returns the points used.
    func drag(_ dy: CGFloat, hostRows: Bool) -> (moved: CGFloat, rows: Int) {
        guard let term, dy != 0, dy.isFinite else { return (0, 0) }
        return perform(mp_term_scroll_drag(term, Double(dy * metrics.scale), hostRows), dy: dy)
    }

    private func perform(_ action: MpScrollAction, dy: CGFloat) -> (moved: CGFloat, rows: Int) {
        defer { mp_bytes_free(action.bytes) }
        switch action.kind {
        case 1:
            takeFrame()
            return (CGFloat(action.moved_px) / metrics.scale, 0)
        case 2:
            if let p = action.bytes.ptr { surfaceDelegate?.surface(self, send: Data(bytes: p, count: action.bytes.len)) }
            return (dy, 0)
        case 3:
            return (dy, Int(action.rows))
        default:
            return (0, 0)
        }
    }

    /// Moves this terminal's history `dy` points (the coordinator's local
    /// route); the distance moved.
    @discardableResult
    func scrollViewport(by dy: CGFloat) -> CGFloat {
        drag(dy, hostRows: false).moved
    }

    /// Whole lines (positive: older).
    func scrollLines(_ lines: Int) {
        _ = drag(CGFloat(lines) * metrics.height, hostRows: false)
    }

    /// A drag came to rest: within half a row of the live screen docks onto
    /// it (V19).
    func settleScroll() {
        guard let term else { return }
        _ = perform(mp_term_scroll_end(term), dy: 0)
    }

    /// Back to the live screen.
    func scrollToBottom() {
        guard let term else { return }
        mp_term_scroll_to_bottom(term)
        takeFrame()
    }

    // MARK: The mouse, for a program that asked for it

    /// Wheel ticks at the grid's centre (positive: older), at most six.
    func sendWheel(lines: Int) {
        guard let term, lines != 0 else { return }
        let count = Int(min(lines.magnitude, 6))
        let row = UInt16(clamping: max(0, gridRows / 2)), col = UInt16(clamping: max(0, cols / 2))
        var out = Data()
        for _ in 0..<count {
            let b = mp_term_mouse(term, 0, lines > 0 ? 4 : 5, row, col, 0)
            if let p = b.ptr { out.append(p, count: b.len) }
            mp_bytes_free(b)
        }
        if !out.isEmpty { surfaceDelegate?.surface(self, send: out) }
    }

    /// A left click at a viewport cell: press and release, as the program
    /// asked them encoded; nothing when it asked for no reports.
    func clickLocal(col: Int, row: Int) {
        guard let term else { return }
        var out = Data()
        for kind: UInt8 in [0, 1] {
            let b = mp_term_mouse(term, kind, 0, UInt16(clamping: row), UInt16(clamping: col), 0)
            if let p = b.ptr { out.append(p, count: b.len) }
            mp_bytes_free(b)
        }
        if !out.isEmpty { surfaceDelegate?.surface(self, send: out) }
    }

    // MARK: Pictures

    /// A still picture of the terminal as it shows (the keyboard cover).
    func snapshot() -> UIView {
        let renderer = CoreTermCGRenderer(metrics: metrics, style: style)
        let image = renderer.image(of: rows, size: bounds.size, xOffset: gridOriginX, yOffset: renderShift,
                                   cursorShown: cursorOn)
        let view = UIImageView(image: image)
        view.frame = frame
        return view
    }

    /// The current frame drawn offscreen (Metal) as BGRA: render-and-compare
    /// tests. nil on the CoreGraphics canvas.
    func renderedPixels() -> (pixels: [UInt8], width: Int, height: Int)? {
        (canvas as? CoreTermMetalCanvas)?.renderOffscreen(rows: rows, size: bounds.size)
    }

    /// The app goes to the background: the images this terminal's emulator
    /// keeps only to share identical ones are let go. Bytes freed.
    @discardableResult
    func releaseImageCaches() -> Int {
        guard let term else { return 0 }
        return mp_term_release_image_caches(term)
    }

    /// A memory warning: that, and this terminal's kitty images no cell
    /// shows any more. Bytes freed.
    func dropUnplacedImages() -> Int {
        guard let term else { return 0 }
        return mp_term_drop_unplaced_images(term)
    }

    /// Bytes this terminal's stored kitty images take (debug probes).
    var kittyImageBytes: Int {
        guard let term else { return 0 }
        return mp_term_kitty_image_bytes(term)
    }

    /// What this view keeps of images (bytes, debug probes): the decoded
    /// pixels its rows draw from, and the textures made from them.
    var viewImageMemory: (rows: Int, textures: Int, textureCount: Int) {
        let t = (canvas as? CoreTermMetalCanvas)?.textureMemory ?? (count: 0, bytes: 0)
        return (rows.imageBytes, t.bytes, t.count)
    }

    /// Which canvas draws: "metal" or "cg".
    var canvasKind: String { canvas is CoreTermMetalCanvas ? "metal" : "cg" }

    /// Off screen: the canvas's drawables go (they come back with the
    /// window). True if it had any to let go.
    @discardableResult
    func releaseDrawablesIfOffScreen() -> Bool {
        guard window == nil, let metal = canvas as? CoreTermMetalCanvas else { return false }
        metal.releaseDrawables()
        if CoreTermMemory.inBackground { layerSuspect = "drawables let go in the background" }
        return true
    }

    /// The glyph atlas's size and what it warmed (debug probes).
    var atlasSummary: String { (canvas as? CoreTermMetalCanvas)?.atlas.summary ?? "no atlas (cg)" }

    /// Waits (tests) until the core has parsed everything pushed and a frame
    /// with it is taken, or `timeout` passes.
    func settle(timeout: TimeInterval = 5, until done: (CoreTerminalView) -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            takeFrame()
            if done(self) { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.005))
        }
        return false
    }
}

/// What the canvases have in common.
@MainActor
protocol CoreTermCanvas: AnyObject {
    var view: UIView { get }
    var yOffset: CGFloat { get set }
    func setStyle(_ style: CoreTermStyle)
    func setMetrics(_ metrics: CoreTermMetrics)
    func setCursorShown(_ shown: Bool)
    func update(_ rows: CoreTermRows, changed: [Int], all: Bool, shift: Int)
    func render(rows: CoreTermRows)
}

extension CoreTermCanvas {
    func update(_ rows: CoreTermRows, changed: [Int], all: Bool) {
        update(rows, changed: changed, all: all, shift: 0)
    }
}

extension CoreTermMetalCanvas: CoreTermCanvas {
    var view: UIView { self }
    func setStyle(_ style: CoreTermStyle) { self.style = style }
    func setCursorShown(_ shown: Bool) { cursorShown = shown }
    func render(rows: CoreTermRows) { render(rows: rows, completion: nil) }
}

/// The CoreGraphics fallback: invalidates the rows a frame changed and draws
/// them in `draw(_:)` (iOS 27 hands `draw` the whole bounds whatever was
/// invalidated, fork patch 26: the rows to draw are kept here).
@MainActor
final class CoreTermCGCanvas: UIView, CoreTermCanvas {
    private var renderer: CoreTermCGRenderer
    private weak var source: CoreTermRows?
    private var dirty = IndexSet()
    private var all = true
    private var images: [UInt64: CGImage] = [:]
    var yOffset: CGFloat = 0 {
        didSet { if yOffset != oldValue { all = true; setNeedsDisplay() } }
    }
    private var cursorShown = true

    init(metrics: CoreTermMetrics, style: CoreTermStyle) {
        renderer = CoreTermCGRenderer(metrics: metrics, style: style)
        super.init(frame: .zero)
        isOpaque = true
        isUserInteractionEnabled = false
        contentMode = .topLeft
    }

    required init?(coder: NSCoder) { fatalError("CoreTermCGCanvas is made in code") }

    var view: UIView { self }
    func setStyle(_ style: CoreTermStyle) { renderer.style = style; all = true }
    func setMetrics(_ metrics: CoreTermMetrics) { renderer = CoreTermCGRenderer(metrics: metrics, style: renderer.style); all = true }
    func setCursorShown(_ shown: Bool) { cursorShown = shown }

    func update(_ rows: CoreTermRows, changed: [Int], all everything: Bool, shift: Int) {
        source = rows
        if everything || shift != 0 { all = true }
        dirty.formUnion(IndexSet(changed))
    }

    func render(rows: CoreTermRows) {
        source = rows
        if all {
            setNeedsDisplay()
            return
        }
        let h = renderer.metrics.height
        for r in dirty {
            setNeedsDisplay(CGRect(x: 0, y: CGFloat(r) * h + yOffset - 2, width: bounds.width, height: h + 4))
        }
    }

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext(), let rows = source else { return }
        let h = renderer.metrics.height
        let toDraw: [Int] = all ? Array(rows.rows.indices) : dirty.sorted()
        if all {
            ctx.setFillColor(coreCGColor(renderer.style.outside))
            ctx.fill(bounds)
        }
        for r in toDraw where r < rows.rows.count {
            renderer.draw(row: r, of: rows, y: CGFloat(r) * h + yOffset, cursorShown: cursorShown, in: ctx,
                          cache: &images)
        }
        all = false
        dirty.removeAll()
    }
}

/// Where a view's main-thread time per frame goes, the longest of each
/// since the last reset (ms): what the debug probes print next to the
/// display's longest gap.
struct CoreTermFrameCosts {
    private(set) var events = 0.0
    private(set) var apply = 0.0
    /// The two halves of an apply: the frame's rows into the view's, and
    /// those rows into the canvas's instances.
    private(set) var decode = 0.0
    private(set) var encode = 0.0
    private(set) var render = 0.0
    /// The most frames one turn applied.
    private(set) var framesInTurn = 0

    mutating func note(events s: CFTimeInterval) { events = max(events, s * 1000) }
    mutating func note(apply s: CFTimeInterval) { apply = max(apply, s * 1000) }
    mutating func note(decode s: CFTimeInterval) { decode = max(decode, s * 1000) }
    mutating func note(encode s: CFTimeInterval) { encode = max(encode, s * 1000) }
    mutating func note(render s: CFTimeInterval) { render = max(render, s * 1000) }
    mutating func note(turn frames: Int) { framesInTurn = max(framesInTurn, frames) }

    var summary: String {
        String(format: "events %.1f, apply %.1f (decode %.1f, encode %.1f), render %.1f ms; frames a turn %d",
               events, apply, decode, encode, render, framesInTurn)
    }
}

/// Images' memory across every core terminal (TERMINAL-VIEW.md §5.8). The
/// core keeps one cache of decoded images for the process (64 MB); each
/// emulator keeps its last images so identical ones share data. Both are
/// let go on a memory warning and when the app goes to the background (the
/// next frame that draws an image decodes it again). On a memory warning
/// each terminal also lets go of the kitty images no cell shows any more.
/// Both times the views off screen (a tmux session's other panes) let their
/// drawables go: 22 MB each on an iPad mini, rebuilt when they show again
/// (TERMINAL-VIEW.md §5.9). What the view on screen has drawn stays.
@MainActor
enum CoreTermMemory {
    private static let views = NSHashTable<CoreTerminalView>.weakObjects()
    private static var observing = false

    /// Between the app going to the background and coming back: no frame
    /// is drawn then (B386).
    private(set) static var inBackground = false

    /// When the GPU's refusal of a frame was last logged: once a minute at
    /// most, however many frames it refuses.
    private static var refusalLoggedAt: CFTimeInterval = -.infinity

    static func shouldReportRefusal(now: CFTimeInterval = CACurrentMediaTime()) -> Bool {
        guard now - refusalLoggedAt >= 60 else { return false }
        refusalLoggedAt = now
        return true
    }

    static func register(_ view: CoreTerminalView) {
        views.add(view)
        guard !observing else { return }
        observing = true
        let center = NotificationCenter.default
        center.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main) { _ in
            // The count it returns is logged there. The observer's closure
            // returns nothing, and Xcode 26's compiler won't let the `Int`
            // through it, @discardableResult or not.
            MainActor.assumeIsolated { _ = memoryWarning() }
        }
        center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { background() }
        }
        // Back from the background (B386): every terminal on screen draws
        // again. Active, not entering the foreground: the GPU takes work
        // again only then.
        center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { foreground() }
        }
    }

    /// The app is active again: the terminals in a window draw again
    /// (``CoreTerminalView/cameToForeground()``).
    /// The app went to the background: no frame is drawn until it is
    /// active again, the images go, and the views off screen let their
    /// drawables go.
    static func background() {
        inBackground = true
        mp_term_images_purge()
        for view in views.allObjects {
            view.releaseImageCaches()
            view.releaseDrawablesIfOffScreen()
        }
    }

    static func foreground() {
        inBackground = false
        for view in views.allObjects where view.window != nil {
            view.cameToForeground()
        }
    }

    /// What a memory warning lets go (bytes): the decoded images, and every
    /// terminal's unplaced kitty images.
    @discardableResult
    static func memoryWarning() -> Int {
        var freed = mp_term_images_bytes()
        mp_term_images_purge()
        var offScreen = 0
        for view in views.allObjects {
            freed += view.dropUnplacedImages()
            if view.releaseDrawablesIfOffScreen() { offScreen += 1 }
        }
        Log.ui.info("core term: a memory warning let \(freed, privacy: .public) bytes of images go, and the drawables of \(offScreen, privacy: .public) views off screen")
        return freed
    }

    /// For the debug probes: the decoded images the process keeps, and the
    /// kitty images every terminal stores (bytes).
    static var summary: String {
        let all = views.allObjects
        let kitty = all.map(\.kittyImageBytes)
        let mine = all.map(\.viewImageMemory)
        var zone = malloc_statistics_t()
        malloc_zone_statistics(nil, &zone)
        let metal = CoreTermGPU.shared.map { Double($0.device.currentAllocatedSize) / 1e6 } ?? 0
        func mb(_ n: Int) -> String { String(format: "%.1f", Double(n) / 1e6) }
        return "\(all.count) views (\(all.filter { $0.window != nil }.count) in a window); "
            + "decoded \(mb(mp_term_images_bytes())) MB, kitty stored "
            + kitty.map(mb).joined(separator: " + ") + " MB; views' pixels "
            + mine.map { mb($0.rows) }.joined(separator: " + ") + " MB, textures "
            + mine.map { "\(mb($0.textures)) (\($0.textureCount))" }.joined(separator: " + ")
            + String(format: " MB; Metal allocated %.1f MB; malloc in use %.1f of %.1f MB", metal,
                     Double(zone.size_in_use) / 1e6, Double(zone.size_allocated) / 1e6)
    }
}

/// A counter shared with Metal's completion threads.
/// A terminal view, weakly, for what Metal's threads tell it (B445).
final class CoreTermViewBox: @unchecked Sendable {
    weak var view: CoreTerminalView?
    init(_ view: CoreTerminalView) { self.view = view }
}

final class CoreTermCounter: @unchecked Sendable {
    private let atomic = Atomic<Int>(0)
    var value: Int { atomic.load(ordering: .relaxed) }
    func add(_ n: Int) { atomic.add(n, ordering: .relaxed) }
    func reset() { atomic.store(0, ordering: .relaxed) }
}

/// The waker's box: the view (weakly) and whether a hop is outstanding.
final class CoreTermWake: @unchecked Sendable {
    let queued = Atomic<Bool>(false)
    @MainActor weak var view: CoreTerminalView?
}

/// Events gathered by `mp_term_take_events`'s callback.
private final class CoreTermEvents {
    var list: [(UInt8, String)] = []
}

/// The core's waker: on its thread; one hop to the main thread at a time.
private let coreTermWake: MpTermWaker = { ctx in
    guard let ctx else { return }
    let box = Unmanaged<CoreTermWake>.fromOpaque(ctx).takeUnretainedValue()
    guard box.queued.compareExchange(expected: false, desired: true, ordering: .sequentiallyConsistent).exchanged
    else { return }
    DispatchQueue.main.async {
        MainActor.assumeIsolated { box.view?.woke() }
    }
}

/// The pacer's target (a display link holds its target strongly).
private final class CoreTermPacerTarget: NSObject {
    private weak var view: CoreTerminalView?
    init(_ view: CoreTerminalView) { self.view = view }
    @objc func tick() { MainActor.assumeIsolated { view?.tick() } }
}
