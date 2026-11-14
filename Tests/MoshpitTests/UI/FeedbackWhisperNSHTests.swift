import Foundation
import Testing
import UIKit
@testable import Moshpit

/// Settings › Send Feedback and Settings › Voice Input › Model in NSH: their
/// switches, the data path between the host and each screen (every state the
/// Swift side posts is a message the Rust screen takes; `ui/tests/*.rs` cover
/// the screens), what the screens tell the host done as the SwiftUI pages do
/// it, and real surfaces on the device.
@Suite("NSH Feedback and Whisper model pages")
@MainActor
struct FeedbackWhisperNSHTests {

    @Test("each page has its own switch, off unless set")
    func switches() {
        #expect(NSHScreen.feedback.switchKey == "MOSHPIT_NSH_FEEDBACK")
        #expect(NSHScreen.whisperModel.switchKey == "MOSHPIT_NSH_WHISPER_MODEL")
        #expect([NSHScreen.feedback, .whisperModel].map(\.name) == ["feedback", "whisper-model"])
        let registered = UserDefaults.standard.volatileDomain(forName: UserDefaults.registrationDomain)
        #expect(registered[NSHScreen.feedback.switchKey] == nil)
        #expect(registered[NSHScreen.whisperModel.switchKey] == nil)
    }

    // MARK: Feedback

    @Test("every Feedback state is a message the Rust screen takes")
    func feedbackMessages() {
        let screen = NSHScreen.feedback.name
        for text in [nil, "", "It froze when I rotated the iPad"] {
            for flags in [(true, false, false, false, false), (false, true, true, true, true)] {
                let message = FeedbackModel.message(
                    text: text, environment: flags.0, log: flags.1, preparing: flags.2,
                    showAddress: flags.3, copied: flags.4)
                #expect(NSHSurface.check(screen: screen, message: message), "\(message)")
            }
        }
        #expect(!NSHSurface.check(screen: screen, message: ["op": "feedback", "environment": true]))
    }

    @Test("Send assembles the mail as the SwiftUI page does, else shows the address")
    func feedbackSend() async throws {
        let model = FeedbackModel()
        model.transcript = { "09:41 log line" }
        model.message = "It froze"
        model.set("log", true)
        model.set("environment", false)
        var composed: [FeedbackMail] = []
        model.canSendMail = { true }
        model.send(compose: { composed.append($0) }, openMailto: { _, _ in Issue.record("no mailto with Mail") })
        try await Self.settle { !composed.isEmpty }
        #expect(composed.first?.body.contains("It froze") == true)
        #expect(composed.first?.log == "09:41 log line")
        #expect(!model.isPreparing)

        // No Mail, and no app takes mailto: the address shows.
        model.canSendMail = { false }
        model.send(compose: { _ in Issue.record("no Mail") }, openMailto: { _, done in done(false) })
        try await Self.settle { model.showAddress }
        var copied: [String] = []
        model.copy = { copied.append($0) }
        model.copyAddress()
        #expect(copied == [FeedbackMail.recipient])
        #expect(model.copied)
    }

