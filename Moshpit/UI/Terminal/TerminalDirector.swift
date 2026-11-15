import SwiftUI
import UIKit

extension View {
    /// The debug director's hooks on the terminal screen (W11g's rig): what
    /// it asks of the keyboard goes through the screen's own intent, as the
    /// keyboard toggle does. Nothing in a Release build.
    func terminalDirector(keyboard: @escaping (Bool) -> Void,
                          present: @escaping (String) -> Void = { _ in }) -> some View {
        #if DEBUG
        modifier(TerminalDirectorHooks(keyboard: keyboard, present: present))
        #else
        self
        #endif
    }
}

#if DEBUG

/// Hears the director's keyboard requests.
private struct TerminalDirectorHooks: ViewModifier {
    let keyboard: (Bool) -> Void
    let present: (String) -> Void

    func body(content: Content) -> some View {
        content
            .onReceive(NotificationCenter.default.publisher(for: TerminalDirector.keyboardRequest)) { note in
                keyboard((note.object as? Bool) == true)
            }
            .onReceive(NotificationCenter.default.publisher(for: TerminalDirector.presentRequest)) { note in
                present(note.object as? String ?? "")
            }
            .onAppear { TerminalDirector.startIfAsked() }
    }
}

/// W11g's director (`-MOSHPIT_TERMINAL_DIRECTOR <mode>`): drives the terminal
/// screen the way a person would and measures it. Evidence goes to stderr
/// as `[w11g] …` lines (`devicectl device process launch --console`).
/// Spike G5's method.
///
/// - `keyboard [-MOSHPIT_DIRECTOR_CYCLES n]`: raises and puts away the
///   keyboard through the screen's own intent, n times; per transition: the
///   pane's sizes, the terminal's frame changes and grid resizes, whether
///   the cover was up, per display frame whether the visible terminal moved
///   or changed while uncovered, and how far ahead of (or behind) the
///   keyboard the pane's bottom edge — the shortcut bar's top — moves.
/// - `switch [-MOSHPIT_DIRECTOR_CYCLES n]`: a tmux session's first two
///   windows, alternately: from the switch to the new pane on screen, and
///   to its resynced frame revealed.
/// - `shots`: holds the keyboard down, then up, announcing each hold
///   (`HOLD <name>`) for a screenshot.
/// - `sheets`: presents the screen's sheets one after another, each held
///   for a screenshot.
/// - `wait`: nothing for a while (`-MOSHPIT_ECHO_PROBE n` times the echo).
/// - `suite`: shots, keyboard and switch in one launch.
///
/// The one-off investigations it also ran (rekey, burst, paths, images,
/// imagerepeat, foldertap, imagepanes) went with E6a; their findings are in
/// the bugs they settled.
@MainActor
enum TerminalDirector {
    static let keyboardRequest = Notification.Name("MoshpitTerminalDirectorKeyboard")
    /// What the director asks the screen to present (`windows`, `sessions`,
    /// `panes`, `switch`, `diagnostics`) or put away (`dismiss`).
    static let presentRequest = Notification.Name("MoshpitTerminalDirectorPresent")
    private static var started = false

    static func say(_ line: String) {
        FileHandle.standardError.write(Data("[w11g] \(line)\n".utf8))
    }

