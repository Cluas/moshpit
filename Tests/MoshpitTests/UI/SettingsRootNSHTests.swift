import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// Settings itself in NSH: its switch, the data path between the host and the
/// screen — the state the Swift side posts is a message the Rust screen takes
/// (`ui/src/settings.rs`'s tests and `ui/tests/settings.rs` cover the screen),
/// what the screen tells the host is applied to `AppSettings` as the SwiftUI
/// page's controls apply it or opens what the row opens — and a real surface
/// on the device that runs the screen, shows UIKit's controls with the
/// settings' values and reports what the user does.
@Suite("NSH Settings root")
@MainActor
struct SettingsRootNSHTests {

    /// Settings and stores of their own, so a test writes nothing the app
    /// keeps.
    static func scratch() -> (AppSettings, ShortcutStore, SSHKeyStore, ThemeStore) {
        let defaults = UserDefaults(suiteName: "w11c.nsh.\(UUID().uuidString)")!
        return (AppSettings(defaults: defaults), ShortcutStore(defaults: defaults),
                SSHKeyStore(defaults: defaults), ThemeStore(defaults: defaults))
    }

    static func snapshot(_ settings: AppSettings, liveSession: Bool = false, filing: Bool = false,
                                 shownIcon: String? = nil) -> SettingsSnapshot {
        let (_, shortcuts, keys, themes) = scratch()
        return SettingsSnapshot(settings: settings, shortcuts: shortcuts, keys: keys, themes: themes,
                                liveSession: liveSession, showsFilingNumber: filing,
                                shownIcon: { shownIcon })
    }

    @Test("the page runs the Rust screen of its name")
    func name() {
        #expect(NSHScreen.settings.name == "settings")
    }

    @Test("every state is a message the Rust screen takes; bad ones are refused")
    func messages() {
        let (settings, _, _, _) = Self.scratch()
        let screen = NSHScreen.settings.name
        #expect(NSHSurface.check(screen: screen, message: SettingsModel.message(Self.snapshot(settings))))
        settings.voiceEngine = .whisper
        settings.cursorShape = .underline
        settings.predictMode = .experimental
        settings.uploadRetentionDays = 0
        let full = SettingsModel.message(Self.snapshot(settings, liveSession: true, filing: true))
        #expect(NSHSurface.check(screen: screen, message: full), "\(full)")
        #expect(full["filing"] as? String == SettingsScreen.icpFilingNumber)
        #expect((full["fonts"] as? [[String: Any]])?.count == TerminalFont.families.count)
        #expect((full["theme"] as? [String: Any]).flatMap { $0["swatches"] as? [String] }?.count == 8)
        var bad = full
        bad["cursorShape"] = "beam"
        #expect(!NSHSurface.check(screen: screen, message: bad))
        bad = full
        bad.removeValue(forKey: "keyboardOnOpen")
        #expect(!NSHSurface.check(screen: screen, message: bad))
        for anchor in SettingsAnchor.allCases {
            #expect(NSHSurface.check(screen: screen, message: ["op": "reveal", "anchor": anchor.rawValue]))
        }
        #expect(!NSHSurface.check(screen: screen, message: ["op": "reveal", "anchor": "top"]))
    }

    @Test("the row names the icon iOS shows, as the NSH App Icon page ticks it")
    func iconName() {
        let (settings, _, _, _) = Self.scratch()
        settings.appIconId = "default"
        #expect(Self.snapshot(settings, shownIcon: "AppIcon-Pit").icon.id == "pit")
        #expect(Self.snapshot(settings, shownIcon: nil).icon.id == "default")
        settings.appIconId = "wq"
        #expect(Self.snapshot(settings, shownIcon: "AppIcon-Gone").icon.id == "wq")
    }

