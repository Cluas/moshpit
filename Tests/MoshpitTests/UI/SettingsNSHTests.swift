import Foundation
import Testing
import UIKit
@testable import Moshpit

/// Settings › App Icon, Accent, Voice Recognition and Voice Language in NSH:
/// their switches, the data path between the host and each screen — every
/// state the Swift side posts is a message the Rust screen takes
/// (`ui/src/*.rs`'s tests and `ui/tests/*.rs` cover the screens), what the
/// screens tell the host is applied as the SwiftUI pages apply it — and real
/// surfaces on the device that run each screen, show what is posted and
/// report what the user does.
@Suite("NSH Settings pages")
@MainActor
struct SettingsNSHTests {

    /// Settings of their own, so a test writes nothing the app keeps.
    private static func scratch() -> (AppSettings, AppThemeStore, UserDefaults) {
        let defaults = UserDefaults(suiteName: "w11b.nsh.\(UUID().uuidString)")!
        return (AppSettings(defaults: defaults), AppThemeStore(defaults: defaults), defaults)
    }

    // MARK: Screens

    @Test("each page runs the Rust screen of its name")
    func names() {
        let screens: [NSHScreen] = [.appIcon, .accent, .voiceEngine, .voiceLanguage]
        #expect(screens.map(\.name) == ["app-icon", "accent", "voice-engine", "voice-language"])
    }

    // MARK: App Icon

    @Test("every App Icon state is a message the Rust screen takes; bad ones are refused")
    func appIconMessages() {
        let screen = NSHScreen.appIcon.name
        for (selected, failure) in [("default", nil), ("pit", "The operation couldn’t be completed."), ("gone", nil)] {
            let message = AppIconModel.message(selected: selected, failure: failure)
            #expect(NSHSurface.check(screen: screen, message: message), "\(message)")
        }
        #expect(!NSHSurface.check(screen: screen, message: ["op": "icons", "icons": []]))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "accents", "selected": "x"]))
        let icons = AppIconModel.message(selected: "default", failure: nil)["icons"] as? [[String: Any]]
        #expect(icons?.map { $0["id"] as? String } == AppIconCatalog.all.map(\.id))
    }

    @Test("an icon's preview is the bundled file the SwiftUI page loads, as an asset URI")
    func appIconPreviews() throws {
        for option in AppIconCatalog.all {
            let uri = try #require(AppIconModel.previewURI(option), "\(option.id)")
            #expect(uri == "asset:/\(option.previewAsset)@3x.png")
            let path = Bundle.main.resourcePath! + "/" + uri.dropFirst("asset:/".count)
            #expect(FileManager.default.fileExists(atPath: path), "\(path)")
            #expect(UIImage(named: option.previewAsset) != nil)
        }
        let missing = AppIconOption(id: "x", name: "X", alternateName: "AppIcon-X", previewAsset: "IconPreview-Nope")
        #expect(AppIconModel.previewURI(missing) == nil)
        #expect(AppIconModel.icon(missing)["image"] == nil)
    }

    @Test("picking an icon switches it first; the setting follows only what iOS accepted")
    func appIconSelect() async throws {
        let (settings, _, _) = Self.scratch()
        settings.appIconId = "default"
        let model = AppIconModel()
        var asked: [String?] = []
        var refuse = false
        var shown: String?
        model.shownIcon = { shown }
        model.setIcon = { name, done in
            asked.append(name)
            if !refuse { shown = name }
            done(refuse ? NSError(domain: "test", code: 1, userInfo: [NSLocalizedDescriptionKey: "No."]) : nil)
        }
        model.select("default", settings: settings)
        #expect(asked.isEmpty, "the icon in use: nothing to do")
        model.select("pit", settings: settings)
        try await Self.settle { settings.appIconId == "pit" }
        #expect(asked == ["AppIcon-Pit"])
        #expect(model.failure == nil)
        refuse = true
        model.select("wq", settings: settings)
        try await Self.settle { model.failure != nil }
        #expect(settings.appIconId == "pit", "a refused change is not recorded")
        #expect(model.failure == "No.")
        refuse = false
        model.select("default", settings: settings)
        try await Self.settle { settings.appIconId == "default" }
        #expect(asked.last == .some(nil), "the primary icon is no alternate")
        #expect(model.failure == nil)
        model.select("no-such-icon", settings: settings)
        #expect(asked.count == 3)
    }