    @Test("Feedback on a real surface: UIKit's switches, the message field, Send reaching the host")
    func feedbackSurface() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .feedback)
        defer { done() }
        #expect(surface.post(FeedbackModel.message(
            text: "Half a sentence", environment: true, log: false, preparing: false,
            showAddress: false, copied: false)))
        let environment = try #require(await SettingsNSHTests.view(identified: "feedback-environment", in: surface)
            as? UISwitch)
        #expect(environment.isOn)
        #expect(await SettingsNSHTests.view(identified: "feedback-message", in: surface) != nil)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        let send = try #require(await SettingsNSHTests.view(identified: "feedback-send", in: surface))
        // The field takes the keyboard as the page opens; the text events
        // that may come first are not what this checks.
        #expect(send.accessibilityActivate())
        var event = try #require(await heard.next())
        while event["ev"] as? String == "text" { event = try #require(await heard.next()) }
        #expect(event["ev"] as? String == "send")
        #expect(event["message"] as? String == "Half a sentence")
    }

    // MARK: Whisper models

    private static func seededOptions() -> [WhisperModelOption] {
        [
            WhisperModelOption(id: "big", name: "Large v3 Turbo", summary: "Most accurate.",
                               approximateBytes: 954_000_000, installedBytes: 1_621_000_000, partialBytes: 0),
            WhisperModelOption(id: "small", name: "Small", summary: "Balanced.",
                               approximateBytes: 470_000_000, installedBytes: nil, partialBytes: 120_000_000),
            WhisperModelOption(id: "tiny", name: "Tiny", summary: "Fastest.",
                               approximateBytes: 75_000_000, installedBytes: nil, partialBytes: 0),
        ]
    }

    @Test("every Model state is a message the Rust screen takes")
    func whisperMessages() {
        let screen = NSHScreen.whisperModel.name
        let options = Self.seededOptions()
        let message = WhisperModelPageModel.message(
            options: options, progress: ["small": 0.42], failure: "The network connection was lost.",
            selected: "big")
        #expect(NSHSurface.check(screen: screen, message: message), "\(message)")
        let models = try? #require(message["models"] as? [[String: Any]])
        #expect(models?[0]["selected"] as? Bool == true)
        #expect(models?[0]["installed"] as? String == WhisperModelPageModel.format(1_621_000_000))
        #expect(models?[1]["partial"] as? String == WhisperModelPageModel.format(120_000_000))
        #expect(models?[1]["progress"] as? Double == 0.42)
        #expect(models?[2]["partial"] == nil)
        #expect(message["storage"] as? String == WhisperModelPageModel.format(1_621_000_000))
        let empty = WhisperModelPageModel.message(options: [], progress: [:], failure: nil, selected: nil)
        #expect(NSHSurface.check(screen: screen, message: empty))
        #expect(empty["storage"] == nil)
        #expect(!NSHSurface.check(screen: screen, message: ["op": "models", "models": [["id": "x"]]]))
    }

    @Test("a row picks only a downloaded model that is not in use")
    func whisperSelect() {
        let defaults = UserDefaults(suiteName: "w11c.whisper.\(UUID().uuidString)")!
        let settings = AppSettings(defaults: defaults)
        let model = WhisperModelPageModel()
        model.catalog = { Self.seededOptions() }
        model.reload(settings: settings)
        model.select("small", settings: settings)
        #expect(settings.whisperModelId == "", "not downloaded")
        model.select("ghost", settings: settings)
        #expect(settings.whisperModelId == "")
    }

    @Test("Model on a real surface: the rows, the progress bar, a download reaching the host")
    func whisperSurface() async throws {
        let (surface, heard, done) = try SettingsNSHTests.surface(running: .whisperModel)
        defer { done() }
        #expect(surface.post(WhisperModelPageModel.message(
            options: Self.seededOptions(), progress: ["small": 0.42], failure: nil, selected: "big")))
        let big = try #require(await SettingsNSHTests.view(identified: "whisper-model-big", in: surface))
        #expect(big.accessibilityTraits.contains(.selected))
        let bar = try #require(Self.first(UIProgressView.self, in: surface))
        #expect(abs(bar.progress - 0.42) < 0.001)
        let download = try #require(await SettingsNSHTests.view(identified: "whisper-model-download-tiny",
                                                                in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(download.accessibilityActivate())
        let event = try #require(await heard.next())
        #expect(event["ev"] as? String == "download")
        #expect(event["id"] as? String == "tiny")
    }

    // MARK: The redesign's pages, drawn (R15, R16, Settings)

    /// One picture: the page, the state it is posted (a host model's
    /// message, or the JSON a host posts), what the picture is called, and
    /// the screen's page for a screen of several (Settings').
    private struct Shot {
        let screen: NSHScreen
        let root: String
        let state: String
        var message: [String: Any]?
        var json: String?
        var page: String?
    }

    /// Settings' hub and its five pages as the app posts them, each shown
    /// once the group that ends it is.
    private static func settingsShots() -> [Shot] {
        let (settings, _, _, _) = SettingsRootNSHTests.scratch()
        let message = SettingsModel.message(SettingsRootNSHTests.snapshot(settings))
        let pages: [(state: String, page: String?, root: String)] = [
            ("hub", nil, "settings-open-appearance"),
            ("appearance", "appearance", "settings-appearance-group"),
            ("terminal", "terminal", "settings-cursor-group"),
            ("connections", "connections", "settings-mosh-group"),
            ("notifications", "notifications", "settings-notifications-group"),
            ("voice", "voice", "settings-voice-group"),
        ]
        return pages.map { shown in
            Shot(screen: .settings, root: shown.root, state: shown.state, message: message, page: shown.page)
        }
    }

    /// Set up a host (R16) as the core would post it: a fresh, healthy host,
    /// with `changes` laid over it (`NSNull` takes a field out).
    private static func hostSetup(_ changes: [String: Any]) -> [String: Any] {
        var setup: [String: Any] = [
            "op": "host-setup",
            "phase": "idle",
            "facts": [
                "uname": "Darwin",
                "tools": ["openssl", "curl", "jq", "tmux"].map { ["name": $0, "present": true] as [String: Any] },
                "missingForPush": [String](),
            ] as [String: Any],
            "nothingWillPush": false,
            "agent": hostAgent("claude", "Claude Code", "~/.claude/settings.json", "absent"),
            "agents": hostAgents(claude: "absent"),
            "hooks": "absent",
            "hooksProof": ["state": "untested"],
            "relayURL": "https://push.example.com",
            "pairing": "absent",
            "pushProof": ["state": "untested"],
            "removable": false,
        ]
        for (key, value) in changes {
            if value is NSNull { setup.removeValue(forKey: key) } else { setup[key] = value }
        }
        return setup
    }

    private static func hostAgent(_ id: String, _ name: String, _ hint: String, _ status: String,
                                  found: Bool = true) -> [String: Any] {
        ["id": id, "name": name, "configHint": hint, "status": status, "found": found]
    }

    /// The seven agents, Claude Code's status scripts `claude`.
    private static func hostAgents(claude: String) -> [[String: Any]] {
        [
            hostAgent("claude", "Claude Code", "~/.claude/settings.json", claude),
            hostAgent("codex", "Codex", "~/.codex/config.toml", "absent"),
            hostAgent("gemini", "Gemini CLI", "~/.gemini/settings.json", "absent", found: false),
            hostAgent("qwen", "Qwen Code", "~/.qwen/settings.json", "absent", found: false),
            hostAgent("qoder", "Qoder", "~/.qoder/settings.json", "absent", found: false),
            hostAgent("factory", "Factory", "~/.factory/settings.json", "absent", found: false),
            hostAgent("codebuddy", "CodeBuddy", "~/.codebuddy/settings.json", "absent", found: false),
        ]
    }

    /// Host Setup's states for the pictures (R16).
    private static func hostSetupShots() -> [Shot] {
        let installed: [String: Any] = [
            "hooks": "current",
            "pairing": "current",
            "pairedRelay": "https://push.example.com",
            "agent": hostAgent("claude", "Claude Code", "~/.claude/settings.json", "current"),
            "agents": hostAgents(claude: "current"),
            "removable": true,
        ]
        let with = { (changes: [String: Any]) in installed.merging(changes) { $1 } }
        let unknown: [String: Any] = ["hooks": "unknown", "pairing": "unknown", "facts": NSNull()]
        return [
            Shot(screen: .hostSetup, root: "hostsetup-looking", state: "looking",
                 message: hostSetup(unknown.merging(["phase": "inspecting",
                                                     "phaseLabel": "Looking at the host…"]) { $1 })),
            Shot(screen: .hostSetup, root: "hostsetup-unread", state: "unread",
                 message: hostSetup(unknown.merging(["error": "Connection refused"]) { $1 })),
            Shot(screen: .hostSetup, root: "hostsetup-host", state: "fresh", message: hostSetup([:])),
            Shot(screen: .hostSetup, root: "hostsetup-host", state: "installed",
                 message: hostSetup(with(["hooksProof": ["state": "proven"],
                                          "pushProof": ["state": "proven"]]))),
            Shot(screen: .hostSetup, root: "hostsetup-host", state: "warnings",
                 message: hostSetup(with([
                    "hooks": "stale",
                    "agent": hostAgent("claude", "Claude Code", "~/.claude/settings.json", "stale"),
                    "agents": hostAgents(claude: "stale"),
                    "facts": [
                        "uname": "Linux",
                        "tools": [["name": "openssl", "present": false], ["name": "curl", "present": true],
                                  ["name": "jq", "present": false], ["name": "tmux", "present": true]]
                            as [[String: Any]],
                        "missingForPush": ["openssl"],
                    ] as [String: Any],
                    "nothingWillPush": true,
                    "error": "That config file isn't valid JSON.",
                    "relayError": "iOS hasn't issued a push token to this phone yet — check that notifications are allowed, then try again.",
                    "hooksProof": ["state": "unavailable",
                                   "message": "Update the hooks for Claude Code first: the test runs what this version of Moshpit installs."],
                    "pushProof": ["state": "failed", "message": "The host reported: could not reach https://push.example.com"],
                 ]))),
            Shot(screen: .hostSetup, root: "hostsetup-host", state: "working",
                 message: hostSetup(with(["phase": "working",
                                          "phaseLabel": "Installing hooks for Claude Code…"]))),
            Shot(screen: .hostSetup, root: "hostsetup-host", state: "testing",
                 message: hostSetup(with(["phase": "proving", "pushProof": ["state": "proving"]]))),
        ]
    }

    /// Each redesigned page in each of its states (R15, R16, Settings).
    private static func shots() -> [Shot] {
        let whisper = """
            {"op":"languages","engine":"whisper","suggested":[{"id":"zh","name":"Chinese"},\
            {"id":"en","name":"English"}],"options":[{"id":"af","name":"Afrikaans"},\
            {"id":"zh","name":"Chinese"},{"id":"en","name":"English"},{"id":"fr","name":"French"},\
            {"id":"de","name":"German"},{"id":"ja","name":"Japanese"}],"selected":"zh"}
            """
        let apple = """
            {"op":"languages","engine":"apple","automatic":"English (United States)",\
            "options":[{"id":"en-US","name":"English (United States)"},{"id":"zh-CN","name":"Chinese (China)"},\
            {"id":"ja-JP","name":"Japanese (Japan)"}],"selected":""}
            """
        let log = """
            {"op":"log","state":"loaded","entries":[\
            {"time":"09:41:02.120","category":"health","message":"reconnect: path changed (wifi → cellular)"},\
            {"time":"09:41:01.250","category":"layout","message":"resize 80x24 → 70x40"},\
            {"time":"09:40:58.003","category":"push-extension","message":"decrypted a notification for %3 and handed it to the system; nothing else in it was read, the rest of the payload stays sealed until the app opens it"}]}
            """
        let feedback = { (preparing: Bool, showAddress: Bool) in
            FeedbackModel.message(text: "It froze when I rotated the iPad", environment: true, log: false,
                                  preparing: preparing, showAddress: showAddress, copied: false)
        }
        return [
            Shot(screen: .voiceEngine, root: "voice-engine", state: "setup",
                 json: #"{"op":"engine","engine":"whisper","needsModel":true}"#),
            Shot(screen: .voiceEngine, root: "voice-engine", state: "apple",
                 json: #"{"op":"engine","engine":"apple","needsModel":false}"#),
            Shot(screen: .voiceLanguage, root: "voice-language", state: "whisper", json: whisper),
            Shot(screen: .voiceLanguage, root: "voice-language", state: "apple", json: apple),
            Shot(screen: .voiceLanguage, root: "voice-language", state: "reading",
                 json: #"{"op":"languages","engine":"apple","automatic":"English","selected":""}"#),
            Shot(screen: .whisperModel, root: "whisper-model", state: "downloading",
                 message: WhisperModelPageModel.message(
                    options: seededOptions(), progress: ["small": 0.42],
                    failure: "There isn’t enough space on this iPhone.", selected: "big")),
            Shot(screen: .whisperModel, root: "whisper-model", state: "idle",
                 message: WhisperModelPageModel.message(
                    options: seededOptions(), progress: [:], failure: nil, selected: "big")),
            Shot(screen: .feedback, root: "feedback", state: "no-mail",
                 message: feedback(false, true)),
            Shot(screen: .feedback, root: "feedback", state: "mail", message: feedback(false, false)),
            Shot(screen: .feedback, root: "feedback", state: "preparing", message: feedback(true, false)),
            Shot(screen: .recentLog, root: "recent-log", state: "loaded", json: log),
            Shot(screen: .recentLog, root: "recent-log", state: "loading", json: #"{"op":"log","state":"loading"}"#),
            Shot(screen: .recentLog, root: "recent-log", state: "empty",
                 json: #"{"op":"log","state":"loaded","entries":[]}"#),
            Shot(screen: .recentLog, root: "recent-log", state: "failed",
                 json: #"{"op":"log","state":"failed","message":"The log store couldn’t be read."}"#),
        ] + hostSetupShots() + settingsShots()
    }

    /// The pictures for review, dark and light: `r16-<screen>-<state>-<dark|light>.png`
    /// in the app's tmp folder. The appearance is the window's, as the app
    /// sets it (R18), and the page follows it as it does on the phone.
    @Test("the redesigned voice, feedback, log, host setup and settings pages draw dark and light, in each state")
    func redesignShots() async throws {
        for style in [UIUserInterfaceStyle.dark, .light] {
            let look = style == .light ? "light" : "dark"
            for shot in Self.shots() {
                let name = "\(shot.screen.name) \(shot.state) \(look)"
                let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
                let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
                // The phone's size: a window as big as the iPad's scene in
                // landscape (744 pt tall) cut the pictures off at that
                // height, white under it (as R10's pictures once were).
                window.frame = NavBarNSHTests.phone
                // The app's appearance, where the app puts it (R18).
                window.overrideUserInterfaceStyle = style
                let root = UIViewController()
                window.rootViewController = root
                window.isHidden = false
                root.view.frame = window.bounds
                let surface = try #require(NSHSurface.make(frame: NavBarNSHTests.phone))
                root.view.addSubview(surface)
                defer {
                    surface.removeFromSuperview()
                    window.isHidden = true
                }
                // The system's appearance chosen, whatever this device keeps.
                var params = NSHScreenParams.current()
                params["appearance"] = "system"
                if let page = shot.page { params["page"] = page }
                let ran = surface.run(screen: shot.screen.name, params: params)
                #expect(ran, "\(name)")
                if let message = shot.message {
                    let posted = surface.post(message)
                    #expect(posted, "\(name)")
                }
                if let json = shot.json {
                    let posted = surface.post(json: json)
                    #expect(posted, "\(name)")
                }
                let shown = await SettingsNSHTests.view(identified: shot.root, in: surface)
                #expect(shown != nil, "\(name)")
                try? await Task.sleep(for: .milliseconds(700))
                Self.shoot(surface, "r16-\(shot.screen.name)-\(shot.state)-\(look)")
            }
        }
    }

    /// Draws the window where `surface` is to the app's tmp folder as
    /// `<name>.png`.
    private static func shoot(_ surface: UIView, _ name: String) {
        guard let window = surface.window else { return }
        let frame = surface.convert(surface.bounds, to: window)
        // What falls outside the window is not drawn: the picture would be
        // blank (white) there, not the page.
        #expect(window.bounds.contains(frame), "\(name): \(frame) outside \(window.bounds)")
        let image = UIGraphicsImageRenderer(bounds: frame).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(name).png")
        try? image.pngData()?.write(to: url)
    }

    // MARK: Helpers

    private static func first<V: UIView>(_ type: V.Type, in root: UIView) -> V? {
        if let hit = root as? V { return hit }
        for child in root.subviews {
            if let hit = first(type, in: child) { return hit }
        }
        return nil
    }

    private static func settle(_ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while !condition(), Date() < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(condition())
    }
}