    private static func arg(_ flag: String) -> String? {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: flag), i + 1 < args.count else { return nil }
        return args[i + 1]
    }

    static func startIfAsked() {
        guard !started, let mode = arg("-MOSHPIT_TERMINAL_DIRECTOR") else { return }
        started = true
        Task { await run(mode) }
    }

    private static func sleep(_ seconds: Double) async {
        try? await Task.sleep(for: .milliseconds(Int(seconds * 1000)))
    }

    @discardableResult
    private static func wait(_ timeout: Double, _ condition: () -> Bool) async -> Bool {
        let deadline = CACurrentMediaTime() + timeout
        while CACurrentMediaTime() < deadline {
            if condition() { return true }
            await sleep(0.02)
        }
        return condition()
    }

    private static var session: SessionHub.ActiveSession? { RustCore.visibleSession }

    /// The terminal on screen and its coordinator.
    fileprivate static var shown: (CoreTerminalView, CoreTerminalHost.Coordinator)? {
        guard let session else { return nil }
        let coordinator = session.tmuxController?.activeCoordinator ?? session.coordinator
        guard let surface = coordinator.surface, surface.view.window != nil else { return nil }
        return (surface, coordinator)
    }

    private static func keyboard(_ up: Bool) {
        NotificationCenter.default.post(name: keyboardRequest, object: up)
    }

    private static func run(_ mode: String) async {
        say("director \(mode) ui=nsh device \(UIDevice.current.model) iOS \(UIDevice.current.systemVersion)")
        // Live, attached, painted, then a beat.
        let ready = await wait(60) {
            guard let session, shown != nil else { return false }
            let tmuxReady = session.tmuxController.map { $0.snapshot.isAttached } ?? true
            return session.viewModel.connState == .live && tmuxReady && !session.awaitingFirstFrame
        }
        guard ready, let session else { return say("FAIL no live terminal on screen after 60 s") }
        say("session \(session.rust == nil ? "swift" : "rust") \(session.connection.connectionProtocol.rawValue)+\(session.connection.multiplexer.rawValue)")
        await sleep(3)
        switch mode {
        case "keyboard": await keyboardCycles(ui: "nsh")
        case "switch": await switches(ui: "nsh")
        case "shots": await shots()
        case "sheets": await sheets()
        case "wait": await sleep(Double(arg("-MOSHPIT_DIRECTOR_HOLD") ?? "60") ?? 60)
        case "suite":
            // One launch, one connect: the shots, the keyboard and the
            // switches.
            await shots()
            await keyboardCycles(ui: "nsh")
            if session.tmuxControl != nil { await switches(ui: "nsh") }
        default: say("FAIL unknown director mode \(mode)")
        }
        say("director \(mode) done")
    }

    // MARK: Keyboard

    private static func keyboardCycles(ui: String) async {
        let cycles = Int(arg("-MOSHPIT_DIRECTOR_CYCLES") ?? "8") ?? 8
        var records: [KeyboardProbe.Record] = []
        keyboard(false)
        await sleep(1.5)
        for cycle in 1...cycles {
            for up in [true, false] {
                guard let (surface, coordinator) = shown, let container = coordinator.hostContainer,
                      let window = surface.view.window else { return say("FAIL no terminal") }
                let probe = KeyboardProbe(surface: surface, container: container, window: window)
                let record = await probe.measure(kind: up ? "up" : "down") { keyboard(up) }
                say("transition \(cycle).\(record.kind) \(record.line)")
                records.append(record)
                await sleep(0.6)
            }
        }
        for kind in ["up", "down"] {
            let set = records.filter { $0.kind == kind }
            say("summary \(kind) n=\(set.count) " + KeyboardProbe.Record.summary(set))
        }
        if let shifts = TerminalNSHModel.current?.keyboardAlign?.shifts, !shifts.isEmpty {
            say("keyboard align: \(shifts.count) animations started with the keyboard, later by median "
                + String(format: "%.1f", median(shifts)) + " ms")
        } else if TerminalNSHModel.current != nil {
            say("keyboard align: off or nothing to move")
        }
        let jumpy = records.filter { $0.visibleJumps > 0 }.count
        say("RESULT keyboard ui=\(ui): \(records.count) transitions, \(jumpy) with a visible jump,"
            + " grid resizes per transition max \(records.map(\.grids).max() ?? 0),"
            + " pane edge leads the keyboard by median \(String(format: "%.1f", median(records.map(\.leadMedian)))) ms")
    }

    // MARK: Pane switches

    private static func switches(ui: String) async {
        guard let control = session?.tmuxControl else { return say("FAIL switch needs tmux") }
        let windows = control.snapshot.sortedWindows
        guard windows.count >= 2 else { return say("FAIL switch needs two windows, has \(windows.count)") }
        let cycles = Int(arg("-MOSHPIT_DIRECTOR_CYCLES") ?? "10") ?? 10
        var shownMs: [Double] = []
        var revealedMs: [Double] = []
        var repaints: [Int] = []
        let view = shown == nil ? "-" : "core"
        for cycle in 0..<(cycles * 2) {
            let target = windows[cycle % 2]
            let before = shown?.0
            let start = CACurrentMediaTime()
            let watch = SwitchProbe()
            control.selectWindow(target.id)
            let swapped = await wait(3) { shown.map { $0.0 !== before } ?? false }
            let t1 = CACurrentMediaTime()
            guard swapped, let (_, coordinator) = shown else {
                watch.stop()
                say("switch \(cycle): the pane never changed"); continue
            }
            let revealed = await wait(5) { !coordinator.isCoverPresented }
            let t2 = CACurrentMediaTime()
            // Half a second after the reveal: anything that still changes on
            // screen is a repaint the person watches.
            await sleep(0.5)
            watch.stop()
            shownMs.append((t1 - start) * 1000)
            if revealed { revealedMs.append((t2 - start) * 1000) }
            repaints.append(watch.visibleChanges)
            say(String(format: "switch %d to %@: on screen %.1f ms, revealed %@, uncovered frames that changed %d",
                       cycle, target.id, (t1 - start) * 1000,
                       revealed ? String(format: "%.1f ms", (t2 - start) * 1000) : "never", watch.visibleChanges))
            await sleep(0.3)
        }
        say("RESULT switch ui=\(ui) view=\(view): n=\(shownMs.count) on screen p50 \(fmt(percentile(shownMs, 0.5))) p95 \(fmt(percentile(shownMs, 0.95))) ms;"
            + " revealed p50 \(fmt(percentile(revealedMs, 0.5))) p95 \(fmt(percentile(revealedMs, 0.95))) ms;"
            + " switches with a visible repaint \(repaints.filter { $0 > 0 }.count) (most changed frames \(repaints.max() ?? 0))")
    }

    /// Per display frame of a switch: whether the pane on screen is
    /// uncovered (no veil) and its text changed since the frame before — a
    /// row-by-row repaint shows as several, a switch that only ever shows
    /// whole frames as none.
    @MainActor
    private final class SwitchProbe {
        private var link: CADisplayLink?
        private var last: (surface: ObjectIdentifier, text: Int)?
        private(set) var visibleChanges = 0

        init() {
            let link = CADisplayLink(target: Target(self), selector: #selector(Target.frame(_:)))
            link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
            link.add(to: .main, forMode: .common)
            self.link = link
        }

        func stop() {
            link?.invalidate()
            link = nil
        }

        fileprivate func frame() {
            guard let (surface, coordinator) = TerminalDirector.shown else { return }
            let text = (0..<surface.grid.rows).map(surface.rowText).hashValue
            let id = ObjectIdentifier(surface)
            defer { last = (id, text) }
            guard let last, last.surface == id, last.text != text, !coordinator.isCoverPresented else { return }
            visibleChanges += 1
        }

        private final class Target: NSObject {
            weak var probe: SwitchProbe?
            init(_ probe: SwitchProbe) { self.probe = probe }
            @objc func frame(_ link: CADisplayLink) { MainActor.assumeIsolated { probe?.frame() } }
        }
    }

    // MARK: Screenshots

    private static func shots() async {
        let hold = Double(arg("-MOSHPIT_DIRECTOR_HOLD") ?? "8") ?? 8
        keyboard(false)
        await sleep(2)
        say("HOLD keyboard-down \(Int(hold)) s")
        await sleep(hold)
        keyboard(true)
        await sleep(2)
        say("HOLD keyboard-up \(Int(hold)) s")
        await sleep(hold)
        keyboard(false)
        await sleep(1)
    }

    // MARK: Sheets (W11h)

    /// The breadcrumb's pickers, the pill's question and (mosh) its
    /// counters, each held for a screenshot, the keyboard down.
    private static func sheets() async {
        let hold = Double(arg("-MOSHPIT_DIRECTOR_HOLD") ?? "6") ?? 6
        keyboard(false)
        await sleep(2)
        var list = ["switch", "install", "folder"]
        if session?.tmuxControl != nil || session?.herdrControl != nil { list = ["windows", "sessions", "panes"] + list }
        if session?.connection.connectionProtocol == .mosh { list.append("diagnostics") }
        if let only = arg("-MOSHPIT_DIRECTOR_ONLY") { list = list.filter { $0 == only } }
        for what in list {
            present(what)
            await sleep(2)
            say("HOLD sheet-\(what) \(Int(hold)) s")
            await sleep(hold)
            present("dismiss")
            await sleep(1.5)
        }
    }

    private static func present(_ what: String) {
        // The NSH pill asks on its own action sheet: the director presses it
        // the way VoiceOver does.
        if what == "switch", TerminalNSHModel.current != nil,
           let window = UIApplication.shared.connectedScenes.compactMap({ ($0 as? UIWindowScene)?.keyWindow }).first,
           let pill = TransitionFinder.find("terminal-transport-switch", in: window) {
            _ = pill.accessibilityActivate()
            return
        }
        if what == "dismiss",
           let window = UIApplication.shared.connectedScenes.compactMap({ ($0 as? UIWindowScene)?.keyWindow }).first {
            var top = window.rootViewController
            while let next = top?.presentedViewController { top = next }
            if top is UIAlertController { top?.dismiss(animated: true) }
        }
        NotificationCenter.default.post(name: presentRequest, object: what)
    }

    // MARK: Bursts (T3)

    static func percentile(_ values: [Double], _ q: Double) -> Double? {
        guard !values.isEmpty else { return nil }
        let sorted = values.sorted()
        return sorted[min(sorted.count - 1, Int((Double(sorted.count - 1) * q).rounded()))]
    }

    static func median(_ values: [Double]) -> Double { percentile(values, 0.5) ?? 0 }

    static func fmt(_ value: Double?) -> String {
        value.map { String(format: "%.1f", $0) } ?? "-"
    }

    static func fmt(_ value: CGFloat) -> String { String(format: "%.1f", Double(value)) }
}

