import MoshpitKit
import UIKit

/// The NSH terminal page's terminal (docs/nsh/DESIGN.md §3.2, 终端组件): the
/// session's persistent terminals, shown in the slot NSH lays out
/// (`moshpit/terminal`, ``TerminalNSHSlot``).
///
/// NSH destroys a native view when its element goes; the terminal is not
/// that view. The terminals belong to the session (the plain path's
/// coordinator owns its view, a tmux controller owns one per pane, on
/// either session path), so the screen can be left and entered again, and
/// its NSH tree rebuilt, without a terminal losing its buffer, its
/// scrollback or its place in it. The stage puts the one to show in the
/// slot inside a ``TerminalHostContainer`` — the container whose frame lock
/// keeps the terminal still while the keyboard moves (the coordinator's
/// keyboard observers drive it, as under SwiftUI) — and does what
/// `TerminalScreen.terminalContent`, `TmuxPaneSplitView` and
/// `PaneTerminalHost` do:
///
/// - a pane switch slides the new pane in from the swipe's side once a
///   swipe has happened (the controller's 0.22 s), and fades otherwise;
/// - a reconnect's kept frame (the retired controller's pane, frozen,
///   untouchable) stays until the new attach has painted, then cross-fades
///   to it (0.3 s), so the frame never jumps;
/// - the keyboard follows the person's intent (``TerminalFocusPolicy``),
///   and a sheet's geometry hold keeps the grid across it.
///
/// A rebuild of the slot's element (a rekey) is announced first
/// (``handover()``): the container moves out of the old slot into the
/// window while the terminal still holds the keyboard — moving a first
/// responder between two superviews of one window keeps it — and the new
/// slot adopts it from there, so the keyboard never moves (spike G5).
@MainActor
final class TerminalNSHStage {
    /// What the slot shows.
    struct Target: Equatable {
        enum Source {
            /// The session's single terminal: plain SSH, mosh, a degraded
            /// tmux or herdr connect, herdr's focused pane.
            case plain(SessionHub.ActiveSession)
            /// A tmux pane of `controller`.
            case pane(TmuxSessionController, String)
        }

        let source: Source
        /// The room around a tmux pane (4 pt, 2 in iPad landscape).
        var inset: CGFloat = 0
        /// A reconnect's kept frame: shown, not touched.
        var frozen = false
        /// After a swipe, a pane switch slides: true from the trailing
        /// edge (forward), false from the leading one; nil fades.
        var slide: Bool?

        /// Which terminal: a new controller's pane is a new one even when
        /// tmux's pane id is the same.
        var key: String {
            switch source {
            case .plain(let session): "plain#\(ObjectIdentifier(session.coordinator).hashValue)"
            case .pane(let controller, let pane): "\(ObjectIdentifier(controller).hashValue)#\(pane)"
            }
        }

        static func == (a: Target, b: Target) -> Bool {
            a.key == b.key && a.inset == b.inset && a.frozen == b.frozen && a.slide == b.slide
        }
    }

    /// The look the plain path's terminal takes (tmux's panes take theirs
    /// from their controller).
    struct Appearance: Equatable {
        var theme: TerminalTheme
        var fontSize: Double
        var fontName: String
        var cursorShape: CursorShape
        var cursorColorId: String
        var cursorBlink: Bool
    }

    /// A terminal in the slot (or on its way out of it).
    private final class Hosted {
        let key: String
        let container = TerminalHostContainer()
        /// The terminal, whichever view draws it (``CoreTermSwitch``).
        let surface: TerminalSurface
        /// Its view.
        let terminal: UIView
        let coordinator: SwiftTerminalView.Coordinator?
        var inset: CGFloat

        @MainActor
        init(key: String, surface: TerminalSurface, coordinator: SwiftTerminalView.Coordinator?, inset: CGFloat) {
            self.key = key
            self.surface = surface
            self.terminal = surface.view
            self.coordinator = coordinator
            self.inset = inset
            container.backgroundColor = .clear
        }