    @Test("a setting the screen sends is applied as the SwiftUI controls apply it")
    func apply() {
        let (settings, _, _, _) = Self.scratch()
        for (key, path) in SettingsModel.flags {
            let before = settings[keyPath: path]
            #expect(SettingsModel.apply(key, !before, to: settings), "\(key)")
            #expect(settings[keyPath: path] == !before, "\(key)")
        }
        #expect(SettingsModel.apply("font", "Menlo", to: settings))
        #expect(settings.fontName == "Menlo")
        #expect(!SettingsModel.apply("font", "Comic Sans", to: settings))
        #expect(settings.fontName == "Menlo")
        #expect(SettingsModel.apply("fontSize", 12.0, to: settings))
        #expect(settings.fontSize == 12)
        #expect(SettingsModel.apply("fontSize", 24, to: settings))
        #expect(settings.fontSize == 24, "a pinch's size, kept (B515)")
        #expect(SettingsModel.apply("fontSize", 40, to: settings))
        #expect(settings.fontSize == 32, "clamped to the pinch's range")
        #expect(SettingsModel.apply("cursorShape", "bar", to: settings))
        #expect(settings.cursorShape == .bar)
        #expect(!SettingsModel.apply("cursorShape", "beam", to: settings))
        #expect(SettingsModel.apply("cursorColor", "white", to: settings))
        #expect(settings.cursorColorId == "white")
        #expect(!SettingsModel.apply("cursorColor", "red", to: settings))
        #expect(SettingsModel.apply("uploadRetentionDays", 0, to: settings))
        #expect(settings.uploadRetentionDays == 0)
        #expect(!SettingsModel.apply("uploadRetentionDays", 3, to: settings))
        #expect(SettingsModel.apply("predictMode", "never", to: settings))
        #expect(settings.predictMode == .never)
        #expect(!SettingsModel.apply("keyboardOnOpen", "yes", to: settings))
        // Gone with its switch (B516): nothing read it.
        #expect(!SettingsModel.apply("keepAlive", false, to: settings))
        #expect(!SettingsModel.apply("colour", true, to: settings))
        // The UDP range's page (R18): a range the rule allows, or nothing.
        #expect(SettingsModel.apply("udpRange", [50000, 50010], to: settings))
        #expect(settings.udpRangeStart == 50000 && settings.udpRangeEnd == 50010)
        let refused: [Any] = [[61000, 60000], [0, 10], [10, 65536], [1], "60000-61000"]
        for value in refused {
            #expect(!SettingsModel.apply("udpRange", value, to: settings), "\(value)")
        }
        #expect(settings.udpRangeStart == 50000 && settings.udpRangeEnd == 50010)
    }