/// One keyboard transition, frame by frame (spike G5's `TransitionProbe`,
/// on the app's own views: the pane's container, whichever UI lays it out).
@MainActor
final class KeyboardProbe {
    struct Record {
        let kind: String
        var start: CFTimeInterval = 0
        var frames = 0
        var coveredFrames = 0
        var visibleJumps = 0
        /// Distinct heights the pane's container passed through.
        var containerHeights: [CGFloat] = []
        var terminalFrameChanges = 0
        var grids = 0
        /// The keyboard's top over the transition.
        var kbMin: CGFloat = .infinity
        var kbMax: CGFloat = -.infinity
        /// The pane's bottom edge (the shortcut bar's top) against the
        /// keyboard: at matching progress, how far ahead it is (ms).
        var leadMedian: Double = 0
        var leadMax: Double = 0
        var leadSamples = 0
        var offCurve: CGFloat = 0
        /// When the move ended, from the action (ms): the pane's last change.
        var settledMs: Double = 0

        var line: String {
            "pane heights \(containerHeights.count) [\(containerHeights.map { String(format: "%.0f", $0) }.joined(separator: ">"))],"
                + " terminal frame changes \(terminalFrameChanges), grid resizes \(grids),"
                + " frames \(frames) (covered \(coveredFrames)), visible jumps \(visibleJumps),"
                + String(format: " keyboard top %.0f…%.0f, edge leads by median %.1f max %.1f ms (%d frames), off-curve max %.1f pt, settled %.0f ms",
                         kbMin, kbMax, leadMedian, leadMax, leadSamples, offCurve, settledMs)
        }

