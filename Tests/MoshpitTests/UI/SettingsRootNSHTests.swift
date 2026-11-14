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

    @Test("the page has its own switch, off unless set")
    func switchKey() {
        #expect(NSHScreen.settings.switchKey == "MOSHPIT_NSH_SETTINGS")
        #expect(NSHScreen.settings.name == "settings")
        let registered = UserDefaults.standard.volatileDomain(forName: UserDefaults.registrationDomain)
        #expect(registered[NSHScreen.settings.switchKey] == nil)
        #expect(Set(NSHScreen.allCases.map(\.switchKey)).count == NSHScreen.allCases.count)
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
        bad.removeValue(forKey: "keepAlive")
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
        #expect(SettingsModel.apply("fontSize", 40, to: settings))
        #expect(settings.fontSize == 18, "clamped to the slider's range")
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
        #expect(!SettingsModel.apply("keepAlive", "yes", to: settings))
        #expect(!SettingsModel.apply("colour", true, to: settings))
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
            == ["appearance", "terminal", "connections", "notifications", "voice"])
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
        #expect((set["value"] as? Double) == min(max(settings.fontSize.rounded() + 1, 8), 18))
    }

    @Test("on a real surface: Connections' switches carry the settings, and a flip reaches the host")
    func connectionsSurface() async throws {
        let (settings, _, _, _) = Self.scratch()
        settings.keepConnectionsAlive = true
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .settings, page: "connections")
        defer { done() }
        #expect(surface.post(SettingsModel.message(Self.snapshot(settings))))
        let keepAlive = try #require(await SettingsNSHTests.view(identified: "settings-keep-alive", in: surface)
            as? UISwitch)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(keepAlive.isOn)
        keepAlive.setOn(false, animated: false)
        keepAlive.sendActions(for: .valueChanged)
        let set = try #require(await heard.next())
        #expect(set["ev"] as? String == "set")
        #expect(set["key"] as? String == "keepAlive")
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