    /// B668: the pet's icon card switches through the gallery's own call,
    /// and a switch it made is not one picked by hand (its next card
    /// defaults to 「换」); one picked in the gallery is.
    @Test("an icon the pet's card switched to is not picked by hand")
    func appIconPickedByHand() async throws {
        let (settings, _, defaults) = Self.scratch()
        settings.appIconId = "default"
        #expect(!settings.appIconPickedByHand, "the primary, never picked")
        let model = AppIconModel()
        var shown: String?
        model.shownIcon = { shown }
        model.setIcon = { name, done in shown = name; done(nil) }
        var switched: Bool?
        model.select("tun-shiny", settings: settings, byHand: false) { switched = $0 }
        try await Self.settle { switched != nil }
        #expect(switched == true && settings.appIconId == "tun-shiny")
        #expect(shown == "AppIcon-TunShiny")
        // B704 (CI run 14): the Rust settings store took this write for no
        // setting, and every icon not the primary read as picked by hand.
        #expect(!settings.appIconPickedByHand)
        #expect(!AppSettings(defaults: defaults).appIconPickedByHand, "kept for the next launch")
        model.select("classic", settings: settings)
        try await Self.settle { settings.appIconId == "classic" }
        #expect(settings.appIconPickedByHand)
        #expect(AppSettings(defaults: defaults).appIconPickedByHand)
        // The card's offer, read from the pet's message.
        #expect(CompanionPet.offeredIcon(#"{"op":"pet","icon":{"id":"tun","species":"tun","keep_default":true}}"#) == "tun")
        #expect(CompanionPet.offeredIcon(#"{"op":"pet"}"#) == nil)
    }

    /// What the answers to Home's icon card did.
    private final class Answers {
        var answered = 0
        var synced = 0
        var switches: [String] = []
    }

    /// B608: 「换」 was the answer whatever iOS said. A switch iOS refused
    /// took the card away, left the icon as it was and never offered this
    /// look again; refused now, the card stays. 「先不换」 is the answer at
    /// once and switches nothing.
    @Test("a 「换」 iOS refuses is no answer: the card asks again")
    func iconSwitchRefusedIsNoAnswer() {
        let refused = Answers()
        CompanionPet.answer(switch: true, offered: "tun",
                            select: { id, done in refused.switches.append(id); done(false) },
                            answered: { refused.answered += 1 }, synced: { refused.synced += 1 })
        #expect(refused.switches == ["tun"])
        #expect(refused.answered == 0, "a refused switch answered the card")
        #expect(refused.synced == 1)

        let switched = Answers()
        CompanionPet.answer(switch: true, offered: "tun",
                            select: { _, done in done(true) },
                            answered: { switched.answered += 1 }, synced: { switched.synced += 1 })
        #expect(switched.answered == 1 && switched.synced == 1)

        let declined = Answers()
        CompanionPet.answer(switch: false, offered: "tun",
                            select: { id, _ in declined.switches.append(id) },
                            answered: { declined.answered += 1 }, synced: { declined.synced += 1 })
        #expect(declined.switches.isEmpty && declined.answered == 1)
    }

    @Test("the tick is the icon iOS shows, not a stale setting")
    func appIconTick() {
        let (settings, _, _) = Self.scratch()
        let model = AppIconModel()
        var shown: String? = "AppIcon-Pit"
        model.shownIcon = { shown }
        settings.appIconId = "teal-from-an-old-catalog"
        #expect(model.selectedID(settings) == "pit")
        shown = nil
        #expect(model.selectedID(settings) == "default", "the primary icon")
        shown = "AppIcon-Removed"
        #expect(model.selectedID(settings) == "teal-from-an-old-catalog", "iOS names no icon of the catalog")
        // Picking the icon iOS already shows does nothing, whatever is stored.
        shown = "AppIcon-Wq"
        settings.appIconId = "default"
        var asked = 0
        model.setIcon = { _, done in asked += 1; done(nil) }
        model.select("wq", settings: settings)
        #expect(asked == 0)
    }

    // MARK: Accent