        /// Fills `slot`, `inset` in from its edges (bounds and center, so a
        /// slide's transform keeps working).
        func place(in slot: UIView) {
            let box = slot.bounds.insetBy(dx: inset, dy: inset)
            guard box.width > 0, box.height > 0 else { return }
            container.bounds = CGRect(origin: .zero, size: box.size)
            container.center = CGPoint(x: box.midX, y: box.midY)
        }

        /// The terminal is still this host's (another screen's stage may
        /// have taken it since).
        var owns: Bool { terminal.superview === container }
    }

    private(set) var target: Target?
    private var hosted: Hosted?
    /// Containers on their way out (a pane sliding away, a kept frame
    /// fading).
    private var leaving: [Hosted] = []
    /// The slots NSH made, newest last; the one in a window shows.
    private var slots: [Weak] = []
    private struct Weak { weak var slot: TerminalNSHSlotView? }
    /// A container parked in the window by a handover, until a slot takes it.
    private var parkedUntil: CFTimeInterval = 0
    /// A protocol switch's still picture of the pane (``holdStill()``): on
    /// top of the slot, under the page's veil, until the new session has
    /// painted.
    private(set) var still: UIView?

    private(set) var focus: TerminalFocusPolicy = .allow
    private(set) var geometryHeld = false
    private(set) var appearance: Appearance?
    /// Where a tmux pane's own Paste and ⌘V go (the session's paste path).
    var onPaste: ((String) -> Void)?
    /// Every swap, for the debug director: the key shown and when.
    var onShow: ((String?) -> Void)?

    /// How long transitions run: a pane switch (the controller's
    /// `withAnimation(.easeInOut(duration: 0.22))`), a reconnect's
    /// cross-fade (the cover stack's 0.3 s).
    static let switchDuration: TimeInterval = 0.22
    static let crossfadeDuration: TimeInterval = 0.3

    /// A pane-switch veil up longer than this when the app comes back is
    /// stuck (its cap is under a second; resync passes extend it a little).
    static let staleVeil: TimeInterval = 5

    /// How long after a terminal is put in a slot the stage looks at what
    /// shows (``check(why:now:)``, B445).
    static let checkDelay: TimeInterval = 2

    private var activeObserver: NSObjectProtocol?
    private var mintObserver: NSObjectProtocol?
    /// A ``verify(why:)`` queued for the next turn.
    private var verifyQueued = false
    /// The ``check(why:now:)`` due ``checkDelay`` after the last time a
    /// terminal was put in a slot.
    private var pendingCheck: DispatchWorkItem?