    @Test("open, done and back reach the page's owner")
    func intents() {
        let (settings, _, _, _) = Self.scratch()
        let model = SettingsModel()
        var opened: [String] = []
        var done = 0
        var back = 0
        let handle = { (message: [String: Any]) in
            model.handle(message, settings: settings, onOpen: { opened.append($0) },
                         onDone: { done += 1 }, onBack: { back += 1 })
        }
        handle(["ev": "open", "page": "whisper-model"])
        handle(["ev": "open", "page": "terminal"])
        handle(["ev": "done"])
        handle(["ev": "back"])
        handle(["ev": "set", "key": "voiceInput", "value": false])
        #expect(opened == ["whisper-model", "terminal"])
        #expect(done == 1)
        #expect(back == 1)
        #expect(settings.voiceInputEnabled == false)
        // The pages pushed on Settings' stack are the rows' pages; the hub's
        // own pages are the NSH screen again (`ui/src/settings.rs`'s Part).
        #expect(SettingsPage(rawValue: "voice-engine") == .voiceEngine)
        #expect(SettingsPage(rawValue: "shortcuts") == nil, "a sheet, not a page")
        #expect(SettingsPart.allCases.map(\.rawValue)
            == ["appearance", "terminal", "connections", "notifications", "voice", "companion",
                "hosts", "udp-range", "notification-info"])
        #expect(SettingsPart(rawValue: "terminal") != nil && SettingsPage(rawValue: "terminal") == nil)
    }

    // MARK: On the device

    @Test("on a real surface: the hub's rows open its pages, and its version row has Copy")
    func surface() async throws {
        let (settings, _, _, _) = Self.scratch()
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .settings)
        defer { done() }
        #expect(surface.post(SettingsModel.message(Self.snapshot(settings))))
        let terminal = try #require(await SettingsNSHTests.view(identified: "settings-open-terminal", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(terminal.accessibilityActivate())
        let open = try #require(await heard.next())
        #expect(open["ev"] as? String == "open")
        #expect(open["page"] as? String == "terminal")

        // The version row's Copy menu: UIKit's, for that row only.
        let version = try #require(await SettingsNSHTests.view(identified: SettingsModel.versionID, in: surface))
        let feedback = try #require(await SettingsNSHTests.view(identified: "settings-feedback", in: surface))
        let menus = NSHContextMenus()
        menus.menus = [SettingsModel.versionID: { [UIAction(title: "Copy") { _ in }] }]
        menus.install(on: surface)
        let interaction = try #require(surface.interactions.compactMap { $0 as? UIContextMenuInteraction }.first)
        let onVersion = surface.convert(CGPoint(x: version.bounds.midX, y: version.bounds.midY), from: version)
        #expect(menus.contextMenuInteraction(interaction, configurationForMenuAtLocation: onVersion) != nil)
        let onFeedback = surface.convert(CGPoint(x: feedback.bounds.midX, y: feedback.bounds.midY), from: feedback)
        #expect(menus.contextMenuInteraction(interaction, configurationForMenuAtLocation: onFeedback) == nil)
    }

    @Test("on a real surface: Terminal's controls are UIKit's with the settings, and its steps set the size")
    func terminalSurface() async throws {
        let (settings, _, _, _) = Self.scratch()
        settings.cursorShape = .bar
        settings.fontName = "Menlo"
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .settings, page: "terminal")
        defer { done() }
        #expect(surface.post(SettingsModel.message(Self.snapshot(settings))))
        let shape = try #require(await SettingsNSHTests.view(identified: "settings-cursor-shape", in: surface)
            as? UISegmentedControl)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(shape.numberOfSegments == 3)
        #expect(shape.selectedSegmentIndex == 1)
        let font = try #require(await SettingsNSHTests.view(identified: "settings-font", in: surface) as? UIButton)
        #expect(font.showsMenuAsPrimaryAction)
        #expect(font.menu?.children.map(\.title) == TerminalFont.families.map(\.label))
        #expect(font.accessibilityLabel?.hasSuffix("Menlo") == true)
        let larger = try #require(await SettingsNSHTests.view(identified: "settings-font-larger", in: surface))
        #expect(larger.accessibilityActivate())
        let set = try #require(await heard.next())
        #expect(set["ev"] as? String == "set")
        #expect(set["key"] as? String == "fontSize")
        #expect((set["value"] as? Double) == min(max(settings.fontSize.rounded() + 1, 8), 32))
    }

    @Test("on a real surface: Connections' switches carry the settings, and a flip reaches the host")
    func connectionsSurface() async throws {
        let (settings, _, _, _) = Self.scratch()
        settings.raiseKeyboardOnOpen = true
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .settings, page: "connections")
        defer { done() }
        #expect(surface.post(SettingsModel.message(Self.snapshot(settings))))
        let keyboard = try #require(await SettingsNSHTests.view(identified: "settings-keyboard-on-open", in: surface)
            as? UISwitch)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(keyboard.isOn)
        keyboard.setOn(false, animated: false)
        keyboard.sendActions(for: .valueChanged)
        let set = try #require(await heard.next())
        #expect(set["ev"] as? String == "set")
        #expect(set["key"] as? String == "keyboardOnOpen")
        #expect(set["value"] as? Bool == false)
    }
}

/// R18 part B: the appearance chosen in Settings reaches the phone through
/// the window (``WindowAppearance``), the pages follow the window's trait
/// as it changes, and the inks the SwiftUI cards and the segmented control
/// draw with take each appearance's side.
@Suite("R18: the app's appearance on its windows")
@MainActor
struct AppAppearanceWindowTests {

    @Test("the chosen appearance is the window's while the NSH Home shows, dark under the SwiftUI one")
    func windowStyle() {
        #expect(AppAppearance.windowStyle(chosen: .system, nshHome: true) == .unspecified)
        #expect(AppAppearance.windowStyle(chosen: .light, nshHome: true) == .light)
        #expect(AppAppearance.windowStyle(chosen: .dark, nshHome: true) == .dark)
        for chosen in AppAppearance.allCases {
            #expect(AppAppearance.windowStyle(chosen: chosen, nshHome: false) == .dark, "\(chosen)")
        }
    }

