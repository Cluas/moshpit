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
/// keyboard observers drive it) — and does this:
///
/// - a pane switch slides the new pane in from the swipe's side once a
///   swipe has happened (the controller's 0.22 s), and fades otherwise;
/// - a sideways drag moves the terminal with the finger, and the screen of
///   the pane it would go to follows it in from that side (a peek, when
///   there is a picture of it; the page's backstage, its name, when not),
///   and rubber-bands where the page names no neighbour; let go past half
///   the width or flicked, the terminal flies on, the peek lands in its
///   place until the pane's own screen is up, and the neighbour is asked
///   for (``onSwitchPane``); else both spring back (the pane-switch
///   design's step 1, `ui/src/terminal/switcher.rs`);
/// - a reconnect's kept frame (the retired controller's pane, frozen,
///   untouchable) stays until the new attach has painted, then cross-fades
///   to it (0.3 s), so the frame never jumps;
/// - the pane overview (B658): the terminal shrinks into its pane's cell
///   as the overview fades in over it, and grows back out of the cell on
///   closing; a picked pane's terminal grows from the picked cell (0.24 s,
///   interruptible, nothing waits on it; under Reduce Motion the page's
///   cross-fade alone); and the overview's pictures are read from the
///   terminals the session already keeps;
/// - the keyboard follows the person's intent (``TerminalFocusPolicy``),
///   and a sheet's geometry hold keeps the grid across it.
///
/// A rebuild of the slot's element (a rekey) is announced first
/// (``handover()``): the container moves out of the old slot into the
/// window while the terminal still holds the keyboard — moving a first
/// responder between two superviews of one window keeps it — and the new
/// slot adopts it from there, so the keyboard never moves (spike G5).
/// What the stage needs of a tmux controller: each pane's one terminal, for
/// the pane's whole life, and its coordinator (``RustTmuxController``).
@MainActor
protocol PaneTerminalSource: AnyObject {
    func surface(for paneId: String) -> CoreTerminalView
    func coordinator(for paneId: String) -> CoreTerminalHost.Coordinator?
    /// The pane's terminal if it has one, never one made for asking.
    func existingSurface(for paneId: String) -> CoreTerminalView?
}

extension PaneTerminalSource {
    func existingSurface(for paneId: String) -> CoreTerminalView? { nil }
}

extension RustTmuxController: PaneTerminalSource {}

@MainActor
final class TerminalNSHStage {
    /// What the slot shows.
    struct Target: Equatable {
        enum Source {
            /// The session's single terminal: plain SSH, mosh, a degraded
            /// tmux or herdr connect, herdr's focused pane.
            case plain(SessionHub.ActiveSession)
            /// A tmux pane of `controller`.
            case pane(any PaneTerminalSource, String)
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
        /// The terminal (the core's view).
        let surface: CoreTerminalView
        /// Its view.
        let terminal: UIView
        let coordinator: CoreTerminalHost.Coordinator?
        var inset: CGFloat