    init() {
        // Back from the background (B386): what the slot should show is in
        // it, and nothing stale covers it.
        activeObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            // What it found is for tests. The observer's closure returns
            // nothing, and Xcode 26 won't let the result through it.
            MainActor.assumeIsolated { _ = self?.reassert() }
        }
        // A terminal was minted (B445): it may be the one the target names
        // now (a pane's terminal made again under the same controller), and
        // then it goes in the slot at once, not when the screen is next
        // entered. On the next turn: its owner has it by then.
        mintObserver = NotificationCenter.default.addObserver(
            forName: TerminalSurfaces.minted, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.queueVerify() }
        }
    }

    deinit {
        if let activeObserver { NotificationCenter.default.removeObserver(activeObserver) }
        if let mintObserver { NotificationCenter.default.removeObserver(mintObserver) }
    }

    /// What shows is what the target names, in the slot on screen, with no
    /// stuck veil over it (B386): on the app's return the terminal may have
    /// been taken by another host, or left out of the slot, or still be
    /// veiled for a switch long over. Each is said in the log, then put
    /// right (``verify(why:)``), and what shows is looked at again a moment
    /// later (``check(why:now:)``). What it found.
    @discardableResult
    func reassert(now: CFTimeInterval = CACurrentMediaTime()) -> Reassert {
        guard target != nil else { return .nothing }
        defer { scheduleCheck(why: "the app's return") }
        var found = Reassert.fine
        switch verify(why: "on return") {
        case .rehosted: return .rehosted
        case .adopted: found = .adopted
        case .fine, .nothing: break
        }
        if hosted?.coordinator?.liftStaleVeil(olderThan: Self.staleVeil, now: now) == true {
            found = .unveiled
        }
        return found
    }

    /// What ``verify(why:)`` did.
    enum Verify: Equatable {
        /// Nothing to look at: no target, no slot on screen, or a handover
        /// under way (the next slot takes the terminal).
        case nothing
        /// What shows is the terminal the target names, in the slot.
        case fine
        /// It was not (a new terminal under the same target, or one another
        /// host took): hosted now, at once.
        case rehosted
        /// Its host was out of the slot on screen: moved in.
        case adopted
    }

    /// What shows is the terminal the target names now, in the slot on
    /// screen (B445). A new terminal under the same target — a pane's made
    /// again under its controller — is hosted at once; so is one another
    /// host took (B386). Nothing without a slot on screen: a screen out of
    /// sight never takes the terminal from the one in sight. Each fix is
    /// said in the log.
    @discardableResult
    func verify(why: String) -> Verify {
        guard let target, let slot, CACurrentMediaTime() >= parkedUntil else { return .nothing }
        guard let hosted, hosted.owns, current(target) === hosted.terminal else {
            let had = self.hosted != nil, owned = self.hosted?.owns ?? false
            let same = self.hosted.map { current(target) === $0.terminal } ?? false
            Log.ui.notice("terminal stage: \(target.key, privacy: .public) is not in its host \(why, privacy: .public) (hosted \(had, privacy: .public), owns \(owned, privacy: .public), the same terminal \(same, privacy: .public)): hosting it")
            show(target, instant: true)
            return .rehosted
        }
        if hosted.container.superview !== slot {
            Log.ui.notice("terminal stage: \(target.key, privacy: .public) is not in the slot on screen \(why, privacy: .public): moving it in")
            adopt(into: slot)
            return .adopted
        }
        return .fine
    }

    /// One ``verify(why:)`` on the next turn, however many terminals are
    /// minted in this one.
    private func queueVerify() {
        guard !verifyQueued, target != nil else { return }
        verifyQueued = true
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.verifyQueued = false
                self.verify(why: "after a terminal was minted")
            }
        }
    }

    // MARK: The look at what shows (B445)

    /// What ``check(why:now:)`` found, and what it put right.
    struct Health: Equatable {
        var key: String
        /// Its host is in the slot on screen.
        var inSlot: Bool
        /// The terminal's size, in points.
        var size: CGSize
        /// Its host's opacity.
        var alpha: CGFloat
        /// How long a pane-switch veil has been up over it; nil: none.
        var veil: TimeInterval?
        /// A protocol switch's picture is over the slot.
        var still: Bool
        /// The terminal's rows have text.
        var text: Bool
        /// Frames drawn since it got its canvas; nil: not counted.
        var frames: Int?
        /// How long since bytes last reached it; nil: none ever did.
        var fed: TimeInterval?
        /// What was wrong and is put right; empty: nothing was.
        var fixed: [String] = []

        var line: String {
            func seconds(_ t: TimeInterval?) -> String {
                t.map { String(format: "%.1f s", $0) } ?? "never"
            }
            let state = "in slot \(inSlot), \(Int(size.width))×\(Int(size.height)) pt, alpha \(String(format: "%.2f", alpha))"
                + ", veil \(veil.map { String(format: "up %.1f s", $0) } ?? "none"), still \(still)"
                + ", text \(text), frames \(frames.map { "\($0)" } ?? "uncounted"), fed \(fed == nil ? "never" : seconds(fed) + " ago")"
            return "\(key): \(state)" + (fixed.isEmpty ? ": fine" : ": fixed \(fixed.joined(separator: ", "))")
        }
    }

    /// Looks at what shows (B445), ``checkDelay`` after a terminal was put
    /// in a slot: the terminal the target names, in the slot on screen,
    /// sized, opaque, under no stuck veil, and drawn when it has text.
    /// Whatever is not is put right and said as an error (the earlier-runs
    /// trail keeps it); otherwise one notice line says what shows. Nil with
    /// nothing to look at.
    @discardableResult
    func check(why: String, now: CFTimeInterval = CACurrentMediaTime()) -> Health? {
        guard let target, let slot, CACurrentMediaTime() >= parkedUntil else { return nil }
        var fixed: [String] = []
        switch verify(why: "at the check") {
        case .rehosted: fixed.append("hosted it again")
        case .adopted: fixed.append("moved it into the slot")
        case .fine, .nothing: break
        }
        guard let hosted, hosted.owns else { return nil }
        let container = hosted.container
        if container.bounds.width < 1 || container.bounds.height < 1, slot.bounds.width > 0, slot.bounds.height > 0 {
            hosted.place(in: slot)
            fixed.append("sized it to the slot")
        }
        let animating = !(container.layer.animationKeys() ?? []).isEmpty
        if (container.alpha < 1 || container.isHidden), !animating, !leaving.contains(where: { $0 === hosted }) {
            container.alpha = 1
            container.isHidden = false
            container.transform = .identity
            fixed.append("made it opaque")
        }
        let veil = hosted.coordinator?.veilAge(now: now)
        if let veil, veil > Self.staleVeil,
           hosted.coordinator?.liftStaleVeil(olderThan: Self.staleVeil, now: now) == true {
            fixed.append("lifted a veil up \(Int(veil)) s")
        } else if let veil {
            // Up, not stale yet: looked at again once it would be.
            scheduleCheck(why: "a veil was up", after: Self.staleVeil - veil + 0.5)
        }
        let text = Self.hasText(hosted.surface)
        let frames = hosted.surface.framesSinceCanvas
        // Nothing is drawn in the background (B386): no frame then is not a
        // fault, and the return looks again.
        if text, frames == 0, !CoreTermMemory.inBackground {
            hosted.surface.redrawAll()
            fixed.append("drew it again (text, nothing drawn)")
        }
        let health = Health(
            key: target.key, inSlot: container.superview === slot, size: hosted.terminal.bounds.size,
            alpha: container.alpha, veil: veil, still: still != nil, text: text, frames: frames,
            fed: hosted.coordinator?.fedAgo(now: now), fixed: fixed)
        if fixed.isEmpty {
            Log.ui.notice("terminal stage: check after \(why, privacy: .public): \(health.line, privacy: .public)")
        } else {
            NSHDiagnostics.error("terminal stage: check after \(why): \(health.line)")
        }
        return health
    }

    /// Whether any row of `surface`'s screen has text.
    static func hasText(_ surface: TerminalSurface) -> Bool {
        (0..<min(surface.grid.rows, 300)).contains { !surface.rowText($0).isEmpty }
    }

    /// One ``check(why:now:)`` ``checkDelay`` from now, in place of one
    /// already due.
    private func scheduleCheck(why: String, after delay: TimeInterval = TerminalNSHStage.checkDelay) {
        pendingCheck?.cancel()
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated { _ = self?.check(why: why) }
        }
        pendingCheck = work
        DispatchQueue.main.asyncAfter(deadline: .now() + max(0.1, delay), execute: work)
    }

    /// What ``reassert(now:)`` found.
    enum Reassert: Equatable {
        /// No target: nothing shows.
        case nothing
        /// What shows was where it should be.
        case fine
        /// The terminal was not in its host: hosted again.
        case rehosted
        /// The host was not in the slot on screen: moved in.
        case adopted
        /// A stuck veil covered it: lifted.
        case unveiled
    }

    // MARK: The slot

    /// The slot on screen: the newest one in a window.
    var slot: TerminalNSHSlotView? {
        slots.reversed().compactMap(\.slot).first { $0.window != nil }
    }

    /// The terminal on screen now.
    var terminal: UIView? { hosted?.owns == true ? hosted?.terminal : nil }

    /// The container on screen now.
    var container: TerminalHostContainer? { hosted?.container }

    func register(_ slot: TerminalNSHSlotView) {
        slots.removeAll { $0.slot == nil }
        slots.append(Weak(slot: slot))
        slot.stage = self
    }

    /// A slot came into (or left) a window: the newest one in a window
    /// takes what shows.
    func slotMoved(_ slot: TerminalNSHSlotView) {
        guard slot.window != nil, self.slot === slot else { return }
        adopt(into: slot)
    }

    /// A slot is leaving its window with the terminal still holding the
    /// keyboard, unannounced (its screen went away): nothing to keep.
    func slotLeaving(_ slot: TerminalNSHSlotView) {}

    /// The slot took its size (NSH laid it out, inside the keyboard's
    /// animation when the keyboard moves it): what shows fills it.
    func slotLaidOut(_ slot: TerminalNSHSlotView) {
        for host in leaving + [hosted].compactMap({ $0 }) where host.container.superview === slot {
            host.place(in: slot)
        }
    }

    // MARK: A protocol switch's still picture

    /// A protocol switch (mosh ⇄ SSH) swaps in a new session, with a new
    /// terminal that has nothing on it yet: the pane's last picture stays
    /// in the slot, over whatever the new session shows, under the page's
    /// veil, until ``releaseStill()`` (the new session has painted). Drawn
    /// from the terminal's own rows (``TerminalSurface/snapshotView()``),
    /// not the screen's, which a Metal layer may not give. Nothing to hold
    /// without a terminal on screen.
    func holdStill() {
        guard still == nil, let hosted, hosted.owns,
              let slot = hosted.container.superview as? TerminalNSHSlotView,
              let picture = hosted.surface.snapshotView() else { return }
        picture.frame = hosted.terminal.convert(hosted.terminal.bounds, to: slot)
        picture.isUserInteractionEnabled = false
        picture.accessibilityElementsHidden = true
        slot.addSubview(picture)
        still = picture
        Log.ui.notice("terminal stage: holding the pane's picture for a protocol switch")
    }

    /// The switch is done: the picture goes at once, the new session's
    /// frame under it.
    func releaseStill() {
        guard let still else { return }
        still.removeFromSuperview()
        self.still = nil
    }

    /// The still picture above what the slot hosts.
    private func keepStillOnTop(in slot: UIView) {
        guard let still else { return }
        if still.superview !== slot { slot.addSubview(still) } else { slot.bringSubviewToFront(still) }
    }

    func slotDestroyed(_ slot: TerminalNSHSlotView) {
        slots.removeAll { $0.slot == nil || $0.slot === slot }
        if let hosted, hosted.container.superview === slot {
            hosted.container.removeFromSuperview()
        }
        for leaving in leaving where leaving.container.superview === slot {
            leaving.container.removeFromSuperview()
        }
    }

    /// Moves what shows (and what is still fading away) into `slot`.
    private func adopt(into slot: TerminalNSHSlotView) {
        for host in leaving + [hosted].compactMap({ $0 }) where host.container.superview !== slot {
            let wasFirst = host.terminal.isFirstResponder
            host.container.transform = .identity
            host.place(in: slot)
            slot.addSubview(host.container)
            if host === hosted, host.owns {
                host.coordinator?.hostContainer = host.container
            }
            Log.ui.notice("terminal stage: \(host.key, privacy: .public) into slot \(slot.serial, privacy: .public), owns \(host.owns, privacy: .public), first responder \(wasFirst, privacy: .public) -> \(host.terminal.isFirstResponder, privacy: .public)")
        }
        parkedUntil = 0
        if let hosted {
            bringForward(hosted, in: slot)
            // A picture lost on the way waits for no byte (B445).
            if hosted.owns { hosted.surface.redrawAll() }
            scheduleCheck(why: "slot \(slot.serial) took it")
        }
        keepStillOnTop(in: slot)
        applyFocus()
    }

    /// The terminal is about to move to a new element (a rekey): keep it in
    /// the window, still first responder, for the slot NSH makes next.
    func handover() {
        guard let hosted, let slot = hosted.container.superview as? TerminalNSHSlotView,
              let window = slot.window else { return }
        let container = hosted.container
        let frame = container.convert(container.bounds, to: window)
        window.insertSubview(container, at: 0)
        container.frame = frame
        parkedUntil = CACurrentMediaTime() + 0.15
        Log.ui.info("terminal stage: handover of \(hosted.key, privacy: .public), first responder \(hosted.terminal.isFirstResponder, privacy: .public)")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self, weak container] in
            guard let self, let container, container.superview === window,
                  CACurrentMediaTime() >= self.parkedUntil else { return }
            // Nobody took it: its screen went away, the keyboard with it.
            container.removeFromSuperview()
        }
    }

    // MARK: What shows

    /// Shows `target` (nil: nothing — an empty state, a blank wait, no
    /// session yet), with the transition its change calls for.
    ///
    /// `instant`: a protocol switch's swap (nothing moves through a switch,
    /// the owner's ask): the old terminal goes and the new one is there at
    /// once, no fade, no slide.
    func show(_ target: Target?, instant: Bool = false) {
        if target == self.target {
            // Nothing to do while what shows is still ours and still the
            // terminal the target names.
            guard let target else { return }
            if let hosted, hosted.owns, current(target) === hosted.terminal { return }
        }
        let previous = self.target
        self.target = target
        guard let target else {
            if let hosted {
                if instant { dismiss(hosted) } else { retire(hosted, duration: Self.crossfadeDuration, slideForward: nil) }
            }
            hosted = nil
            onShow?(nil)
            return
        }
        if let hosted, hosted.key == target.key, hosted.owns, current(target) === hosted.terminal {
            // The same terminal: only its room or its frozenness changed.
            if hosted.inset != target.inset {
                hosted.inset = target.inset
                if let slot = hosted.container.superview as? TerminalNSHSlotView {
                    hosted.place(in: slot)
                }
            }
            hosted.container.isUserInteractionEnabled = !target.frozen
            return
        }
        guard let next = host(target) else { return }
        let old = hosted
        hosted = next
        onShow?(target.key)
        guard let old else {
            applyFocus()
            return
        }
        if instant {
            dismiss(old)
            next.container.alpha = 1
            next.container.transform = .identity
            applyFocus()
            return
        }
        // A pane slides only after a swipe, and never out of or into a kept
        // frame; a reconnect's swap fades slower.
        var plain = false
        if case .plain = target.source { plain = true }
        let reconnect = previous?.frozen == true || target.frozen || plain
        let slide = reconnect ? nil : target.slide
        let duration = reconnect ? Self.crossfadeDuration : Self.switchDuration
        if let forward = slide, let slot = next.container.superview {
            let width = slot.bounds.width
            next.container.transform = CGAffineTransform(translationX: forward ? width : -width, y: 0)
        }
        next.container.alpha = 0
        retire(old, duration: duration, slideForward: slide)
        UIView.animate(withDuration: duration, delay: 0, options: [.curveEaseInOut, .allowUserInteraction]) {
            next.container.alpha = 1
            next.container.transform = .identity
        }
        applyFocus()
    }

    /// The terminal `target` names now, if it has one yet (an address
    /// reused by a new controller makes the same key; the view tells).
    private func current(_ target: Target) -> UIView? {
        switch target.source {
        case .plain(let session): session.coordinator.surface?.view
        case .pane(let controller, let pane): controller.surface(for: pane).view
        }
    }

    /// A host for `target`'s terminal, in the slot when there is one.
    private func host(_ target: Target) -> Hosted? {
        let surface: TerminalSurface
        let coordinator: SwiftTerminalView.Coordinator?
        switch target.source {
        case .plain(let session):
            coordinator = session.coordinator
            surface = plainSurface(session.coordinator)
        case .pane(let controller, let pane):
            surface = controller.surface(for: pane)
            coordinator = controller.coordinator(for: pane)
            coordinator?.onPaste = onPaste
        }
        let hosted = Hosted(key: target.key, surface: surface, coordinator: coordinator, inset: target.inset)
        hosted.container.host(surface)
        coordinator?.hostContainer = hosted.container
        hosted.container.geometryHeld = geometryHeld
        hosted.container.isUserInteractionEnabled = !target.frozen
        if let slot {
            hosted.place(in: slot)
            slot.addSubview(hosted.container)
            keepStillOnTop(in: slot)
            // Every terminal put in a slot is said, tmux's and herdr's alike
            // (B445: only a slot's own adoption was, so a swap's terminal
            // looked as if it never arrived), and drawn whole at once.
            let size = hosted.container.bounds.size
            Log.ui.notice("terminal stage: \(target.key, privacy: .public) hosted into slot \(slot.serial, privacy: .public), \(Int(size.width), privacy: .public)×\(Int(size.height), privacy: .public) pt\(target.frozen ? ", frozen" : "", privacy: .public)")
            surface.redrawAll()
            scheduleCheck(why: "hosted in slot \(slot.serial)")
        } else {
            Log.ui.notice("terminal stage: \(target.key, privacy: .public) hosted, no slot on screen yet")
        }
        return hosted
    }

    /// The plain path's terminal: the coordinator's own, re-wired, or a new
    /// one (as `SwiftTerminalView.makeUIView` does).
    private func plainSurface(_ coordinator: SwiftTerminalView.Coordinator) -> TerminalSurface {
        let look = appearance
        let surface = SwiftTerminalView.mintOwnedSurface(
            coordinator: coordinator, fontName: look?.fontName ?? "system",
            fontSize: look?.fontSize ?? AppSettings.shared.fontSize)
        style(surface)
        return surface
    }

    /// Takes `host` away: slid out the side opposite the new pane's, or
    /// faded, then out of the hierarchy.
    private func retire(_ host: Hosted, duration: TimeInterval, slideForward: Bool?) {
        Log.ui.info("terminal stage: \(host.key, privacy: .public) leaves the slot over \(duration, privacy: .public) s")
        leaving.append(host)
        host.container.isUserInteractionEnabled = false
        let width = host.container.superview?.bounds.width ?? host.container.bounds.width
        UIView.animate(withDuration: duration, delay: 0, options: [.curveEaseInOut, .allowUserInteraction]) {
            host.container.alpha = 0
            if let forward = slideForward {
                host.container.transform = CGAffineTransform(translationX: forward ? -width : width, y: 0)
            }
        } completion: { [weak self] _ in
            guard let self else { return }
            self.leaving.removeAll { $0 === host }
            // Unless it is showing again (a quick swipe back).
            guard self.hosted !== host else { return }
            host.container.removeFromSuperview()
            host.container.transform = .identity
        }
    }

    /// Takes `host` away at once (a protocol switch's swap).
    private func dismiss(_ host: Hosted) {
        Log.ui.info("terminal stage: \(host.key, privacy: .public) leaves the slot at once")
        leaving.removeAll { $0 === host }
        host.container.layer.removeAllAnimations()
        host.container.removeFromSuperview()
        host.container.transform = .identity
        host.container.alpha = 1
    }

    /// The shown pane on top of what fades out.
    private func bringForward(_ host: Hosted, in slot: UIView) {
        if host.container.superview === slot { slot.bringSubviewToFront(host.container) }
    }

    // MARK: The keyboard and the geometry hold

    /// The person's keyboard intent and a sheet's hold, as the SwiftUI
    /// screen passes them to its hosts.
    func apply(focus: TerminalFocusPolicy, geometryHeld: Bool) {
        self.focus = focus
        if self.geometryHeld != geometryHeld {
            self.geometryHeld = geometryHeld
            if let hosted, hosted.owns { hosted.container.geometryHeld = geometryHeld }
        }
        applyFocus()
    }

    /// `PaneTerminalHost.updateUIView`'s focus rules: take it (now, and
    /// back if something stole it), leave it, or give it up.
    private func applyFocus() {
        guard let hosted, hosted.owns else { return }
        let terminal = hosted.terminal
        switch focus {
        case .take:
            if !terminal.isFirstResponder, terminal.window != nil, !hosted.surface.isFinding {
                DispatchQueue.main.async { [weak terminal] in _ = terminal?.becomeFirstResponder() }
            }
        case .allow:
            break
        case .resign:
            if terminal.isFirstResponder {
                DispatchQueue.main.async { [weak terminal] in _ = terminal?.resignFirstResponder() }
            }
        }
    }

    /// The plain path's look, as `SwiftTerminalView.updateUIView` keeps it.
    func apply(appearance: Appearance) {
        guard appearance != self.appearance else { return }
        self.appearance = appearance
        guard let hosted, hosted.owns, case .plain? = target?.source, hosted.coordinator != nil
        else { return }
        hosted.surface.apply(font: TerminalFont.font(id: appearance.fontName, size: CGFloat(appearance.fontSize)))
        style(hosted.surface)
    }

    private func style(_ surface: TerminalSurface) {
        guard let look = appearance else { return }
        surface.apply(look: TerminalLook(theme: look.theme, cursorShape: look.cursorShape,
                                         cursorColorId: look.cursorColorId, cursorBlink: look.cursorBlink))
    }
}