        static func summary(_ set: [Record]) -> String {
            func stat(_ values: [Double]) -> String {
                guard !values.isEmpty else { return "-" }
                return String(format: "mean %.1f max %.1f", values.reduce(0, +) / Double(values.count),
                              values.max() ?? 0)
            }
            return "pane heights \(stat(set.map { Double($0.containerHeights.count) }));"
                + " terminal frame changes \(stat(set.map { Double($0.terminalFrameChanges) }));"
                + " grid resizes \(stat(set.map { Double($0.grids) }));"
                + " visible jumps \(stat(set.map { Double($0.visibleJumps) }));"
                + " edge lead ms \(stat(set.map(\.leadMedian)));"
                + " off-curve pt \(stat(set.map { Double($0.offCurve) }));"
                + " settled ms \(stat(set.map(\.settledMs)))"
        }
    }

    private let surface: CoreTerminalView
    private let terminal: UIView
    private let container: UIView
    private let window: UIWindow
    private var record = Record(kind: "")
    private var previous: (anchor: CGFloat, text: UInt64)?
    private var lastFrame: CGRect = .null
    private var lastGrid = ""
    private var lastChange: CFTimeInterval = 0
    private var samples: [(t: CFTimeInterval, edge: CGFloat, kb: CGFloat?)] = []