    @Test("every Accent state is a message the Rust screen takes")
    func accentMessages() throws {
        let mine = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        for (custom, selected) in [([mine], mine.id), ([], "signal-room")] {
            let state = AccentModel.state(builtIns: AppThemeCatalog.builtIns, custom: custom, selected: selected)
            #expect(NSHSurface.check(screen: NSHScreen.accent.name,
                                     json: try pageMessage(message: .accent(state: state))))
        }
        var accent = AccentModel.accent(mine)
        #expect(accent.accentHex == "#2FD8A8")
        #expect(accent.screenBgHex == NSHScreenParams.hex(mine.screenBG))
        accent.accentHex = "teal"
        #expect(throws: PageError.self) {
            try pageMessage(message: .accent(state: AccentPageState(
                builtIn: [accent], custom: [], selected: accent.id, deleting: nil)))
        }
    }

    @Test("a pick is ticked at once and applied as the page goes away, as on the SwiftUI page")
    func accentPick() {
        let (settings, store, _) = Self.scratch()
        settings.appThemeId = "signal-room"
        let model = AccentModel()
        #expect(model.selectedID(settings, store: store) == "signal-room")
        model.select("terminal-green", settings: settings, store: store)
        #expect(model.selectedID(settings, store: store) == "terminal-green")
        #expect(settings.appThemeId == "signal-room", "not applied while the page shows")
        model.commit(settings)
        #expect(settings.appThemeId == "terminal-green")
        // A stored id that names no accent resolves to Signal Room.
        settings.appThemeId = "deleted-long-ago"
        #expect(AccentModel().selectedID(settings, store: store) == "signal-room")
    }

    /// B609: a pick waited for the page to go away, which a phone that ends
    /// the app in the background never lets happen: the next launch wore
    /// the old accent. It applies as the app leaves too.
    @Test("a pick applies as the app goes to the background with the page up")
    func accentPickKeptOnLeaving() {
        let (settings, store, _) = Self.scratch()
        settings.appThemeId = "signal-room"
        let center = NotificationCenter()
        let model = AccentModel()
        model.commitOnLeaving(settings, center: center)
        model.commitOnLeaving(settings, center: center)
        model.select("terminal-green", settings: settings, store: store)
        #expect(settings.appThemeId == "signal-room", "not applied while the page shows")
        center.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
        #expect(settings.appThemeId == "terminal-green", "the pick was lost as the app left")
    }

    @Test("deleting the picked custom accent falls back to Signal Room")
    func accentRemove() {
        let (settings, store, _) = Self.scratch()
        let mine = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        store.save(mine)
        settings.appThemeId = mine.id
        let model = AccentModel()
        #expect(model.selectedID(settings, store: store) == mine.id)
        model.remove(mine, settings: settings, store: store)
        #expect(store.customThemes.isEmpty)
        #expect(settings.appThemeId == "signal-room")
        #expect(model.pendingID == "signal-room")
        // Deleting one that is not picked leaves the pick alone.
        let other = AppTheme.custom(id: "app-ffff0000", name: "Red", accentHex: "FF0000")
        store.save(other)
        model.select("amber-console", settings: settings, store: store)
        model.remove(other, settings: settings, store: store)
        #expect(model.pendingID == "amber-console")
    }

    @Test("a custom accent's row has the SwiftUI page's context menu")
    func accentMenuActions() {
        let menus = AccentRowMenus()
        let mine = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        var edited: [String] = []
        var deleted: [String] = []
        menus.onEdit = { edited.append($0.id) }
        menus.onDelete = { deleted.append($0.id) }
        let actions = menus.actions(for: mine)
        #expect(actions.map(\.title) == [String(localized: "Edit"), String(localized: "Delete")])
        #expect(actions[1].attributes.contains(.destructive))
        #expect(AccentRowMenus.cellID(mine.id) == "accent-cell-app-1a2b3c4d")
    }

    // MARK: Voice

    @Test("every Recognition state is a message the Rust screen takes")
    func voiceEngineMessages() {
        for engine in VoiceEngineKind.allCases {
            for installed in [0, 2] {
                let message = VoiceEngineModel.message(engine: engine, installedModels: installed)
                #expect(NSHSurface.check(screen: NSHScreen.voiceEngine.name, message: message), "\(message)")
                #expect(message["needsModel"] as? Bool == (engine == .whisper && installed == 0))
            }
        }
        #expect(!NSHSurface.check(screen: NSHScreen.voiceEngine.name, message: ["op": "engine", "engine": "siri",
                                                                             "needsModel": false]))
    }

    @Test("picking Whisper leaves it on auto-detect (B670)")
    func voiceEngineSelect() {
        let (settings, _, _) = Self.scratch()
        settings.voiceEngine = .apple
        settings.whisperLanguage = ""
        let model = VoiceEngineModel()
        model.countInstalled = { 0 }
        model.select(.whisper, settings: settings)
        #expect(settings.voiceEngine == .whisper)
        #expect(settings.whisperLanguage == "")
        settings.whisperLanguage = "ja"
        model.select(.apple, settings: settings)
        model.select(.whisper, settings: settings)
        #expect(settings.whisperLanguage == "ja", "a language already picked stays")
    }

    @Test("every Language state is a message the Rust screen takes")
    func voiceLanguageMessages() {
        let options = [VoiceLanguageModel.Option(id: "en-US", name: "English (United States)")]
        let messages = [
            VoiceLanguageModel.appleMessage(automatic: "English", options: nil, selected: ""),
            VoiceLanguageModel.appleMessage(automatic: "English", options: options, selected: "en-US"),
            VoiceLanguageModel.whisperMessage(suggested: [], all: [.init(id: "zh", name: "Chinese")], selected: ""),
            VoiceLanguageModel.whisperMessage(
                suggested: [.init(id: "zh", name: "Chinese"), .init(id: "en", name: "English")],
                all: WhisperLanguageCatalog.options().map { .init(id: $0.id, name: $0.name) },
                selected: "zh"),
        ]
        for message in messages {
            #expect(NSHSurface.check(screen: NSHScreen.voiceLanguage.name, message: message), "\(message)")
        }
        #expect(messages[0]["options"] == nil, "still reading: no options")
        #expect(!NSHSurface.check(screen: NSHScreen.voiceLanguage.name,
                                  message: ["op": "languages", "engine": "whisper", "selected": ""]))
    }

    @Test("a picked language goes to the setting of the engine in use")
    func voiceLanguageSelect() async {
        let (settings, _, _) = Self.scratch()
        settings.voiceEngine = .apple
        let model = VoiceLanguageModel()
        model.readAppleOptions = { [VoiceLanguageModel.Option(id: "en-US", name: "English")] }
        await model.load(settings: settings)
        #expect(model.appleOptions == [VoiceLanguageModel.Option(id: "en-US", name: "English")])
        model.select("en-US", settings: settings)
        #expect(settings.voiceInputLocaleId == "en-US")
        settings.voiceEngine = .whisper
        model.select("zh", settings: settings)
        #expect(settings.whisperLanguage == "zh")
        #expect(settings.voiceInputLocaleId == "en-US")
        model.select("", settings: settings)
        #expect(settings.whisperLanguage == "")
    }

    // MARK: On the device

    @Test("App Icon on a real surface: the posted icons show and a tap asks the host")
    func appIconSurface() async throws {
        let (surface, heard, done) = try Self.surface(running: .appIcon)
        defer { done() }
        #expect(surface.post(AppIconModel.message(selected: "pit", failure: nil)))
        let pit = try #require(await Self.view(identified: "app-icon-pit", in: surface))
        #expect(pit.accessibilityTraits.contains(.selected))
        let island = try #require(await Self.view(identified: "app-icon-island", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(island.accessibilityActivate())
        let event = try #require(await heard.next())
        #expect(event["ev"] as? String == "select")
        #expect(event["id"] as? String == "island")
    }

    @Test("Accent on a real surface: a custom accent's row gets the context menu, a built-in's none")
    func accentSurface() async throws {
        let (surface, heard, done) = try Self.surface(running: .accent)
        defer { done() }
        let mine = AppTheme.custom(id: "app-1a2b3c4d", name: "Mint", accentHex: "2FD8A8")
        #expect(surface.post(.accent(state: AccentModel.state(
            builtIns: AppThemeCatalog.builtIns, custom: [mine], selected: "signal-room"))))
        let cell = try #require(await Self.view(identified: AccentRowMenus.cellID(mine.id), in: surface))
        let builtIn = try #require(await Self.view(identified: "accent-row-moshpit-classic", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        let menus = AccentRowMenus()
        menus.editable = { [mine] }
        menus.install(on: surface)
        let interaction = try #require(surface.interactions.compactMap { $0 as? UIContextMenuInteraction }.first)
        let inCell = surface.convert(CGPoint(x: cell.bounds.midX, y: cell.bounds.midY), from: cell)
        let configuration = try #require(menus.contextMenuInteraction(interaction, configurationForMenuAtLocation: inCell))
        #expect(configuration.identifier as? String == mine.id)
        let onBuiltIn = surface.convert(CGPoint(x: builtIn.bounds.midX, y: builtIn.bounds.midY), from: builtIn)
        #expect(menus.contextMenuInteraction(interaction, configurationForMenuAtLocation: onBuiltIn) == nil)
        #expect(builtIn.accessibilityActivate())
        let event = try #require(await heard.next())
        #expect(event["ev"] as? String == "select")
        #expect(event["id"] as? String == "moshpit-classic")
    }

    @Test("Recognition and Language on real surfaces: what is posted shows, a pick reaches the host")
    func voiceSurfaces() async throws {
        do {
            let (surface, heard, done) = try Self.surface(running: .voiceEngine)
            defer { done() }
            #expect(surface.post(VoiceEngineModel.message(engine: .whisper, installedModels: 0)))
            let models = try #require(await Self.view(identified: "voice-engine-models", in: surface))
            await NSHTouchAudit.pressablesTakeTouches(in: surface)
            #expect(models.accessibilityActivate())
            #expect(try #require(await heard.next())["ev"] as? String == "models")
        }
        do {
            let (surface, heard, done) = try Self.surface(running: .voiceLanguage)
            defer { done() }
            #expect(surface.post(VoiceLanguageModel.appleMessage(
                automatic: "English", options: [.init(id: "en-US", name: "English (United States)")],
                selected: "")))
            let automatic = try #require(await Self.view(identified: "voice-language-automatic", in: surface))
            #expect(automatic.accessibilityTraits.contains(.selected))
            let english = try #require(await Self.view(identified: "voice-language-en-US", in: surface))
            await NSHTouchAudit.pressablesTakeTouches(in: surface)
            #expect(english.accessibilityActivate())
            let event = try #require(await heard.next())
            #expect(event["ev"] as? String == "select")
            #expect(event["id"] as? String == "en-US")
        }
    }

    // MARK: Helpers

    /// What a surface's screen told the host, in order.
    @MainActor
    final class Heard {
        private var messages: [[String: Any]] = []
        func append(_ message: [String: Any]) { messages.append(message) }
        /// The next message, waiting up to ``NSHTestWait/ceiling``.
        func next() async -> [String: Any]? {
            let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
            while messages.isEmpty, Date() < deadline {
                try? await Task.sleep(for: .milliseconds(20))
            }
            return messages.isEmpty ? nil : messages.removeFirst()
        }
    }

    /// A surface in a window of the test host's scene, running `screen`
    /// (on its page `page`, `ui/src/screen.rs`'s `Params.page`).
    static func surface(running screen: NSHScreen, page: String? = nil) throws -> (NSHSurface, Heard, () -> Void) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        window.addSubview(surface)
        window.isHidden = false
        let heard = Heard()
        surface.onMessage = { heard.append($0) }
        var params = NSHScreenParams.current()
        if let page { params["page"] = page }
        #expect(surface.run(screen: screen.name, params: params))
        return (surface, heard, {
            surface.removeFromSuperview()
            window.isHidden = true
        })
    }

    /// The view under `root` with accessibility identifier `id`, waiting up
    /// to ``NSHTestWait/ceiling`` for the screen to show it. The deepest
    /// one wins: R3's controls (Switch, Segmented, Select) carry their id
    /// both on NSH's wrapper and, for UIKit, on the control inside it, and a
    /// test asks for the control.
    static func view(identified id: String, in root: UIView) async -> UIView? {
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while Date() < deadline {
            if let hit = deepest(identified: id, in: root) { return hit }
            try? await Task.sleep(for: .milliseconds(20))
        }
        // Why, when it is the host and not the screen: a host out of the
        // foreground draws nothing.
        Issue.record(Comment(rawValue: "\(id) never showed; " + hostState(root)))
        return nil
    }

    /// The deepest view under `root` identified `id`, in the first subtree
    /// that has one.
    static func deepest(identified id: String, in root: UIView) -> UIView? {
        for child in root.subviews {
            if let hit = deepest(identified: id, in: child) { return hit }
        }
        return root.accessibilityIdentifier == id ? root : nil
    }

    /// Where the test host is, for a wait that ran out.
    static func hostState(_ root: UIView) -> String {
        let app = UIApplication.shared.applicationState
        let scene = root.window?.windowScene
        let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .map { "\($0.windows.count)w/\($0.activationState.rawValue)" }
        func count(_ view: UIView) -> Int { 1 + view.subviews.reduce(0) { $0 + count($1) } }
        return "app=\(app.rawValue) scene=\(scene.map { String($0.activationState.rawValue) } ?? "none")"
            + " scenes=\(windows) inWindow=\(root.window != nil) views=\(count(root))"
            + " uptime=\(Int(ProcessInfo.processInfo.systemUptime))"
    }

    /// Waits up to ``NSHTestWait/ceiling`` for `condition`.
    private static func settle(_ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while !condition(), Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(condition())
    }
}