/// `moshpit/terminal`'s view: the box NSH lays out for the terminal. It
/// tells the stage when it reaches a window, so the stage can move the
/// terminal in (before that, moving it here would take it out of the
/// window, and its keyboard with it).
final class TerminalNSHSlotView: UIView {
    weak var stage: TerminalNSHStage?
    /// Which slot this is, for the logs.
    let serial: Int
    private static var made = 0

    override init(frame: CGRect) {
        Self.made += 1
        serial = Self.made
        super.init(frame: frame)
        clipsToBounds = true
        backgroundColor = .clear
        accessibilityIdentifier = "terminal-slot"
    }

    required init?(coder: NSCoder) {
        fatalError("TerminalNSHSlotView is made in code")
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        super.willMove(toWindow: newWindow)
        if newWindow == nil, window != nil { stage?.slotLeaving(self) }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        stage?.slotMoved(self)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        stage?.slotLaidOut(self)
    }
}

/// `moshpit/terminal`: the slot. Props: `generation` (a rekey's count, for
/// the logs). Commands: `focus`, `blur` (NSH has no focus for native
/// views; the host's keyboard intent decides otherwise).
@MainActor
final class TerminalNSHSlot: NSHComponent {
    private let slot = TerminalNSHSlotView()
    private weak var stage: TerminalNSHStage?
    var view: UIView { slot }

    init(stage: TerminalNSHStage) {
        self.stage = stage
        stage.register(slot)
    }

    func setProp(_ name: String, _ value: Any?) {
        if name == "generation" {
            Log.ui.debug("terminal slot \(self.slot.serial, privacy: .public): generation \(String(describing: value), privacy: .public)")
        }
    }

    func command(_ name: String, _ args: Any?) {
        guard let terminal = stage?.terminal else { return }
        switch name {
        case "focus": _ = terminal.becomeFirstResponder()
        case "blur": _ = terminal.resignFirstResponder()
        default: break
        }
    }

    func destroy() {
        stage?.slotDestroyed(slot)
    }
}
