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
/// screen the way a person would and measures it, on whichever UI draws the
/// screen (SwiftUI, or NSH with `-MOSHPIT_NSH_TERMINAL 1`) and whichever path
/// runs the session (`-MOSHPIT_RUST_SESSION 0/1`), so the two can be
/// compared on one device. Evidence goes to stderr as `[w11g] …` lines
/// (`devicectl device process launch --console`). Spike G5's method.
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
/// - `rekey` (NSH): with the keyboard up, the screen makes the terminal's
///   element again: the same view, still first responder, the keyboard
///   still.
/// - `wait`: nothing for a while (`-MOSHPIT_ECHO_PROBE n` times the echo).
/// - `suite`: shots, keyboard, switch and (NSH) rekey in one launch.
/// - `burst` (T3): `cat`s a big file in the pane (`-MOSHPIT_DIRECTOR_BURST
///   burst.txt`, relative to its directory) n times and measures, per
///   display frame until the end marker shows, the main thread's longest
///   gap (the stall a person feels) and how long the burst took; on the
///   core's view also the frames that changed what shows and the longest
///   gap between two (a frozen screen). `-MOSHPIT_DIRECTOR_BACKLOG s` (T4)
///   blocks the main thread s seconds after the command, so the session's
///   channel fills as it does while the app is busy or away, and measures
///   the catch-up from the moment the main thread is free again.
/// - `paths` (T3): prints a path to a file the host has
///   (`-MOSHPIT_DIRECTOR_PATH out/a.png`, relative to the pane's directory)
///   and one it hasn't, and times until the first is a link on the terminal
///   on screen (the core's view) while the second stays text.
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
    fileprivate static var shown: (TerminalSurface, SwiftTerminalView.Coordinator)? {
        guard let session else { return nil }
        let coordinator = session.tmuxController?.activeCoordinator ?? session.coordinator
        guard let surface = coordinator.surface, surface.view.window != nil else { return nil }
        return (surface, coordinator)
    }

    private static func keyboard(_ up: Bool) {
        NotificationCenter.default.post(name: keyboardRequest, object: up)
    }

    private static func run(_ mode: String) async {
        let ui = NSHScreen.terminal.isOn ? "nsh" : "swiftui"
        say("director \(mode) ui=\(ui) device \(UIDevice.current.model) iOS \(UIDevice.current.systemVersion)")
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
        case "keyboard": await keyboardCycles(ui: ui)
        case "switch": await switches(ui: ui)
        case "shots": await shots()
        case "sheets": await sheets()
        case "rekey": await rekey()
        case "paths": await paths()
        case "burst": await bursts()
        case "images": await images()
        case "imagerepeat": await imageRepeats()
        case "foldertap": await folderTap()
        case "imagepanes": await imagePanes()
        case "wait": await sleep(Double(arg("-MOSHPIT_DIRECTOR_HOLD") ?? "60") ?? 60)
        case "suite":
            // One launch, one connect: the shots, the keyboard, the switches,
            // and on the NSH screen the rekey.
            await shots()
            await keyboardCycles(ui: ui)
            if session.tmuxControl != nil { await switches(ui: ui) }
            if TerminalNSHModel.current != nil { await rekey() }
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
        let view = shown.map { $0.0 is CoreTerminalView ? "core" : "other" } ?? "-"
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

    private static func bursts() async {
        let file = arg("-MOSHPIT_DIRECTOR_BURST") ?? "burst.txt"
        let cycles = Int(arg("-MOSHPIT_DIRECTOR_CYCLES") ?? "3") ?? 3
        let backlog = Double(arg("-MOSHPIT_DIRECTOR_BACKLOG") ?? "0") ?? 0
        guard let session, let (surface, _) = shown else { return say("FAIL burst: no terminal") }
        let core = surface as? CoreTerminalView
        var gaps: [Double] = []
        var durations: [Double] = []
        var frozen: [Double] = []
        var rates: [Double] = []
        // Where the main thread's time went: the view's longest apply and
        // turn, and this probe's own longest look at the screen.
        var applies: [Double] = []
        var turns: [Int] = []
        var looks: [Double] = []
        // Whether the atlas's common CJK is in before the first screen.
        if let core { say("before the first: " + core.atlasSummary) }
        for cycle in 1...cycles {
            let marker = "BURST-\(cycle)-END"
            // The marker as printed, never as typed (`%s`).
            session.sendInput(Data("cat \(file); printf 'BURST-%s-END\\n' \(cycle)\r".utf8))
            if backlog > 0 {
                // Busy (or away): the session's channel fills meanwhile.
                Thread.sleep(forTimeInterval: backlog)
            }
            let probe = FrameGapProbe()
            let start = CACurrentMediaTime()
            // The frames that changed what shows, while it runs.
            var shown: [CFTimeInterval] = []
            var encoded = core?.rowsEncoded ?? 0
            core?.resetFrameCosts()
            core?.onFrame = { [weak core] in
                guard let core, core.rowsEncoded != encoded else { return }
                encoded = core.rowsEncoded
                shown.append(CACurrentMediaTime())
            }
            var look = 0.0
            let done = await wait(120) {
                let at = CACurrentMediaTime()
                defer { look = max(look, (CACurrentMediaTime() - at) * 1000) }
                // Where the marker lands: just above the prompt. (Every row
                // every 20 ms cost the main thread up to 20 ms a look in a
                // debug build, inside the gaps this measures.)
                let bottom = min(surface.cursor.row, surface.grid.rows - 1)
                return (max(0, bottom - 3)...max(0, bottom)).contains { surface.rowText($0) == marker }
            }
            core?.onFrame = nil
            probe.stop()
            let end = CACurrentMediaTime()
            let ms = (end - start) * 1000
            guard done else { say("burst \(cycle): never ended"); continue }
            gaps.append(probe.longest)
            durations.append(ms)
            // The longest the screen stood still: from the start to the
            // first new frame, between two, to the end.
            let marks = [start] + shown + [end]
            let still = zip(marks.dropFirst(), marks).map { ($0 - $1) * 1000 }.max() ?? 0
            let fps = Double(shown.count) / max(end - start, 0.001)
            looks.append(look)
            var costs = ""
            if let core {
                frozen.append(still)
                rates.append(fps)
                applies.append(core.frameCosts.apply)
                turns.append(core.frameCosts.framesInTurn)
                costs = "; main thread per frame: " + core.frameCosts.summary
            }
            say(String(format: "burst %d: %.0f ms, main thread longest gap %.1f ms, p99 %.1f ms over %d frames;"
                       + " %d frames changed the screen (%.0f/s), longest still %.0f ms",
                       cycle, ms, probe.longest, probe.p99, probe.count, shown.count, fps, still)
                + costs + String(format: "; the probe's look %.1f ms", look))
            await sleep(1.5)
        }
        let view = surface is CoreTerminalView ? "core" : "other"
        if let core { say(core.atlasSummary) }
        say("RESULT burst view=\(view) backlog=\(backlog)s: n=\(gaps.count) longest gap max \(fmt(gaps.max())) median \(fmt(percentile(gaps, 0.5))) ms;"
            + " duration median \(fmt(percentile(durations, 0.5))) ms;"
            + " screen frames/s median \(fmt(percentile(rates, 0.5))), longest still max \(fmt(frozen.max())) ms;"
            + " apply max \(fmt(applies.max())) ms, frames a turn max \(turns.max() ?? 0), probe look max \(fmt(looks.max())) ms")
    }

    /// The main thread's display-frame gaps while it runs.
    @MainActor
    private final class FrameGapProbe {
        private var link: CADisplayLink?
        private var last: CFTimeInterval = 0
        private var gaps: [Double] = []

        init() {
            let link = CADisplayLink(target: Target(self), selector: #selector(Target.frame(_:)))
            link.add(to: .main, forMode: .common)
            self.link = link
            // From now: work done before the first tick (the first frame
            // after a backlog's wait) is a gap too.
            last = CACurrentMediaTime()
        }

        func stop() {
            link?.invalidate()
            link = nil
        }

        var longest: Double { gaps.max() ?? 0 }
        var count: Int { gaps.count }
        var p99: Double { TerminalDirector.percentile(gaps, 0.99) ?? 0 }

        fileprivate func frame(_ link: CADisplayLink) {
            if last > 0 { gaps.append((link.timestamp - last) * 1000) }
            last = link.timestamp
        }

        private final class Target: NSObject {
            weak var probe: FrameGapProbe?
            init(_ probe: FrameGapProbe) { self.probe = probe }
            @objc func frame(_ link: CADisplayLink) { MainActor.assumeIsolated { probe?.frame(link) } }
        }
    }

    // MARK: Images (T6)

    /// Kitty, sixel and iTerm2 images from the rig (`-MOSHPIT_DIRECTOR_IMAGES`
    /// names the folder of mkimages.py's files: four quadrants, red, green /
    /// blue, yellow), each on a cleared screen and checked by its pixels;
    /// then a 1200 x 3000 image, scrolled past and back, and what it costs
    /// in memory until the screen and the history are cleared.
    private static func images() async {
        guard let session, let (surface, _) = shown, let core = surface as? CoreTerminalView else {
            return say("FAIL images: not the core's view")
        }
        let dir = arg("-MOSHPIT_DIRECTOR_IMAGES") ?? "."
        let red = (255, 0, 0), green = (0, 255, 0), blue = (0, 0, 255), yellow = (255, 255, 0)
        var failed = 0
        for p in ["kitty", "sixel", "iterm2"] {
            guard await shell(session, core, "cat \(dir)/img-\(p).txt", tag: p) else {
                say("FAIL images \(p): never ended")
                failed += 1
                continue
            }
            await sleep(0.5)
            let v = checkImage(core, name: p, at: [(0.25, 0.25, red), (0.75, 0.25, green),
                                                   (0.25, 0.75, blue), (0.75, 0.75, yellow)])
            if !v.ok { failed += 1 }
            say("images \(p): \(v.text)")
        }
        // The large one, and 300 lines after it.
        let before = footprint()
        guard await shell(session, core, "cat \(dir)/img-big.txt; seq 1 300 | sed 's/^/after big /'", tag: "big") else {
            return say("FAIL images big: never ended")
        }
        await sleep(1)
        let onScreen = footprint()
        // Back to the top of the history, where it starts: its upper half.
        core.scrollLines(core.historyLines)
        await sleep(0.5)
        let top = checkImage(core, name: "big-top", at: [(0.25, 0.25, red), (0.75, 0.25, green)])
        // Back down until only its last rows show, at the top: its lower half.
        core.scrollToBottom()
        await sleep(0.3)
        core.scrollLines(300 + 2 + core.grid.rows / 3)
        await sleep(0.5)
        let bottom = checkImage(core, name: "big-bottom", at: [(0.25, 0.5, blue), (0.75, 0.5, yellow)])
        core.scrollToBottom()
        if !top.ok { failed += 1 }
        if !bottom.ok { failed += 1 }
        say("images big, scrolled to its top: \(top.text)")
        say("images big, scrolled to its end: \(bottom.text)")
        // Gone: the screen and the history cleared.
        _ = await shell(session, core, "printf '\\033[3J'", tag: "gone")
        await sleep(1.5)
        let after = footprint()
        say(String(format: "images big: memory %.1f MB before, %.1f with it on screen and in the history (+%.1f), %.1f once both are cleared",
                   before, onScreen, onScreen - before, after))
        say("RESULT images: " + (failed == 0 ? "PASS" : "FAIL \(failed)"))
    }

    /// Whether an image's memory comes back: the rig's 1200 x 3000 image in
    /// each protocol (img-big-<protocol>.txt) shown, then the screen and the
    /// history cleared, `-MOSHPIT_DIRECTOR_CYCLES` times (10), the app's
    /// footprint after each: flat is a cache filled once, rising a leak.
    private static func imageRepeats() async {
        guard let session, let (surface, _) = shown, let core = surface as? CoreTerminalView else {
            return say("FAIL imagerepeat: not the core's view")
        }
        let dir = arg("-MOSHPIT_DIRECTOR_IMAGES") ?? "."
        let repeats = Int(arg("-MOSHPIT_DIRECTOR_CYCLES") ?? "10") ?? 10
        _ = await shell(session, core, "printf '\\033[3J'", tag: "start")
        await sleep(1.5)
        say(String(format: "imagerepeat: %.1f MB at the start", footprint()))
        for p in ["kitty", "sixel", "iterm2"] {
            var withIt: [Double] = []
            var cleared: [Double] = []
            for i in 1...repeats {
                guard await shell(session, core, "cat \(dir)/img-big-\(p).txt", tag: "\(p)\(i)") else {
                    say("FAIL imagerepeat \(p) \(i): never ended")
                    break
                }
                await sleep(0.8)
                withIt.append(footprint())
                _ = await shell(session, core, "printf '\\033[3J'", tag: "\(p)\(i)gone")
                await sleep(1.5)
                cleared.append(footprint())
            }
            func list(_ v: [Double]) -> String { v.map { String(format: "%.1f", $0) }.joined(separator: " ") }
            say("imagerepeat \(p): shown \(list(withIt)) MB; cleared \(list(cleared)) MB")
        }
        say(String(format: "RESULT imagerepeat: %.1f MB at the end", footprint()))
    }

    /// Images' memory across panes: each of the session's first three tmux
    /// windows shows the rig's 1200 x 3000 image in every protocol, cleared
    /// after each, `-MOSHPIT_DIRECTOR_CYCLES` times (3), the footprint after
    /// each pane; then a memory warning, as the system sends it.
    private static func imagePanes() async {
        guard let session, let control = session.tmuxControl else { return say("FAIL imagepanes needs tmux") }
        let windows = Array(control.snapshot.sortedWindows.prefix(3))
        guard windows.count == 3 else { return say("FAIL imagepanes needs three windows, has \(windows.count)") }
        let dir = arg("-MOSHPIT_DIRECTOR_IMAGES") ?? "."
        let repeats = Int(arg("-MOSHPIT_DIRECTOR_CYCLES") ?? "3") ?? 3
        say(String(format: "imagepanes: %.1f MB at the start; ", footprint()) + CoreTermMemory.summary)
        for (n, window) in windows.enumerated() {
            let before = shown?.0
            control.selectWindow(window.id)
            _ = await wait(3) { shown.map { $0.0 !== before } ?? false }
            await sleep(1)
            guard let (surface, _) = shown, let core = surface as? CoreTerminalView else {
                return say("FAIL imagepanes: pane \(n + 1) is not the core's view")
            }
            for p in ["kitty", "sixel", "iterm2"] {
                for i in 1...repeats {
                    guard await shell(session, core, "cat \(dir)/img-big-\(p).txt", tag: "w\(n)\(p)\(i)") else {
                        say("FAIL imagepanes pane \(n + 1) \(p) \(i): never ended")
                        continue
                    }
                    await sleep(0.8)
                    _ = await shell(session, core, "printf '\\033[3J'", tag: "w\(n)\(p)\(i)gone")
                    await sleep(1)
                }
            }
            say(String(format: "imagepanes pane %d (%@): %.1f MB once its images are cleared; ", n + 1, window.id, footprint())
                + CoreTermMemory.summary)
        }
        await sleep(1.5)
        let before = footprint()
        say("imagepanes before the warning: " + CoreTermMemory.summary)
        NotificationCenter.default.post(name: UIApplication.didReceiveMemoryWarningNotification, object: UIApplication.shared)
        await sleep(2)
        say("imagepanes after the warning: " + CoreTermMemory.summary)
        let after = footprint()
        // Whether what is left comes back by itself: ten seconds on, then a
        // second warning.
        await sleep(10)
        say(String(format: "imagepanes 10 s later: %.1f MB; ", footprint()) + CoreTermMemory.summary)
        NotificationCenter.default.post(name: UIApplication.didReceiveMemoryWarningNotification, object: UIApplication.shared)
        await sleep(3)
        say(String(format: "imagepanes after a second warning: %.1f MB; ", footprint()) + CoreTermMemory.summary)
        // Back to the first pane, its drawables let go: it draws again.
        let before1 = shown?.0
        control.selectWindow(windows[0].id)
        _ = await wait(3) { shown.map { $0.0 !== before1 } ?? false }
        let rendersBefore = (shown?.0 as? CoreTerminalView)?.renders ?? 0
        await sleep(1.5)
        if let back = shown?.0 as? CoreTerminalView {
            say(String(format: "imagepanes back on pane 1: %.1f MB; renders %d → %d, rows %@; ", footprint(), rendersBefore,
                       back.renders, (0..<back.grid.rows).contains { !back.rowText($0).isEmpty } ? "with text" : "empty")
                + CoreTermMemory.summary)
        }
        say(String(format: "RESULT imagepanes: %.1f MB before a memory warning, %.1f after", before, after))
    }

    /// `script` on a cleared screen, then a marker printed (never typed);
    /// whether the marker showed.
    private static func shell(_ session: SessionHub.ActiveSession, _ core: CoreTerminalView, _ script: String,
                              tag: String) async -> Bool {
        let marker = "IMG-\(tag)-END"
        session.sendInput(Data("clear; \(script); printf 'IMG-%s-END\\n' \(tag)\r".utf8))
        return await wait(30) {
            let bottom = min(core.cursor.row, core.grid.rows - 1)
            return (max(0, bottom - 3)...max(0, bottom)).contains { core.rowText($0) == marker }
        }
    }

    /// The picture as the view draws it: the box of its strongly coloured
    /// pixels (the images; text and the prompt are not), the colour at each
    /// `(x, y)` fraction of that box against what it should be. Saves the
    /// picture as tmp/director-<name>.png.
    private static func checkImage(_ core: CoreTerminalView, name: String,
                                   at points: [(Double, Double, (Int, Int, Int))]) -> (ok: Bool, text: String) {
        guard let (px, w, h) = core.renderedPixels() else { return (false, "no pixels") }
        // BGRA.
        func color(_ x: Int, _ y: Int) -> (Int, Int, Int) {
            let i = (y * w + x) * 4
            return (Int(px[i + 2]), Int(px[i + 1]), Int(px[i]))
        }
        var minX = w, minY = h, maxX = -1, maxY = -1
        for y in stride(from: 0, to: h, by: 2) {
            for x in stride(from: 0, to: w, by: 2) {
                let (r, g, b) = color(x, y)
                guard max(r, g, b) > 200, min(r, g, b) < 60 else { continue }
                minX = min(minX, x); maxX = max(maxX, x)
                minY = min(minY, y); maxY = max(maxY, y)
            }
        }
        let path = savePNG(px, w, h, name: name)
        guard maxX >= 0 else { return (false, "no image on screen; \(path)") }
        let bw = maxX - minX + 1, bh = maxY - minY + 1
        var ok = true
        var seen: [String] = []
        for (fx, fy, want) in points {
            let x = minX + Int(Double(bw) * fx), y = minY + Int(Double(bh) * fy)
            let c = color(x, y)
            let good = abs(c.0 - want.0) < 40 && abs(c.1 - want.1) < 40 && abs(c.2 - want.2) < 40
            if !good { ok = false }
            seen.append("(\(c.0),\(c.1),\(c.2))\(good ? "" : "≠(\(want.0),\(want.1),\(want.2))")")
        }
        return (ok, "\(ok ? "PASS" : "FAIL") box \(minX),\(minY) \(bw)x\(bh) px of \(w)x\(h); " + seen.joined(separator: " ") + "; \(path)")
    }

    private static func savePNG(_ px: [UInt8], _ w: Int, _ h: Int, name: String) -> String {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("director-\(name).png")
        var data = px
        let made: CGImage? = data.withUnsafeMutableBytes { raw in
            CGContext(data: raw.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                      space: CGColorSpaceCreateDeviceRGB(),
                      bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)?
                .makeImage()
        }
        guard let image = made, let png = UIImage(cgImage: image).pngData(), (try? png.write(to: url)) != nil else {
            return "not saved"
        }
        return "tmp/" + url.lastPathComponent
    }

    /// The app's memory footprint (MB), as the jetsam limit counts it.
    private static func footprint() -> Double {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return kr == KERN_SUCCESS ? Double(info.phys_footprint) / 1e6 : 0
    }

    // MARK: Folder taps (T10)

    /// B302: the host folder sheet's rows do not answer taps. Opens the
    /// sheet as the Files key does, finds the row of `-MOSHPIT_DIRECTOR_PATH`
    /// by its identifier, and says what a touch at its centre reaches:
    /// UIKit's hit test, and every gesture recognizer on the way from the
    /// hit view to its window (what could take or hold the touch).
    private static func folderTap() async {
        guard let path = arg("-MOSHPIT_DIRECTOR_PATH") else { return say("FAIL foldertap: no -MOSHPIT_DIRECTOR_PATH") }
        present("folder")
        let id = "remote-folder-item-\(path)"
        var row: UIView?
        let found = await wait(30) {
            for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
                for window in scene.windows {
                    if let v = AccentRowMenus.view(identified: id, in: window), v.window != nil { row = v; return true }
                }
            }
            return false
        }
        guard found, let row, let window = row.window else {
            // What there is instead.
            var ids: [String] = []
            func walk(_ v: UIView) {
                if let i = v.accessibilityIdentifier, i.hasPrefix("remote-folder") { ids.append(i) }
                v.subviews.forEach(walk)
            }
            for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
                for w in scene.windows { walk(w) }
            }
            return say("FAIL foldertap: \(id) never showed; there are \(ids.prefix(12))")
        }
        await sleep(1.5)
        let center = row.convert(CGPoint(x: row.bounds.midX, y: row.bounds.midY), to: window)
        say("foldertap row \(type(of: row)) frame \(row.convert(row.bounds, to: window)) in window \(type(of: window)) level \(window.windowLevel.rawValue)")
        let hit = window.hitTest(center, with: nil)
        say("foldertap hit \(hit.map { "\(type(of: $0))" } ?? "nothing") id \(hit?.accessibilityIdentifier ?? "-"); inside the row: \(hit?.isDescendant(of: row) ?? false)")
        if let hit {
            let local = hit.convert(center, from: window)
            let again = hit.hitTest(local, with: nil)
            say("foldertap the hit view's own hitTest: \(again.map { $0 === hit ? "itself" : "\(type(of: $0))" } ?? "nil"); "
                + "pointInside \(hit.point(inside: local, with: nil)); responds to hitTest override: "
                + "\(type(of: hit).instanceMethod(for: #selector(UIView.hitTest(_:with:))) != UIView.instanceMethod(for: #selector(UIView.hitTest(_:with:))))")
            if let parent = hit.superview {
                let p = parent.convert(center, from: window)
                say("foldertap its parent's hitTest: \(parent.hitTest(p, with: nil).map { "\(type(of: $0)) id \($0.accessibilityIdentifier ?? "-")" } ?? "nil")")
            }
        }
        // Every window above, at that point (an overlay window takes it first).
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            for w in scene.windows where w !== window && !w.isHidden && w.windowLevel >= window.windowLevel {
                let p = w.convert(center, from: window)
                say("foldertap other window \(type(of: w)) level \(w.windowLevel.rawValue) hit \(w.hitTest(p, with: nil).map { "\(type(of: $0))" } ?? "nothing")")
            }
        }
        // The views from the hit one up to the list, as NSH made them.
        var up: UIView? = hit
        for depth in 0..<7 {
            guard let v = up else { break }
            let f = v.convert(v.bounds, to: window)
            say("foldertap up \(depth) \(type(of: v)) id \(v.accessibilityIdentifier ?? "-") label \(v.accessibilityLabel ?? "-") tag \(v.tag) "
                + "frame \(Int(f.minX)),\(Int(f.minY)) \(Int(f.width))x\(Int(f.height)) alpha \(v.alpha) hidden \(v.isHidden) "
                + "subviews \(v.subviews.count) bg \(v.backgroundColor.map { "\($0)" } ?? "-")")
            up = v.superview
        }
        // The list's children that cover the row's centre, front to back.
        if let list = AccentRowMenus.view(identified: "remote-folder-list", in: window) {
            func covering(_ v: UIView, _ depth: Int) {
                guard depth < 8 else { return }
                for child in v.subviews.reversed() {
                    let p = child.convert(center, from: window)
                    guard child.bounds.contains(p), !child.isHidden, child.alpha > 0.01 else { continue }
                    let f = child.convert(child.bounds, to: window)
                    say("foldertap covers \(depth) \(type(of: child)) id \(child.accessibilityIdentifier ?? "-") "
                        + "frame \(Int(f.minX)),\(Int(f.minY)) \(Int(f.width))x\(Int(f.height)) interaction \(child.isUserInteractionEnabled) "
                        + "row-inside \(row.isDescendant(of: child))")
                    covering(child, depth + 1)
                }
            }
            covering(list, 0)
        }
        var view: UIView? = hit
        var depth = 0
        while let v = view {
            let recognizers = (v.gestureRecognizers ?? []).map { r in
                "\(type(of: r))\(r.isEnabled ? "" : " off")\(r.cancelsTouchesInView ? " cancels" : "")\(r.delaysTouchesBegan ? " delaysBegan" : "")"
            }
            let interactions = v.interactions.map { "\(type(of: $0))" }
            if !recognizers.isEmpty || !interactions.isEmpty || v === row {
                say("foldertap \(depth) \(type(of: v))\(v === row ? " (the row)" : "")\(v.isUserInteractionEnabled ? "" : " NO-INTERACTION"): "
                    + recognizers.joined(separator: ", ") + (interactions.isEmpty ? "" : "; interactions " + interactions.joined(separator: ", ")))
            }
            view = v.superview
            depth += 1
        }
        say("RESULT foldertap done")
    }

    // MARK: Paths (T3)

    private static func paths() async {
        let real = arg("-MOSHPIT_DIRECTOR_PATH") ?? "out/a.png"
        let fake = "out/nope-\(Int.random(in: 1000...9999)).png"
        guard let session, let (surface, _) = shown else { return say("FAIL paths: no terminal") }
        guard let core = surface as? CoreTerminalView else { return say("FAIL paths: not the core's view") }
        let line = "see \(real) and \(fake)"
        let start = CACurrentMediaTime()
        session.sendInput(Data("printf '%s\\n' '\(line)'\r".utf8))
        var place: (row: Int, col: Int, fakeCol: Int)?
        let linked = await wait(15) {
            for row in 0..<core.grid.rows {
                let text = core.rowText(row)
                guard text == line, let a = text.range(of: real), let b = text.range(of: fake) else { continue }
                // Columns: the row is ASCII here.
                place = (row, text.distance(from: text.startIndex, to: a.lowerBound),
                         text.distance(from: text.startIndex, to: b.lowerBound))
                return core.link(at: (col: place!.col, row: row)) != nil
            }
            return false
        }
        let ms = (CACurrentMediaTime() - start) * 1000
        guard let place, linked else { return say("FAIL paths: \(real) never became a link (printed: \(place != nil))") }
        let link = core.link(at: (col: place.col, row: place.row)) ?? ""
        let last = core.link(at: (col: place.col + real.count - 1, row: place.row)) ?? ""
        let after = core.link(at: (col: place.col + real.count, row: place.row))
        let other = core.link(at: (col: place.fakeCol, row: place.row))
        let path = RemotePathLink.path(from: link) ?? ""
        let ok = path.hasSuffix("/" + real) && last == link && after == nil && other == nil
        say("\(ok ? "PASS" : "FAIL") paths: \(real) a link after \(String(format: "%.0f", ms)) ms, opens \(path),"
            + " last cell \(last == link), the cell after \(after == nil ? "plain" : "linked"),"
            + " the missing file \(other == nil ? "plain" : "linked")")
    }

    // MARK: Rekey

    private static func rekey() async {
        guard let model = TerminalNSHModel.current else { return say("FAIL rekey needs the NSH screen") }
        keyboard(true)
        guard await wait(3, { shown?.0.view.isFirstResponder == true }) else { return say("FAIL no keyboard") }
        await sleep(1.5)
        guard let (surface, coordinator) = shown, let container = coordinator.hostContainer,
              let window = surface.view.window else { return }
        let lines = surface.historyLines
        let probe = KeyboardProbe(surface: surface, container: container, window: window)
        let record = await probe.measure(kind: "rekey") { model.post(["op": "rekey"]) }
        await sleep(1)
        let same = shown?.0 === surface
        let first = surface.view.isFirstResponder
        let after = surface.historyLines
        say("\(same && first && record.kbMax - record.kbMin < 1 && after >= lines ? "PASS" : "FAIL") rekey:"
            + " same view \(same), first responder \(first), keyboard top \(fmt(record.kbMin))…\(fmt(record.kbMax)),"
            + " history \(lines) → \(after) lines, visible jumps \(record.visibleJumps)")
        keyboard(false)
    }

    // MARK: Numbers

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

    private let surface: TerminalSurface
    private let terminal: UIView
    private let container: UIView
    private let window: UIWindow
    private var record = Record(kind: "")
    private var previous: (anchor: CGFloat, text: UInt64)?
    private var lastFrame: CGRect = .null
    private var lastGrid = ""
    private var lastChange: CFTimeInterval = 0
    private var samples: [(t: CFTimeInterval, edge: CGFloat, kb: CGFloat?)] = []

    init(surface: TerminalSurface, container: UIView, window: UIWindow) {
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