    @Test("the probe sets its window's style once in the window, and again as the choice changes")
    func probe() throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
        let probe = WindowAppearance.Probe()
        probe.style = .light
        #expect(window.overrideUserInterfaceStyle == .unspecified, "nothing before it is in the window")
        window.addSubview(probe)
        defer { probe.removeFromSuperview() }
        #expect(window.overrideUserInterfaceStyle == .light)
        probe.style = .unspecified
        #expect(window.overrideUserInterfaceStyle == .unspecified, "the system's")
        probe.style = .dark
        #expect(window.overrideUserInterfaceStyle == .dark)
    }

    @Test("a page following the system turns light as its window does, without running again")
    func pageFollowsTheWindow() async throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
        window.frame = NavBarNSHTests.phone
        window.overrideUserInterfaceStyle = .dark
        let surface = try #require(NSHSurface.make(frame: NavBarNSHTests.phone))
        window.addSubview(surface)
        window.isHidden = false
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        var params = NSHScreenParams.current()
        params["appearance"] = "system"
        #expect(surface.run(screen: NSHScreen.settings.name, params: params))
        let (settings, _, _, _) = SettingsRootNSHTests.scratch()
        #expect(surface.post(SettingsModel.message(SettingsRootNSHTests.snapshot(settings))))
        let row = try #require(await SettingsNSHTests.view(identified: "settings-open-appearance", in: surface))
        #expect(row.traitCollection.userInterfaceStyle == .dark)
        window.overrideUserInterfaceStyle = .light
        try? await Task.sleep(for: .milliseconds(300))
        #expect(row.traitCollection.userInterfaceStyle == .light, "the same page, now light")
    }

    @Test("the SwiftUI pages' accent as words is the NSH pages' primary-text: Signal Room's 4E4DEC")
    func accentText() {
        let accent = UIColor(red: 0x6C / 255, green: 0x6B / 255, blue: 0xEF / 255, alpha: 1)
        #expect(Self.bytes(InkContrast.readable(accent)) == [0x4E, 0x4D, 0xEC, 255])
        // Already readable: kept.
        let deep = UIColor(red: 0x1F / 255, green: 0x3A / 255, blue: 0x93 / 255, alpha: 1)
        #expect(Self.bytes(InkContrast.readable(deep)) == Self.bytes(deep))
        // Its dark side is the accent itself, as before.
        let dark = UITraitCollection(userInterfaceStyle: .dark)
        #expect(Self.bytes(UIColor(Ink.accentText).resolvedColor(with: dark))
                == Self.bytes(UIColor(Ink.accent).resolvedColor(with: dark)))
    }

    @Test("the form inks the SwiftUI sheets draw with have a light side, and the dark one as before")
    func formInks() {
        let light = UITraitCollection(userInterfaceStyle: .light)
        let dark = UITraitCollection(userInterfaceStyle: .dark)
        let side = { (color: Color, traits: UITraitCollection) in Self.bytes(UIColor(color).resolvedColor(with: traits)) }
        #expect(side(Ink.group, light) == [0xFF, 0xFF, 0xFF, 255])
        #expect(side(Ink.group, dark) == Self.bytes(UIColor(Color(hex: "11141A").opacity(0.94))))
        #expect(side(Ink.pageBG, light) == [0xF3, 0xF2, 0xEE, 255])
        #expect(side(Ink.sectionTitle, light) == [0x5E, 0x5C, 0x56, 255])
        #expect(side(Ink.warnText, light) == [0x93, 0x52, 0x00, 255])
        #expect(side(Ink.warnText, dark) == side(Ink.warn, dark))
        // Words on the light page read (4.5:1).
        let page = (Double(0xF3) / 255, Double(0xF2) / 255, Double(0xEE) / 255)
        for (name, ink) in [("warn", Ink.warnText), ("section", Ink.sectionTitle)] {
            let rgb = side(ink, light).prefix(3).map { Double($0) / 255 }
            #expect(InkContrast.contrast((rgb[0], rgb[1], rgb[2]), page) >= 4.5, "\(name)")
        }
    }

    /// Draws `view` alone in a phone-sized window of `style` to the app's tmp
    /// folder as `<name>.png`.
    private static func shoot<V: View>(_ view: V, _ style: UIUserInterfaceStyle, _ name: String) async {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
        window.frame = NavBarNSHTests.phone
        window.overrideUserInterfaceStyle = style
        window.rootViewController = UIHostingController(rootView: view.environment(AppSettings.shared))
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        try? await Task.sleep(for: .milliseconds(700))
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        try? image.pngData()?.write(to: FileManager.default.temporaryDirectory.appendingPathComponent("\(name).png"))
    }

    @Test("the SwiftUI sheets that follow the app draw dark and light (R18 E)")
    func swiftUISheetShots() async {
        for (look, style) in [("dark", UIUserInterfaceStyle.dark), ("light", .light)] {
            await Self.shoot(OfflineHostInfoView(), style, "r18e-offline-host-\(look)")
            await Self.shoot(NewAgentTaskSheet(
                initialRepos: ["~/src/api", "~/src/web"],
                load: { (repos: ["~/src/api", "~/src/web"], agents: ["claude", "codex"]) },
                start: { _ in .failed("The host refused: branch already exists") }),
                style, "r18e-agent-task-\(look)")
        }
    }

    /// `color`'s 8-bit sRGB components and alpha.
    private static func bytes(_ color: UIColor) -> [Int] {
        var (r, g, b, a): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        color.getRed(&r, green: &g, blue: &b, alpha: &a)
        return [r, g, b, a].map { Int(($0 * 255).rounded()) }
    }

    @Test("the adaptive inks take each appearance's side; the dark side is the old dark ink")
    func inks() {
        let light = UITraitCollection(userInterfaceStyle: .light)
        let dark = UITraitCollection(userInterfaceStyle: .dark)
        let side = { (color: UIColor, traits: UITraitCollection) in Self.bytes(color.resolvedColor(with: traits)) }
        // The segmented control's track and pill (Chrome.installAppearance).
        #expect(side(InkUI.segTrack, light) == [0xEC, 0xEB, 0xE6, 255])
        #expect(side(InkUI.segTrack, dark) == Self.bytes(UIColor(Color(hex: "090B10").opacity(0.70))))
        #expect(side(InkUI.segActive, light) == [0xFF, 0xFF, 0xFF, 255])
        #expect(side(InkUI.segActive, dark) == Self.bytes(UIColor(Color(hex: "2A303A").opacity(0.98))))
        // The words on them read in both.
        #expect(side(InkUI.primary, light) != side(InkUI.primary, dark))
        #expect(side(InkUI.secondary, light) != side(InkUI.secondary, dark))
        // SwiftUI's side of a card's ink, as an environment resolves it.
        var lightEnvironment = EnvironmentValues()
        lightEnvironment.colorScheme = .light
        var darkEnvironment = EnvironmentValues()
        darkEnvironment.colorScheme = .dark
        let modal = { (environment: EnvironmentValues) in Self.bytes(UIColor(cgColor: Ink.modalBG.resolve(in: environment).cgColor)) }
        #expect(modal(lightEnvironment) == [0xFF, 0xFF, 0xFF, 255])
        #expect(modal(darkEnvironment) == [0x0C, 0x0E, 0x12, 255])
    }

    // MARK: The status bar over the terminal (R18, device probe)

    /// How the page pushed over Home keeps a dark appearance of its own.
    enum Keep: String, CaseIterable {
        /// Only its own view is dark (the NSH terminal's surface, R18).
        case scoped
        /// And `.toolbarColorScheme(.dark, for: .navigationBar)`, its bar hidden.
        case toolbar
        /// And `.preferredColorScheme(.dark)` (the window flips).
        case preferred
    }

    /// Home under a pushed page, both reporting the appearance they draw in.
    private struct Stack: View {
        let keep: Keep
        let home: (UIUserInterfaceStyle) -> Void
        let top: (UIUserInterfaceStyle) -> Void
        @State private var path = [1]

        var body: some View {
            NavigationStack(path: $path) {
                Trait(style: nil, seen: home)
                    .navigationDestination(for: Int.self) { _ in
                        pushed.navigationBarHidden(true)
                    }
            }
        }

        @ViewBuilder private var pushed: some View {
            let page = Trait(style: .dark, seen: top)
            switch keep {
            case .scoped: page
            case .toolbar: page.toolbarColorScheme(.dark, for: .navigationBar)
            case .preferred: page.preferredColorScheme(.dark)
            }
        }
    }

    /// A view that reports its trait's style, forced to `style` if given (as
    /// NSH forces its surface's root).
    private struct Trait: UIViewRepresentable {
        let style: UIUserInterfaceStyle?
        let seen: (UIUserInterfaceStyle) -> Void

        func makeUIView(context: Context) -> Reporter {
            let view = Reporter()
            view.seen = seen
            if let style { view.overrideUserInterfaceStyle = style }
            return view
        }

        func updateUIView(_ view: Reporter, context: Context) {}

        final class Reporter: UIView {
            var seen: (UIUserInterfaceStyle) -> Void = { _ in }
            override func layoutSubviews() {
                super.layoutSubviews()
                seen(traitCollection.userInterfaceStyle)
            }
        }
    }

    /// What the status bar does when a dark page is pushed over a light app,
    /// for each way of keeping the page dark: the status bar's style, and
    /// Home's appearance under the page. A probe for the device: it records
    /// `r18-status-bar.txt` in the app's tmp folder and checks only what
    /// does not depend on SwiftUI's choices.
    @Test("R18 probe: the status bar over a dark page pushed on a light app")
    func statusBarOverADarkPage() async throws {
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let before = scene.windows.first { $0.isKeyWindow }
        var lines: [String] = []
        for keep in Keep.allCases {
            var home = UIUserInterfaceStyle.unspecified
            var top = UIUserInterfaceStyle.unspecified
            let window = UIWindow(windowScene: scene)
            window.overrideUserInterfaceStyle = .light
            window.rootViewController = UIHostingController(rootView: Stack(
                keep: keep, home: { home = $0 }, top: { top = $0 }))
            window.makeKeyAndVisible()
            try? await Task.sleep(for: .milliseconds(800))
            let bar = scene.statusBarManager?.statusBarStyle ?? .default
            let words: (UIStatusBarStyle) -> String = {
                switch $0 {
                case .lightContent: "light content (for a dark page)"
                case .darkContent: "dark content (for a light page)"
                default: "default"
                }
            }
            let style: (UIUserInterfaceStyle) -> String = {
                switch $0 {
                case .light: "light"
                case .dark: "dark"
                default: "unspecified"
                }
            }
            lines.append("\(keep.rawValue): status bar \(words(bar)); Home \(style(home)); top \(style(top)); "
                         + "root preferred \(words(window.rootViewController?.preferredStatusBarStyle ?? .default))")
            #expect(top == .dark, "\(keep): the pushed page is dark")
            window.isHidden = true
            window.rootViewController = nil
        }
        before?.makeKey()
        let report = lines.joined(separator: "\n")
        print("[r18 status bar]\n" + report)
        try? report.write(to: FileManager.default.temporaryDirectory.appendingPathComponent("r18-status-bar.txt"),
                          atomically: true, encoding: .utf8)
    }
}

