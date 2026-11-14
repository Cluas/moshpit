import SwiftTerm
import Testing
import UIKit
@testable import Moshpit

/// The terminal screen in NSH (`MOSHPIT_NSH_TERMINAL`): its switch, what the
/// host posts it (every state a message the Rust screen takes), what it
/// tells the host (every intent read back), and the stage that shows the
/// session's terminals in its slot — a pane switch, a reconnect's kept frame,
/// the handover that keeps a terminal in the window across a rebuild.
@MainActor
@Suite("Terminal in NSH")
struct TerminalNSHTests {
    // MARK: The screen

    @Test("the terminal runs the Rust screen of its name")
    func name() {
        #expect(NSHScreen.terminal.name == "terminal")
    }

    // MARK: What the host posts

    static let plan = BreadcrumbPlan(
        sessionTitle: "moshpit", sessionSqueezable: true, windowTitle: "2:agents",
        paneTitle: "claude", paneSignal: .attention)

    static func chrome() -> TerminalNSHChrome {
        var chrome = TerminalNSHChrome()
        chrome.name = "studio"
        chrome.user = "cluas"
        chrome.host = "studio.local"
        chrome.port = 2222
        chrome.mux = .tmux
        chrome.pill = .mosh
        chrome.state = .live
        chrome.crumbs = .init(plan: plan, sessionIcon: "square.stack.3d.up", windowIcon: "macwindow", live: true)
        chrome.content = .terminal
        return chrome
    }

    static func states() -> [TerminalNSHChrome] {
        var states = [chrome()]
        var poster = chrome()
        poster.crumbs = nil
        poster.state = .connecting
        poster.content = .none
        poster.cover = .init(kind: .poster, state: .connecting, touches: true)
        states.append(poster)
        var veil = chrome()
        veil.state = .reconnecting
        veil.crumbs?.live = false
        veil.cover = .init(kind: .veil, state: .reconnecting, ended: false)
        states.append(veil)
        var follow = chrome()
        follow.cover = .init(kind: .follow, state: .live, owner: "iPad", touches: true)
        states.append(follow)
        var empty = chrome()
        empty.content = .empty
        empty.empty = .init(mux: .herdr, missing: true, quiet: false)
        states.append(empty)
        // The host knows the answer: the screen shows it at once.
        var known = empty
        known.empty = .init(mux: .tmux, missing: false, quiet: true, settled: true)
        states.append(known)
        for banner: TerminalNSHChrome.Banner in [
            .returnPath, .roam(srtt: 182.5, history: [100, 182.5]), .roam(srtt: nil, history: []),
            .degrade(.tmux), .degrade(.herdr), .degrade(.moshServer),
        ] {
            var state = chrome()
            state.banner = banner
            states.append(state)
        }
        var notice = chrome()
        notice.notice = .init(text: "This pane is open on another device", action: "Open here")
        notice.link = "github.com"
        notice.sidebar = true
        notice.compact = true
        notice.pill = .moshRoaming
        notice.state = .offline
        states.append(notice)
        return states
    }