    init(surface: CoreTerminalView, container: UIView, window: UIWindow) {
        self.surface = surface
        self.terminal = surface.view
        self.container = container
        self.window = window
    }

    func measure(kind: String, _ action: () -> Void) async -> Record {
        record = Record(kind: kind)
        record.start = CACurrentMediaTime()
        previous = nil
        samples = []
        lastFrame = terminal.frame
        lastGrid = grid()
        lastChange = record.start
        let link = CADisplayLink(target: ProbeTarget(self), selector: #selector(ProbeTarget.frame(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        action()
        // Until the pane has been still a while (and at most 4 s).
        let deadline = CACurrentMediaTime() + 4
        while CACurrentMediaTime() < deadline {
            try? await Task.sleep(for: .milliseconds(20))
            if CACurrentMediaTime() - lastChange > 0.6, CACurrentMediaTime() - record.start > 0.8 { break }
        }
        link.invalidate()
        record.settledMs = (lastChange - record.start) * 1000
        curve()
        return record
    }

    /// The grid, "COLSxROWS".
    private func grid() -> String {
        "\(surface.grid.cols)x\(surface.grid.rows)"
    }

    /// The text on screen, row by row.
    private func visibleLines() -> [String] {
        (0..<surface.grid.rows).map(surface.rowText)
    }

    fileprivate func frame() {
        record.frames += 1
        let now = CACurrentMediaTime()
        // The cover: a plate over the terminal inside its container.
        let covered = container.subviews.contains { $0 !== terminal && ($0.layer.presentation()?.opacity ?? $0.layer.opacity) >= 0.99 }
        if covered { record.coveredFrames += 1 }
        let layer = terminal.layer.presentation() ?? terminal.layer
        let anchor = layer.convert(CGPoint.zero, to: nil).y
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for line in visibleLines() {
            for byte in line.utf8 {
                hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3
            }
            hash = (hash ^ 10) &* 0x100_0000_01b3
        }
        if let previous, !covered, abs(previous.anchor - anchor) >= 0.5 || previous.text != hash {
            record.visibleJumps += 1
        }
        previous = (anchor, hash)
        // The pane's container: its height, the terminal's frame, the grid.
        let box = (container.layer.presentation() ?? container.layer)
        let height = box.bounds.height
        if record.containerHeights.last.map({ abs($0 - height) >= 0.5 }) ?? true {
            record.containerHeights.append(height)
            lastChange = now
        }
        if terminal.frame != lastFrame {
            record.terminalFrameChanges += 1
            lastFrame = terminal.frame
            lastChange = now
        }
        let grid = grid()
        if grid != lastGrid {
            record.grids += 1
            lastGrid = grid
            lastChange = now
        }
        // The shortcut bar's top where it is drawn, mid-animation: NSH's bar
        // view by its test id; under SwiftUI (no id) the pane container's
        // bottom, which SwiftUI lays out frame by frame right above it.
        let edge: CGFloat
        if let bar = Self.find(id: "terminal-shortcut-bar", in: window) {
            let layer = bar.layer.presentation() ?? bar.layer
            edge = layer.convert(CGPoint.zero, to: nil).y
        } else {
            edge = box.convert(CGPoint(x: 0, y: box.bounds.maxY), to: nil).y
        }
        var kbTop: CGFloat?
        if let host = Self.keyboardHost() {
            let kb = host.layer.presentation() ?? host.layer
            kbTop = kb.convert(CGPoint.zero, to: nil).y
        }
        samples.append((now, edge, kbTop))
    }

    /// The pane edge's progress against the keyboard's, frame by frame.
    private func curve() {
        let kbs = samples.compactMap(\.kb)
        record.kbMin = kbs.min() ?? .infinity
        record.kbMax = kbs.max() ?? -.infinity
        guard let first = samples.first, let last = samples.last,
              let kb0 = kbs.first, let kb1 = kbs.last else { return }
        let edgeSpan = last.edge - first.edge
        let kbSpan = kb1 - kb0
        guard abs(edgeSpan) > 1, abs(kbSpan) > 1 else { return }
        let kbTimeline = samples.compactMap { s in s.kb.map { (t: s.t, p: ($0 - kb0) / kbSpan) } }
        var leads: [Double] = []
        for s in samples {
            let p = (s.edge - first.edge) / edgeSpan
            guard p > 0.05, p < 0.95 else { continue }
            if let i = kbTimeline.firstIndex(where: { $0.p >= p }), i > 0 {
                let a = kbTimeline[i - 1], b = kbTimeline[i]
                let t = a.t + (b.t - a.t) * Double((p - a.p) / max(0.0001, b.p - a.p))
                leads.append((t - s.t) * 1000)
            }
        }
        record.leadSamples = leads.count
        // How far off the keyboard's curve the bar is, in points (G5's
        // measure): the progress gap times the bar's travel.
        for s in samples {
            guard let kb = s.kb else { continue }
            let gap = abs((s.edge - first.edge) / edgeSpan - (kb - kb0) / kbSpan) * abs(edgeSpan)
            record.offCurve = max(record.offCurve, gap)
        }
        if !leads.isEmpty {
            let sorted = leads.sorted()
            record.leadMedian = sorted[sorted.count / 2]
            record.leadMax = sorted.last ?? 0
        }
    }

    /// The keyboard's host view in the keyboard's window: its frame moves
    /// with the keyboard.
    static func keyboardHost() -> UIView? {
        for scene in UIApplication.shared.connectedScenes {
            guard let scene = scene as? UIWindowScene else { continue }
            for window in scene.windows {
                let name = String(describing: type(of: window))
                guard name.contains("Keyboard") || name.contains("TextEffects") else { continue }
                if let host = find("InputSetHostView", in: window), host.bounds.height > 50 { return host }
            }
        }
        return nil
    }

    static func find(id: String, in view: UIView) -> UIView? {
        if view.accessibilityIdentifier == id { return view }
        for sub in view.subviews {
            if let found = find(id: id, in: sub) { return found }
        }
        return nil
    }

    private static func find(_ fragment: String, in view: UIView) -> UIView? {
        if String(describing: type(of: view)).contains(fragment) { return view }
        for sub in view.subviews {
            if let found = find(fragment, in: sub) { return found }
        }
        return nil
    }
}

private final class ProbeTarget: NSObject {
    private weak var probe: KeyboardProbe?
    init(_ probe: KeyboardProbe) { self.probe = probe }
    @objc func frame(_ link: CADisplayLink) {
        MainActor.assumeIsolated { probe?.frame() }
    }
}
#endif

#if DEBUG
/// Finds a view by its accessibility identifier.
@MainActor
enum TransitionFinder {
    static func find(_ id: String, in view: UIView) -> UIView? {
        if view.accessibilityIdentifier == id { return view }
        for sub in view.subviews {
            if let hit = find(id, in: sub) { return hit }
        }
        return nil
    }
}
#endif