/// Settings › Set up a host never picks a host silently: with one saved
/// connection it goes straight in, with none or several it lists them; a
/// connected one opens its Host Setup, one that isn't is offered Connect.
/// The list's message and the page's "which host" message are the screens'.
@Suite("Set up a host: which host")
@MainActor
struct HostSetupRouteTests {
    static func host(_ n: Int, live: Bool) -> SettingsHost {
        SettingsHost(id: UUID(), name: "host-\(n)", user: "dev", host: "192.0.2.\(n)", port: 22, live: live)
    }

    @Test("the hub's row: straight in with one saved connection, the list with none or several")
    func entering() {
        let (live, cold) = (Self.host(1, live: true), Self.host(2, live: false))
        #expect(HostSetupRoute.entering([]) == .list)
        #expect(HostSetupRoute.entering([live]) == .setup(live.id))
        #expect(HostSetupRoute.entering([cold]) == .connect(cold.id))
        #expect(HostSetupRoute.entering([live, cold]) == .list)
        // Two connected: the user picks; the first in Home's order is not taken.
        #expect(HostSetupRoute.entering([live, Self.host(3, live: true)]) == .list)
    }

    @Test("a pick on the list: a connected one's setup, Connect first for one that isn't")
    func choosing() {
        let (live, cold) = (Self.host(1, live: true), Self.host(2, live: false))
        let hosts = [live, cold]
        #expect(HostSetupRoute.choosing(live.id, in: hosts) == .setup(live.id))
        #expect(HostSetupRoute.choosing(cold.id, in: hosts) == .connect(cold.id))
        #expect(HostSetupRoute.choosing(UUID(), in: hosts) == nil, "a connection no longer saved")
    }

    @Test("a saved connection on the list is its name (user@host without one), user, host and port")
    func entries() {
        let named = ServerConnection(name: "gpu-01", host: "192.0.2.10", port: 2222, username: "dev")
        let host = SettingsHost(named, live: true)
        #expect(host.name == "gpu-01" && host.user == "dev" && host.host == "192.0.2.10" && host.port == 2222)
        #expect(host.id == named.id && host.live)
        let nameless = ServerConnection(host: "192.0.2.11", username: "ci")
        #expect(SettingsHost(nameless, live: false).name == "ci@192.0.2.11")
    }

    @Test("the list's message and the page's host message are the screens'")
    func messages() {
        let hosts = [Self.host(1, live: true), Self.host(2, live: false)]
        let list = SettingsHost.message(hosts)
        #expect(NSHSurface.check(screen: NSHScreen.settings.name, message: list))
        #expect((list["hosts"] as? [[String: Any]])?.count == 2)
        let connection = ServerConnection(name: "gpu-01", host: "192.0.2.10", port: 2222, username: "dev")
        let named = HostSetupScreenModel.host(connection)
        #expect(NSHSurface.check(screen: NSHScreen.hostSetup.name, message: named))
        #expect(named["name"] as? String == "gpu-01")
    }
}