        @MainActor
        init(key: String, surface: CoreTerminalView, coordinator: CoreTerminalHost.Coordinator?, inset: CGFloat) {
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
    /// What lies outside the shown terminal's grid, when it isn't the
    /// theme's background (B496): the page's chrome takes it.
    var onGround: ((UInt32?) -> Void)?
    /// The last colour said through ``onGround``.
    private var saidGround: UInt32??

    /// Says the shown terminal's ground if it changed (B496).
    private func sayGround() {
        let ground = hosted.flatMap { $0.owns ? $0.surface.groundColor : nil }
        guard saidGround != .some(ground) else { return }
        saidGround = .some(ground)
        onGround?(ground)
    }

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
    static func hasText(_ surface: CoreTerminalView) -> Bool {
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
        paint(slot)
    }

    /// The terminal theme's background behind what the slot hosts (B473).
    /// The page under the slot is the app theme's terminal ground (near
    /// black), which showed around a tmux pane's inset as a black strip
    /// beside the terminal's own grey.
    private func paint(_ slot: TerminalNSHSlotView) {
        guard let ground = Self.color(appearance?.theme.backgroundHex) else { return }
        slot.backgroundColor = ground
    }

    /// `#rrggbb` as a colour.
    static func color(_ hex: String?) -> UIColor? {
        guard let hex, let rgb = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "# ")), radix: 16)
        else { return nil }
        return UIColor(red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
                       blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
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
    /// from the terminal's own rows (``CoreTerminalView/snapshotView()``),
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
            sayGround()
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
        sayGround()
        guard let old else {
            applyFocus()
            return
        }
        // The keyboard stays where it is (B486): handed to the new terminal
        // while the old one, still in the window, holds it. Taken out with
        // the keyboard, the old one let it drop — a protocol switch's swap
        // dropped it every time — and only a later `take` brought it back.
        if old.owns, old.terminal.isFirstResponder { handKeyboard(to: next) }
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
        // A swipe that asked for this pane says the side it comes from.
        var swiped: Bool?
        if let committed, CACurrentMediaTime() - committed.at < Self.committedFor { swiped = committed.forward }
        committed = nil
        let slide = reconnect ? nil : (swiped ?? target.slide)
        let duration = reconnect ? Self.crossfadeDuration : Self.switchDuration
        // The pane picked in the overview: it grows from its cell over the
        // old one fading (B658).
        if !reconnect, let from = pickedCell(for: target) {
            if old.coordinator !== next.coordinator {
                old.coordinator?.onSwipe = nil
                old.coordinator?.onStep = nil
                old.coordinator?.onShowAll = nil
            }
            retire(old, duration: Self.overviewDuration, slideForward: nil)
            next.container.alpha = 1
            next.container.transform = Self.fit(next.container.bounds.size, at: next.container.center, into: from)
            animateOverview(next, to: .identity)
            applyFocus()
            return
        }
        // A swipe's peek of this pane landed in its place: the terminal goes
        // under it at once, and the peek lifts when its screen is up.
        if !reconnect, landed, let peek, case .pane(_, let pane) = target.source, pane == peek.pane, swiped != nil {
            if old.coordinator !== next.coordinator {
                old.coordinator?.onSwipe = nil
                old.coordinator?.onStep = nil
                old.coordinator?.onShowAll = nil
            }
            retire(old, duration: duration, slideForward: nil)
            next.container.alpha = 1
            next.container.transform = .identity
            peek.superview?.bringSubviewToFront(peek)
            liftPeek(over: next.coordinator)
            applyFocus()
            return
        }
        if let forward = slide, let slot = next.container.superview {
            let width = slot.bounds.width
            next.container.transform = CGAffineTransform(translationX: forward ? width : -width, y: 0)
        }
        next.container.alpha = 0
        // A terminal a swipe threw is on its way out already: it only fades.
        retire(old, duration: duration, slideForward: old === flying ? nil : slide)
        if old.coordinator !== next.coordinator {
            old.coordinator?.onSwipe = nil
            old.coordinator?.onStep = nil
            old.coordinator?.onShowAll = nil
        }
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
        let surface: CoreTerminalView
        let coordinator: CoreTerminalHost.Coordinator?
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
        // Its ground, to the page's chrome while it shows (B496).
        coordinator?.onGround = { [weak self, weak hosted] in
            guard let self, let hosted, self.hosted === hosted else { return }
            self.sayGround()
        }
        hosted.container.host(surface)
        coordinator?.hostContainer = hosted.container
        // The sideways drag and VoiceOver's steps are this stage's while it
        // hosts the terminal (the page's order, no wrap-around).
        coordinator?.onSwipe = { [weak self, weak hosted] phase in
            guard let self, let hosted, self.hosted === hosted else { return }
            self.swipe(phase, on: hosted)
        }
        coordinator?.onStep = { [weak self] forward in _ = self?.step(forward: forward) }
        coordinator?.onShowAll = { [weak self] in self?.onShowAll?() }
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
    /// one (``CoreTerminalHost/mintOwnedSurface(coordinator:fontName:fontSize:)``).
    private func plainSurface(_ coordinator: CoreTerminalHost.Coordinator) -> CoreTerminalView {
        let look = appearance
        let surface = CoreTerminalHost.mintOwnedSurface(
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

    // MARK: The sideways swipe

    /// How far the drag goes, as a share of the width, before a lift
    /// switches (half: held short of it, the peek is only looked at); a
    /// flick faster than ``flickSpeed`` (points a second) past
    /// ``flickDistance`` switches too.
    static let commitShare: CGFloat = 0.5
    static let flickSpeed: CGFloat = 500
    static let flickDistance: CGFloat = 24
    /// Where nothing is: the drag moves the terminal a quarter as far, never
    /// past ``rubberLimit``.
    static let rubberShare: CGFloat = 0.25
    static let rubberLimit: CGFloat = 60
    /// The structural spring's response (`spring-structural`, critically
    /// damped).
    static let springResponse: TimeInterval = 0.35
    /// The pane coming back after a swipe switched the one terminal in
    /// place (mosh, herdr): `base`.
    static let returnDuration: TimeInterval = 0.24
    /// How long a committed swipe's side waits for the pane it asked for.
    static let committedFor: CFTimeInterval = 1.5
    /// The hint's nudge toward the next pane.
    static let nudgeDistance: CGFloat = 32

    /// The panes either side of the one shown, by id (the page's
    /// `switcher.rs`): none either side with nowhere to go.
    private(set) var neighbours: (previous: String?, next: String?) = (nil, nil)
    /// Show pane `pane` (`forward`: the next one): a swipe let go past the
    /// threshold, or a step.
    var onSwitchPane: ((_ pane: String, _ forward: Bool) -> Void)?
    /// A swipe began (true) or settled (false): the page shows its dots
    /// while it lasts.
    var onSwiping: ((Bool) -> Void)?
    /// VoiceOver asked for every pane: the page's overview.
    var onShowAll: (() -> Void)?

    /// The drag under way: on which host, how far, and whether past the
    /// threshold (its haptic says each crossing).
    private struct Drag {
        weak var host: Hosted?
        var past = false
    }
    private var drag: Drag?
    /// The direction of the switch a swipe or a step asked for, until the
    /// pane shows (``committedFor`` at most).
    private var committed: (forward: Bool, at: CFTimeInterval)?
    /// The terminal a committed swipe threw, while it flies.
    private weak var flying: Hosted?
    /// The screen of a pane either side the page has a picture of (the
    /// snapshots `swipe-pictures` asked for), by id.
    var peekRows: ((String) -> [String]?)?
    /// The peek following the drag, or landed in the terminal's place after
    /// a switch until the pane's own screen is up.
    private var peek: TerminalPeekView?
    /// The peek landed: the switch it stands in for.
    private var landed = false
    /// How long a landed peek waits for the pane's screen at most; one whose
    /// terminal never veils lifts sooner.
    static let peekHeldFor: CFTimeInterval = 1.2
    static let peekUnveiledFor: CFTimeInterval = 0.4

    func setNeighbours(previous: String?, next: String?) {
        neighbours = (previous, next)
    }

    /// Whether a drag of `dx` goes to the next pane: leftward in a
    /// left-to-right layout.
    private func forward(_ dx: CGFloat) -> Bool {
        let ltr = (slot?.effectiveUserInterfaceLayoutDirection ?? .leftToRight) == .leftToRight
        return ltr ? dx < 0 : dx > 0
    }

    /// The pane a drag of `dx` would go to.
    private func neighbour(_ dx: CGFloat) -> String? {
        guard dx != 0 else { return nil }
        return forward(dx) ? neighbours.next : neighbours.previous
    }

    /// Where a drag of `dx` puts the terminal toward nothing: resisting,
    /// never past ``rubberLimit``.
    static func rubberBand(_ dx: CGFloat) -> CGFloat {
        let d = rubberLimit
        let x = abs(dx)
        let shown = d * (1 - 1 / (x * rubberShare / d + 1))
        return dx < 0 ? -shown : shown
    }

    private var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }

    /// One phase of a sideways drag on `host`'s terminal.
    private func swipe(_ phase: TerminalSwipe, on host: Hosted) {
        switch phase {
        case .began:
            // One pane: no swipe, no rubber band hinting at more.
            guard neighbours.previous != nil || neighbours.next != nil, host.owns, target?.frozen != true else {
                drag = nil
                return
            }
            // A settle still in flight stops where it is (an overview's
            // growth too).
            stopOverviewMotion()
            if let presentation = host.container.layer.presentation() {
                host.container.transform = presentation.affineTransform()
            }
            host.container.layer.removeAllAnimations()
            dropPeek()
            drag = Drag(host: host)
            uncover(true)
            onSwiping?(true)
        case .moved(let dx):
            guard var drag, drag.host === host else { return }
            let width = max(host.container.bounds.width, 1)
            let toward = neighbour(dx)
            let x = toward == nil ? Self.rubberBand(dx) : dx
            host.container.transform = CGAffineTransform(translationX: x, y: 0)
            followPeek(dx: toward == nil ? 0 : dx, host: host)
            let past = toward != nil && abs(dx) > width * Self.commitShare
            if past != drag.past { Haptics.select() }
            drag.past = past
            self.drag = drag
        case .ended(let dx, let velocity):
            guard let drag, drag.host === host else { return }
            self.drag = nil
            let width = max(host.container.bounds.width, 1)
            let flick = abs(velocity) > Self.flickSpeed && abs(dx) > Self.flickDistance && (velocity < 0) == (dx < 0)
            if let pane = neighbour(dx), abs(dx) > width * Self.commitShare || flick {
                if !drag.past { Haptics.select() }
                commit(host, to: pane, forward: forward(dx), toward: dx < 0 ? -width : width, velocity: velocity)
            } else {
                if neighbour(dx) == nil, abs(dx) > 8 { Haptics.tap(.light) }
                settle(host, velocity: velocity)
            }
        case .cancelled:
            guard let drag, drag.host === host else { return }
            self.drag = nil
            settle(host, velocity: 0)
        }
    }

    /// The slot's own ground off (on): the page's backstage under it shows
    /// where the terminal moved away.
    private func uncover(_ open: Bool) {
        for slot in slots.compactMap(\.slot) {
            if open { slot.backgroundColor = .clear } else { paint(slot) }
        }
    }

    /// Springs `host` back to its place, at the lift's speed.
    private func settle(_ host: Hosted, velocity: CGFloat) {
        let peek = self.peek
        let off = peek.map { $0.transform.tx - host.container.transform.tx } ?? 0
        let done = { [weak self] in
            guard let self, self.drag == nil else { return }
            if let peek, self.peek === peek { self.dropPeek() }
            self.uncover(false)
            self.onSwiping?(false)
        }
        guard !reduceMotion else {
            host.container.transform = .identity
            return done()
        }
        spring(host, to: 0, velocity: velocity, alongside: {
            peek?.transform = CGAffineTransform(translationX: off, y: 0)
        }, completion: done)
    }

    /// The structural spring (critically damped) moving `host` to `x` from
    /// where it is, carrying the lift's `velocity`.
    private func spring(_ host: Hosted, to x: CGFloat, velocity: CGFloat, alongside: (() -> Void)? = nil,
                        completion: @escaping () -> Void) {
        let from = host.container.transform.tx
        let distance = x - from
        let relative = abs(distance) > 1 ? velocity / distance : 0
        let timing = UISpringTimingParameters(dampingRatio: 1, initialVelocity: CGVector(dx: relative, dy: 0))
        let animator = UIViewPropertyAnimator(duration: Self.springResponse, timingParameters: timing)
        animator.isUserInteractionEnabled = true
        animator.addAnimations {
            host.container.transform = CGAffineTransform(translationX: x, y: 0)
            alongside?()
        }
        animator.addCompletion { _ in completion() }
        animator.startAnimation()
    }

    /// The swipe switches: `host` flies on toward `x` (off the slot) while
    /// the pane is asked for. A new pane's terminal slides in from that side
    /// when it shows (``show(_:instant:)``); the one terminal of a mosh or
    /// herdr session, switched in place, comes back from the other side
    /// under its switch veil.
    private func commit(_ host: Hosted, to pane: String, forward: Bool, toward x: CGFloat, velocity: CGFloat) {
        committed = (forward, CACurrentMediaTime())
        flying = host
        // The peek of that pane lands where the terminal was.
        let peek = self.peek?.pane == pane ? self.peek : nil
        if peek == nil { dropPeek() }
        landed = peek != nil
        onSwitchPane?(pane, forward)
        let done = { [weak self, weak host] in
            guard let self else { return }
            if let host, self.hosted === host, self.drag == nil, let peek, self.peek === peek, self.landed {
                // The one terminal, switched in place: back under the peek,
                // which lifts once the pane's screen is up.
                host.container.transform = .identity
                peek.superview?.bringSubviewToFront(peek)
                self.liftPeek(over: host.coordinator)
            } else if let host, self.hosted === host, self.drag == nil {
                // Still this terminal: it switched in place (or the new pane
                // isn't here yet). It comes back from the other side.
                if self.reduceMotion {
                    host.container.transform = .identity
                } else {
                    host.container.transform = CGAffineTransform(translationX: -x, y: 0)
                    UIView.animate(withDuration: Self.returnDuration, delay: 0,
                                   options: [.curveEaseInOut, .allowUserInteraction]) {
                        host.container.transform = .identity
                    }
                }
            }
            if self.flying === host { self.flying = nil }
            self.uncover(false)
            self.onSwiping?(false)
        }
        guard !reduceMotion else {
            peek?.transform = .identity
            return done()
        }
        spring(host, to: x, velocity: velocity, alongside: { peek?.transform = .identity }, completion: done)
    }

    // MARK: The peek

    /// The peek of the pane a drag of `dx` goes to, `dx` along: beside the
    /// terminal on the side it uncovers. None without a picture of it (the
    /// backstage names it), or toward nothing (`dx` 0).
    private func followPeek(dx: CGFloat, host: Hosted) {
        guard dx != 0, let pane = neighbour(dx), let slot = host.container.superview else { return dropPeek() }
        // The pane's own terminal, when it has one, draws its picture: its
        // colours, glyphs and ground, as it looks shown. Plain rows (a pane
        // only read) take the live terminal's ground and ink.
        let surface = paneSurface(pane)
        let rows = surface == nil ? (peekRows?(pane) ?? []) : []
        if peek?.pane != pane {
            dropPeek()
            let made: TerminalPeekView
            if let surface {
                made = TerminalPeekView(pane: pane, picture: surface.picture(size: host.container.bounds.size),
                                        ground: surface.ground)
            } else {
                guard !rows.isEmpty else { return }
                made = TerminalPeekView(pane: pane, from: host.surface)
            }
            made.isUserInteractionEnabled = false
            slot.insertSubview(made, belowSubview: host.container)
            peek = made
        }
        guard let peek else { return }
        if !rows.isEmpty, rows != peek.rows {
            // Drawn by the live terminal, in its colours and cells: the
            // pane's own screen, when it replaces the peek, looks the same.
            peek.picture = host.surface.picture(of: rows, size: host.container.bounds.size)
            peek.rows = rows
        }
        peek.bounds = host.container.bounds
        peek.center = host.container.center
        let width = slot.bounds.width
        peek.transform = CGAffineTransform(translationX: dx + (dx < 0 ? width : -width), y: 0)
    }

    /// Lifts the landed peek once the terminal under it shows the pane's
    /// screen: its switch veil came and went, or never came within
    /// ``peekUnveiledFor``; at most ``peekHeldFor``.
    private func liftPeek(over coordinator: CoreTerminalHost.Coordinator?,
                          since: CFTimeInterval = CACurrentMediaTime(), veiled: Bool = false) {
        guard landed, let peek else { return }
        let age = CACurrentMediaTime() - since
        let covered = coordinator?.isCoverPresented ?? false
        let saw = veiled || covered
        if (!covered && (saw || age > Self.peekUnveiledFor)) || age > Self.peekHeldFor {
            landed = false
            self.peek = nil
            UIView.animate(withDuration: 0.15, delay: 0, options: [.allowUserInteraction]) {
                peek.alpha = 0
            } completion: { _ in peek.removeFromSuperview() }
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self, weak coordinator] in
            self?.liftPeek(over: coordinator, since: since, veiled: saw)
        }
    }

    /// The peek out at once.
    private func dropPeek() {
        landed = false
        peek?.removeFromSuperview()
        peek = nil
    }

    /// One pane on (VoiceOver's Next pane / Previous pane, a lift on a view
    /// that doesn't track the drag): nothing past either end.
    @discardableResult
    func step(forward: Bool) -> Bool {
        guard let pane = forward ? neighbours.next : neighbours.previous else { return false }
        committed = (forward, CACurrentMediaTime())
        Haptics.select()
        onSwitchPane?(pane, forward)
        return true
    }

    /// The hint's nudge: the terminal leans ``nudgeDistance`` toward the
    /// pane a swipe would go to, uncovering its name, and springs back.
    func nudge(forward: Bool) {
        guard !reduceMotion, drag == nil, let hosted, hosted.owns,
              (forward ? neighbours.next : neighbours.previous) != nil else { return }
        let ltr = (slot?.effectiveUserInterfaceLayoutDirection ?? .leftToRight) == .leftToRight
        let x = (forward == ltr ? -1 : 1) * Self.nudgeDistance
        uncover(true)
        UIView.animate(withDuration: 0.24, delay: 0.3, options: [.curveEaseOut, .allowUserInteraction]) {
            hosted.container.transform = CGAffineTransform(translationX: x, y: 0)
        } completion: { [weak self, weak hosted] _ in
            guard let self, let hosted, self.drag == nil else { return }
            self.spring(hosted, to: 0, velocity: 0) { [weak self] in
                guard let self, self.drag == nil else { return }
                self.uncover(false)
            }
        }
    }

    // MARK: The overview (B658)

    /// The live terminal shrinking into its cell, or growing out of one:
    /// `base`, inside the owner's 250 ms.
    static let overviewDuration: TimeInterval = 0.24
    /// How long a picked cell waits for its pane's terminal to show (a tmux
    /// switch is a round trip); later, the pane comes the ordinary way.
    static let pickedFor: CFTimeInterval = 1

    /// The overview is open.
    private(set) var overviewOpen = false
    /// Where the shown pane's cell is, in the slot's points.
    private(set) var overviewCard: CGRect?
    /// The cell a pane was picked from (the slot's points), and when.
    private var picked: (pane: String, rect: CGRect, at: CFTimeInterval)?
    /// The shown terminal is in (or on its way into) its cell.
    private(set) var shrunk = false
    private var overviewAnimator: UIViewPropertyAnimator?

    /// The rows of `panes`' screens the stage can read without drawing
    /// anything: each pane's own terminal on a tmux attach; on a mosh or
    /// herdr session only the pane it shows (`current`), from its one
    /// terminal.
    /// Pane `pane`'s own terminal, if the stage's controller has one
    /// already (a tmux attach's panes): what a swipe's peek draws.
    private func paneSurface(_ pane: String) -> CoreTerminalView? {
        guard let target, !target.frozen, case .pane(let controller, _) = target.source else { return nil }
        return controller.existingSurface(for: pane)
    }

    func pictures(of panes: [String], current: String?) -> [String: [String]] {
        guard let target, !target.frozen else { return [:] }
        var pictures: [String: [String]] = [:]
        switch target.source {
        case .pane(let controller, _):
            for pane in panes {
                if let surface = controller.existingSurface(for: pane) { pictures[pane] = surface.screenRows() }
            }
        case .plain(let session):
            if let current, panes.contains(current), let surface = session.coordinator.surface {
                pictures[current] = surface.screenRows()
            }
        }
        return pictures
    }

    /// The overview opened (it waits for the shown pane's cell,
    /// ``overviewAnchor(pane:rect:picked:in:)``) or closed: the terminal
    /// grows back out of its cell, or out of the one just picked.
    func overview(open: Bool) {
        guard open != overviewOpen else { return }
        overviewOpen = open
        if open {
            overviewCard = nil
            picked = nil
            return
        }
        guard let hosted, hosted.owns else {
            stopOverviewMotion()
            shrunk = false
            return
        }
        if let picked, CACurrentMediaTime() - picked.at < Self.pickedFor {
            if shows(picked.pane) {
                // The pane shown, or the one terminal a mosh or herdr
                // session switches in place.
                self.picked = nil
                grow(hosted, from: picked.rect)
            } else {
                // A tmux pane on its way: this one is back in place at
                // once, under the page's fade; the picked one grows from its
                // cell when it shows.
                settleOverview(hosted)
            }
            return
        }
        grow(hosted, from: shrunk ? overviewCard : nil)
    }

    /// Where pane `pane`'s cell is (`rect`, in `surface`'s points): the
    /// shown pane's, which the terminal shrinks into, or the one just
    /// `picked`.
    func overviewAnchor(pane: String, rect: CGRect, picked isPick: Bool, in surface: UIView) {
        guard let slot else { return }
        let cell = slot.convert(rect, from: surface)
        if isPick {
            picked = (pane, cell, CACurrentMediaTime())
            return
        }
        guard overviewOpen else { return }
        overviewCard = cell
        guard let hosted, hosted.owns, target?.frozen != true, drag == nil, !reduceMotion else { return }
        shrunk = true
        animateOverview(hosted, to: Self.fit(hosted.container.bounds.size, at: hosted.container.center, into: cell))
    }

    /// The transform that puts a view of `size` centred at `center` inside
    /// `rect`, its shape kept, centred in it.
    nonisolated static func fit(_ size: CGSize, at center: CGPoint, into rect: CGRect) -> CGAffineTransform {
        guard size.width > 0, size.height > 0, rect.width > 0, rect.height > 0 else { return .identity }
        let scale = min(rect.width / size.width, rect.height / size.height)
        return CGAffineTransform(translationX: rect.midX - center.x, y: rect.midY - center.y)
            .scaledBy(x: scale, y: scale)
    }

    /// Whether pane `pane` is what the slot shows: the pane itself, or any
    /// pane of the one terminal a mosh or herdr session has.
    private func shows(_ pane: String) -> Bool {
        guard let target else { return false }
        switch target.source {
        case .pane(_, let shown): return shown == pane
        case .plain: return true
        }
    }

    /// The cell `target`'s pane was just picked from, taken.
    private func pickedCell(for target: Target) -> CGRect? {
        guard let picked, !reduceMotion, CACurrentMediaTime() - picked.at < Self.pickedFor,
              case .pane(_, let pane) = target.source, pane == picked.pane else { return nil }
        self.picked = nil
        return picked.rect
    }

    /// `host` grows to its place from `rect` (from where it is when it is
    /// in that cell already); at once without a cell or under Reduce
    /// Motion.
    private func grow(_ host: Hosted, from rect: CGRect?) {
        shrunk = false
        guard let rect, !reduceMotion else { return settleOverview(host) }
        if rect != overviewCard || host.container.transform.isIdentity {
            stopOverviewMotion()
            host.container.transform = Self.fit(host.container.bounds.size, at: host.container.center, into: rect)
        }
        animateOverview(host, to: .identity)
    }

    /// `host` in its place at once.
    private func settleOverview(_ host: Hosted) {
        stopOverviewMotion()
        shrunk = false
        host.container.transform = .identity
    }

    /// The overview's motion stops where it is.
    private func stopOverviewMotion() {
        guard let animator = overviewAnimator else { return }
        overviewAnimator = nil
        // Stopped without finishing, the properties stay where they were.
        animator.stopAnimation(true)
    }

    /// `host` moves to `transform` from where it is (an interrupted motion
    /// carries on from its place): touches go where it is going at once.
    private func animateOverview(_ host: Hosted, to transform: CGAffineTransform) {
        stopOverviewMotion()
        let animator = UIViewPropertyAnimator(duration: Self.overviewDuration, curve: .easeInOut) {
            host.container.transform = transform
        }
        animator.isUserInteractionEnabled = true
        animator.addCompletion { [weak self, weak animator] _ in
            guard let self, let animator, self.overviewAnimator === animator else { return }
            self.overviewAnimator = nil
        }
        overviewAnimator = animator
        animator.startAnimation()
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

    /// `host`'s terminal takes the keyboard from the terminal holding it, in
    /// this turn: between two text views of one window it never goes down.
    /// Not while the keyboard is to go (`resign`), nor into a terminal off
    /// screen.
    private func handKeyboard(to host: Hosted) {
        guard focus != .resign, host.owns, host.terminal.window != nil, !host.surface.isFinding else { return }
        let took = host.terminal.becomeFirstResponder()
        Log.ui.notice("terminal stage: \(host.key, privacy: .public) took the keyboard from the terminal it replaced: \(took, privacy: .public)")
    }

    /// The slot's focus rules: take it (now, and back if something stole
    /// it), leave it, or give it up.
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

    /// The plain path's look: font, colours and cursor, whatever changed.
    func apply(appearance: Appearance) {
        guard appearance != self.appearance else { return }
        self.appearance = appearance
        for slot in slots.compactMap(\.slot) { paint(slot) }
        guard let hosted, hosted.owns, case .plain? = target?.source, hosted.coordinator != nil
        else { return }
        hosted.surface.apply(font: TerminalFont.font(id: appearance.fontName, size: CGFloat(appearance.fontSize)))
        style(hosted.surface)
    }

    private func style(_ surface: CoreTerminalView) {
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

/// The screen of a pane either side, as the swipe brings it in: a picture
/// drawn as a terminal draws (its colours, glyphs, grid and ground), else
/// its rows as plain text in the terminal's own font, ground and cell
/// height, laid out where that terminal lays its grid. Never touched.
final class TerminalPeekView: UIView {
    let pane: String
    /// The rows the picture was drawn from (a pane only read).
    var rows: [String] = [] { didSet { setNeedsDisplay() } }
    private let font: UIFont
    private let lineHeight: CGFloat
    private let originX: CGFloat
    private let ink: UIColor
    /// Drawn instead of `rows` as text.
    var picture: UIImage? { didSet { setNeedsDisplay() } }

    /// A pane only read: its rows drawn by the live terminal
    /// (``CoreTerminalView/picture(of:size:)``) once they are set, else as
    /// plain text in that terminal's font, ground and ink.
    init(pane: String, from surface: CoreTerminalView) {
        self.pane = pane
        font = surface.font
        lineHeight = max(surface.cellSize.height, 1)
        originX = surface.gridOriginX
        ink = UIColor(cgColor: coreCGColor(surface.style.foreground))
        picture = nil
        super.init(frame: .zero)
        backgroundColor = surface.ground
        contentMode = .redraw
        accessibilityElementsHidden = true
    }

    /// The pane's terminal as it draws itself (``CoreTerminalView/picture(size:)``).
    init(pane: String, picture: UIImage, ground: UIColor) {
        self.pane = pane
        font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        lineHeight = 1
        originX = 0
        ink = .label
        self.picture = picture
        super.init(frame: .zero)
        backgroundColor = ground
        contentMode = .redraw
        accessibilityElementsHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override func draw(_ rect: CGRect) {
        if let picture {
            picture.draw(at: .zero)
            return
        }
        let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: ink]
        for (i, row) in rows.enumerated() {
            let y = CGFloat(i) * lineHeight
            if y > bounds.height { break }
            guard y + lineHeight >= rect.minY, y <= rect.maxY, !row.isEmpty else { continue }
            (row as NSString).draw(at: CGPoint(x: originX, y: y), withAttributes: attributes)
        }
    }
}