    @Test("every chrome the host builds is a message the Rust screen takes")
    func chromeCrossesTheBoundary() {
        for state in Self.states() {
            #expect(NSHSurface.check(screen: NSHScreen.terminal.name, message: state.message),
                    "\(state.message)")
        }
    }

    @Test("the breadcrumb says what VoiceOver reads for the pane, and only a live plan is pressable")
    func crumbsCarryTheirLabels() {
        let json = TerminalNSHChrome.json(Self.chrome().crumbs!)
        #expect(json["paneLabel"] as? String == "claude, \(AgentSignal.attention.label)")
        #expect(json["signal"] as? String == "attention")
        #expect(json["live"] as? Bool == true)
    }

    @Test("the shortcut bar's state is a message the screen takes, every kind of chip")
    func barCrossesTheBoundary() {
        let kinds: [ShortcutKind] = [.keyCombo, .text, .command, .dpad, .arrows, .paste,
                                     .scroll, .ctrl, .mic, .image, .files]
        let chips = kinds.map { kind in
            TerminalNSHBar.chip(TerminalShortcut(kind: kind, chipLabel: "x"))
        }
        let bar = TerminalNSHBar(chips: chips, keyboardDown: false, ctrlArmed: true, voice: true,
                                 micActive: true, clipboardHasImage: true, camera: true,
                                 sent: [.init(id: UUID().uuidString, number: 1, filename: "a.png")])
        #expect(NSHSurface.check(screen: NSHScreen.terminal.name, message: bar.message))
        #expect(Set(chips.map(\.kind)) == ["key", "text", "dpad", "arrows", "paste", "scroll", "ctrl",
                                           "mic", "image", "files"])
    }

    @Test("the panels' messages are the screen's, and none takes a panel away")
    func panelsCrossTheBoundary() {
        let screen = NSHScreen.terminal.name
        #expect(NSHSurface.check(screen: screen, message: TerminalNSHPanel.message("dictation", nil)))
        #expect(NSHSurface.check(screen: screen, message: TerminalNSHPanel.message("attachment", nil)))
        #expect(NSHSurface.check(screen: screen, message: TerminalNSHPanel.message("preview", nil)))
        #expect(NSHSurface.check(screen: screen, message: TerminalNSHPanel.level(0.4)))
        let controller = VoiceDictationController()
        #expect(NSHSurface.check(screen: screen,
                                 message: TerminalNSHPanel.message("dictation", .dictation(controller))))
        let upload = ImageAttachmentController(uploaderProvider: { throw SSHError.sessionClosed })
        #expect(NSHSurface.check(screen: screen,
                                 message: TerminalNSHPanel.message("attachment", .attachment(upload))))
    }

    @Test("bad messages are refused")
    func badMessages() {
        let screen = NSHScreen.terminal.name
        #expect(!NSHSurface.check(screen: screen, message: ["op": "chrome", "colour": 1]))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "focus"]))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "level", "level": "loud"]))
        #expect(NSHSurface.check(screen: screen, message: ["op": "rekey"]))
        // The scene's key commands, handed to the page (W11h).
        #expect(NSHSurface.check(screen: screen, message: ["op": "key", "stroke": "mod+k"]))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "key"]))
    }

    // MARK: What the screen tells the host

    @Test("every intent the screen tells is read back; anything else is nothing")
    func intents() {
        let cases: [([String: Any], TerminalNSHIntent)] = [
            (["ev": "back"], .back),
            (["ev": "sidebar"], .sidebar),
            (["ev": "switch-transport"], .transport),
            (["ev": "diagnostics"], .diagnostics),
            (["ev": "diagnostics-dismiss"], .diagnosticsDismiss),
            (["ev": "repaint"], .repaint),
            (["ev": "crumb", "crumb": "window"], .crumb(.window)),
            (["ev": "window", "number": 3], .window(3)),
            (["ev": "take-over"], .takeOver),
            (["ev": "create"], .create),
            (["ev": "install"], .install),
            (["ev": "banner-action"], .bannerAction),
            (["ev": "banner-dismiss"], .bannerDismiss),
            (["ev": "notice-action"], .noticeAction),
            (["ev": "open-link"], .openLink),
            (["ev": "chip", "id": "abc"], .chip("abc")),
            (["ev": "arrow", "dir": "left"], .arrow("left")),
            (["ev": "scroll", "dir": "up"], .scroll("up")),
            (["ev": "keyboard"], .keyboard),
            (["ev": "mic"], .mic),
            (["ev": "image"], .image),
            (["ev": "image-paste"], .imagePaste),
            (["ev": "image-camera"], .imageCamera),
            (["ev": "image-sent", "id": "x"], .imageSent("x")),
            (["ev": "dictation", "action": "send"], .dictation(.send)),
            (["ev": "attachment", "action": "retry"], .attachment(.retry)),
            (["ev": "preview", "action": "confirm"], .preview(.confirm)),
        ]
        for (message, intent) in cases {
            #expect(TerminalNSHIntent(message) == intent, "\(message)")
        }
        // The pill's tap alone never switches: the page asks first.
        for bad: [String: Any] in [["ev": "crumb"], ["ev": "arrow", "dir": "sideways"], ["ev": "fly"], [:],
                                   ["ev": "transport"], ["ev": "window", "number": 0], ["ev": "window"]] {
            #expect(TerminalNSHIntent(bad) == nil, "\(bad)")
        }
    }

    @Test("the model keeps the handover and the anchors, and hands on the rest")
    func modelHandlesItsOwn() {
        let model = TerminalNSHModel()
        #expect(model.handle(["ev": "anchor", "name": "breadcrumb", "rect": [100, 30, 220, 32]]) == nil)
        #expect(model.anchors.breadcrumb == CGRect(x: 100, y: 30, width: 220, height: 32))
        #expect(model.handle(["ev": "anchor", "name": "pill", "rect": [60, 34, 50, 27.5]]) == nil)
        #expect(model.anchors.pill?.height == 27.5)
        #expect(model.handle(["ev": "handover"]) == nil)
        #expect(model.handle(["ev": "back"]) == .back)
    }

    // MARK: The stage

    /// A slot in a window, as NSH lays it out.
    static func slot(for stage: TerminalNSHStage) -> (TerminalNSHSlotView, UIWindow) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        window.frame = CGRect(x: 0, y: 0, width: 744, height: 1133)
        window.isHidden = false
        let slot = TerminalNSHSlotView(frame: CGRect(x: 0, y: 100, width: 744, height: 800))
        stage.register(slot)
        window.addSubview(slot)
        return (slot, window)
    }

    @Test("a pane shows in the slot, inset, and a switch swaps the session's own terminals")
    func paneSwitch() async throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = Self.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let first = controller.surface(for: "%1").view
        #expect(stage.terminal === first)
        let container = try #require(stage.container)
        #expect(container.superview === slot)
        #expect(container.frame == slot.bounds.insetBy(dx: 4, dy: 4))
        #expect(controller.coordinator(for: "%1")?.hostContainer === container)

        stage.show(.init(source: .pane(controller, "%2"), inset: 4, slide: true))
        let second = controller.surface(for: "%2").view
        #expect(stage.terminal === second)
        #expect(second !== first, "each pane keeps its own terminal")
        // The old pane slides away and leaves the hierarchy; its terminal is
        // the controller's still.
        try await Self.settle { first.window == nil }
        #expect(controller.surface(for: "%1").view === first)

        // Back to the first: the very same view, not a new one.
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        #expect(stage.terminal === first)
        #expect(first.superview?.superview === slot)
    }

    @Test("a reconnect's kept frame is shown untouchable, then fades to the new attach's pane")
    func keptFrame() async throws {
        let retired = TmuxSessionController(sshSession: MockTmuxTransport())
        let fresh = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (_, window) = Self.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(retired, "%3"), inset: 4, frozen: true))
        let ghost = try #require(stage.container)
        #expect(!ghost.isUserInteractionEnabled, "a frozen frame takes no touch")
        stage.show(.init(source: .pane(fresh, "%3"), inset: 4))
        #expect(stage.terminal === fresh.surface(for: "%3").view, "the same pane id on a new controller is a new terminal")
        #expect(stage.container?.isUserInteractionEnabled == true)
        try await Self.settle { ghost.superview == nil }
    }

    @Test("a sheet's hold reaches the container, and a rekey's handover keeps the terminal in the window")
    func holdAndHandover() async throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = Self.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        stage.apply(focus: .allow, geometryHeld: true)
        #expect(stage.container?.geometryHeld == true)
        stage.apply(focus: .allow, geometryHeld: false)
        #expect(stage.container?.geometryHeld == false)

        // The screen announces a new element: the container waits in the
        // window, the new slot adopts it.
        let terminal = try #require(stage.terminal)
        stage.handover()
        #expect(terminal.window === window, "parked, never out of the window")
        #expect(stage.container?.superview === window)
        let next = TerminalNSHSlotView(frame: slot.frame)
        stage.register(next)
        window.addSubview(next)
        stage.slotDestroyed(slot)
        slot.removeFromSuperview()
        #expect(stage.container?.superview === next)
        #expect(terminal.window === window)
    }

    /// Waits up to two seconds for `condition`.
    static func settle(_ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(2)
        while !condition(), Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(condition())
    }

    // MARK: R12's pictures

    /// The plan R12's pictures show: `train › trainer › claude`, the window
    /// named by tmux after Claude Code's version, the pane in `~/src/trainer`.
    static func r12Chrome() -> TerminalNSHChrome {
        var tmux = TmuxSnapshot()
        tmux.sessions = ["$0": SessionInfo(id: "$0", name: "train", isAttached: true)]
        tmux.windows = ["@1": WindowInfo(id: "@1", sessionId: "$0", name: "2.1.322", index: 1)]
        var pane = PaneInfo(id: "%3", windowId: "@1", index: 0, command: "2.1.322")
        pane.path = "/home/dev/src/trainer"
        tmux.panes = ["%3": pane]
        tmux.activeSessionId = "$0"
        tmux.activeWindowId = "@1"
        tmux.activePaneId = "%3"
        tmux.isAttached = true
        let plan = BreadcrumbPlan.make(
            snapshot: tmux, hooks: ["%3": AgentHook(state: "attention", agent: "claude")])!
        var chrome = Self.chrome()
        chrome.name = "gpu-01"
        chrome.host = "gpu-01.example.com"
        chrome.crumbs = .init(plan: plan, sessionIcon: "square.stack.3d.up", windowIcon: "macwindow", live: true)
        return chrome
    }

    /// The states R12 is checked by, by name.
    static func r12States() -> [(String, TerminalNSHChrome)] {
        let live = r12Chrome()
        var poster = live
        poster.crumbs = nil
        poster.state = .connecting
        poster.content = .none
        poster.cover = .init(kind: .poster, state: .connecting, touches: true)
        var veil = live
        veil.state = .reconnecting
        veil.crumbs?.live = false
        veil.cover = .init(kind: .veil, state: .reconnecting, ended: false)
        var ended = veil
        ended.cover = .init(kind: .veil, state: .offline, ended: true)
        var follow = live
        follow.cover = .init(kind: .follow, state: .live, owner: "iPad")
        var banner = live
        banner.pill = .moshRoaming
        banner.banner = .roam(srtt: 182.5, history: [100, 182.5])
        banner.notice = .init(text: "This pane is open on another device", action: "Open here")
        banner.link = "example.com"
        var empty = live
        empty.crumbs = nil
        empty.content = .empty
        empty.empty = .init(mux: .tmux, missing: false, quiet: false)
        return [("live", live), ("poster", poster), ("veil", veil), ("ended", ended),
                ("follow", follow), ("banner", banner), ("empty", empty)]
    }

    @Test("R12: the chrome in each state, on a dark and a light terminal theme, drawn to pictures")
    func r12Pictures() async throws {
        let bar = TerminalNSHBar(
            chips: [.keyCombo, .text, .ctrl, .arrows, .mic, .image].map {
                TerminalNSHBar.chip(TerminalShortcut(kind: $0, chipLabel: $0 == .ctrl ? "ctrl" : "esc"))
            },
            keyboardDown: true, ctrlArmed: true, voice: true, micActive: false,
            clipboardHasImage: false, camera: true, sent: [])
        for (look, background) in [("dark", "#0D1117"), ("light", "#FDF6E3")] {
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            let window = scene.map { UIWindow(windowScene: $0) }
                ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
            // The phone's size, no taller than the window: the iPad stands in
            // landscape (744 pt tall), and what lies outside the window is
            // never drawn.
            let frame = CGRect(x: 0, y: 0, width: NavBarNSHTests.phone.width,
                               height: min(NavBarNSHTests.phone.height, window.bounds.height))
            let surface = try #require(NSHSurface.make(frame: frame))
            window.addSubview(surface)
            window.isHidden = false
            defer {
                surface.removeFromSuperview()
                window.isHidden = true
            }
            var params = NSHScreenParams.current()
            params["terminal"] = background
            #expect(surface.run(screen: NSHScreen.terminal.name, params: params), "\(look)")
            #expect(surface.post(bar.message))
            for (name, state) in Self.r12States() {
                #expect(surface.post(state.message), "\(look) \(name)")
                // The empty state shows the wait for its first four seconds
                // (the attach handshake's), then what it found.
                try? await Task.sleep(for: .milliseconds(name == "empty" ? 4_600 : 700))
                Self.shoot(surface, "\(name)-\(look)")
            }
            // The voice panel over the bar.
            let voice: [String: Any] = [
                "op": "dictation",
                "panel": ["phase": ["phase": "listening"], "engine": "Whisper · 中文",
                          "finalized": "run the trainer tests", "volatile": " again",
                          "insertable": true] as [String: Any],
            ]
            #expect(surface.post(Self.r12States()[0].1.message))
            #expect(surface.post(voice), "\(look) voice")
            try? await Task.sleep(for: .milliseconds(700))
            Self.shoot(surface, "voice-\(look)")
        }
    }

    @Test("R18: on a light app the terminal keeps its colour scheme's appearance, drawn")
    func r18TerminalKeepsItsScheme() async throws {
        for (look, background, scheme) in [("dark-scheme", "#0D1117", UIUserInterfaceStyle.dark),
                                           ("light-scheme", "#FDF6E3", .light)] {
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
            window.frame = NavBarNSHTests.phone
            // The app in the light appearance, where the app puts it (R18).
            window.overrideUserInterfaceStyle = .light
            let surface = try #require(NSHSurface.make(frame: NavBarNSHTests.phone))
            window.addSubview(surface)
            window.isHidden = false
            defer {
                surface.removeFromSuperview()
                window.isHidden = true
            }
            var params = NSHScreenParams.current()
            params["terminal"] = background
            params["appearance"] = "system"
            #expect(surface.run(screen: NSHScreen.terminal.name, params: params), "\(look)")
            #expect(surface.post(Self.r12States()[0].1.message), "\(look)")
            let bar = try #require(await SettingsNSHTests.view(identified: "terminal-top-bar", in: surface),
                                   "\(look): the bar")
            try? await Task.sleep(for: .milliseconds(700))
            // Its scheme's, not the window's: a dark scheme stays dark on a
            // light app, and Home's appearance is the window's alone.
            #expect(bar.traitCollection.userInterfaceStyle == scheme, "\(look)")
            #expect(window.overrideUserInterfaceStyle == .light, "\(look): the window is the app's")
            Self.shoot(surface, "r18-light-app-\(look)")
        }
    }

    /// Draws the window where `surface` is to the app's tmp folder as
    /// `terminal-<name>.png`.
    static func shoot(_ surface: UIView, _ name: String) {
        guard let window = surface.window else { return }
        let frame = surface.convert(surface.bounds, to: window)
        let image = UIGraphicsImageRenderer(bounds: frame).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("terminal-\(name).png")
        try? image.pngData()?.write(to: url)
    }

    // MARK: On a real surface

    @Test("on a real surface: the chrome draws, its slot is the stage's, and a press reaches the host")
    func realSurface() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .terminal)
        defer { done() }
        #expect(surface.post(Self.chrome().message))
        let back = try #require(await SettingsNSHTests.view(identified: "terminal-back", in: surface))
        #expect(back.accessibilityActivate())
        // Skip the anchors the bar reports as it lays out.
        var message = await heard.next()
        while message?["ev"] as? String == "anchor" { message = await heard.next() }
        #expect(message?["ev"] as? String == "back")
        #expect(await SettingsNSHTests.view(identified: "terminal-breadcrumb", in: surface) != nil)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(await SettingsNSHTests.view(identified: "keyboard-toggle", in: surface) != nil)
    }
}
