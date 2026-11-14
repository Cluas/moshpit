import Testing
import UIKit
@testable import Moshpit

/// What the NSH terminal's sheets and popovers say across the boundary
/// (W11h): the pickers' lists, a row menu's pick, the host's text prompt and
/// its answer, mosh's counters; and what the screens tell the host read back.
@MainActor
@Suite("Terminal sheets in NSH")
struct TerminalNSHSheetsTests {
    // MARK: The pickers

    static let rows: [MuxPickerState.Row] = [
        .init(id: "@1", index: 1, name: "agents", panes: 2, command: "zsh", signal: .attention),
        .init(id: "@2", index: 2, name: "logs", panes: 1, command: "tail"),
    ]

    @Test("the pickers run the Rust screen of their name")
    func pickerName() {
        #expect(NSHScreen.muxPicker.name == "mux-picker")
    }

    @Test("every list the host posts is a message the picker takes, in both multiplexers' words")
    func pickerCrossesTheBoundary() {
        let screen = NSHScreen.muxPicker.name
        for kind in [MuxPickerKind.windows, .sessions, .panes] {
            for mux in [Multiplexer.tmux, .herdr] {
                var rows = Self.rows
                rows[0].agent = "claude"
                rows[1].attached = true
                rows[1].windows = 3
                rows[1].window = "1:agents"
                let state = MuxPickerState(kind: kind, mux: mux, rows: rows, active: "@2")
                #expect(NSHSurface.check(screen: screen, message: state.message), "\(kind) \(mux)")
                #expect(state.message["mux"] as? String == (mux == .herdr ? "herdr" : "tmux"))
            }
        }
        let empty = MuxPickerState(kind: .panes, mux: .tmux, rows: [], active: nil)
        #expect(NSHSurface.check(screen: screen, message: empty.message))
        #expect(empty.message["active"] == nil)
        let json = (MuxPickerState(kind: .windows, mux: .tmux, rows: Self.rows, active: "@1")
            .message["rows"] as? [[String: Any]])?.first
        #expect(json?["signal"] as? String == "attention")
        #expect(json?["panes"] as? Int == 2)
    }

    @Test("a row menu's pick and the prompt's answer are the picker's messages")
    func pickerAnswers() {
        let screen = NSHScreen.muxPicker.name
        #expect(NSHSurface.check(screen: screen, message: ["op": "act", "action": "rename", "id": "@1"]))
        #expect(NSHSurface.check(screen: screen, message: ["op": "act", "action": "kill", "id": "$0"]))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "act", "action": "zap", "id": "@1"]))
        #expect(NSHSurface.check(screen: screen, message: NSHTextPrompt.answer(id: 3, text: "build")))
        #expect(NSHSurface.check(screen: screen, message: NSHTextPrompt.answer(id: 3, text: nil)))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "picker", "kind": "tabs"]))
    }

    @Test("what the picker tells the host is read back; anything else is nothing")
    func pickerEvents() {
        #expect(MuxPickerModel.event(["ev": "select", "id": "@2"]) == .select("@2"))
        #expect(MuxPickerModel.event(["ev": "new", "name": "build"]) == .new("build"))
        #expect(MuxPickerModel.event(["ev": "new", "name": NSNull()]) == .new(nil))
        #expect(MuxPickerModel.event(["ev": "new"]) == .new(nil))
        #expect(MuxPickerModel.event(["ev": "rename", "id": "@1", "name": "ops"]) == .rename("@1", "ops"))
        #expect(MuxPickerModel.event(["ev": "kill", "id": "@1"]) == .kill("@1"))
        let prompt: [String: Any] = ["ev": "prompt", "id": 4, "title": "New Window",
                                     "message": "Leave blank to let the program name it.",
                                     "placeholder": "Name (optional)", "text": "",
                                     "confirm": "Create", "cancel": "Cancel"]
        guard case .prompt(let question)? = MuxPickerModel.event(prompt) else {
            Issue.record("no prompt")
            return
        }
        #expect(question.id == 4)
        #expect(question.message == "Leave blank to let the program name it.")
        #expect(question.confirm == "Create")
        var bare = prompt
        bare["message"] = NSNull()
        if case .prompt(let q)? = MuxPickerModel.event(bare) { #expect(q.message == nil) }
        for bad: [String: Any] in [["ev": "select"], ["ev": "rename", "id": "@1"], ["ev": "prompt", "id": 1],
                                   ["ev": "fly"], [:]] {
            #expect(MuxPickerModel.event(bad) == nil, "\(bad)")
        }
    }

    @Test("windows and sessions rows carry Rename and the kill in the multiplexer's words; panes none")
    func pickerMenus() {
        var picked: [(String, String)] = []
        let windows = MuxPickerState(kind: .windows, mux: .herdr, rows: Self.rows, active: "@1")
        let menus = MuxPickerModel.menus(for: windows) { picked.append(($0, $1)) }
        #expect(Set(menus.keys) == ["mux-picker-row-@1", "mux-picker-row-@2"])
        let items = menus["mux-picker-row-@2"]?() ?? []
        let titles = items.compactMap { ($0 as? UIAction)?.title }
        #expect(titles == [String(localized: "Rename"),
                           String(localized: "\(Multiplexer.herdr.vocabulary.killVerb) \(Multiplexer.herdr.vocabulary.window)")])
        #expect((items.last as? UIAction)?.attributes.contains(.destructive) == true)
        let panes = MuxPickerState(kind: .panes, mux: .tmux, rows: Self.rows, active: "@1")
        #expect(MuxPickerModel.menus(for: panes) { _, _ in }.isEmpty)
        _ = picked
    }

    // MARK: The pill

    @Test("mosh's counters, shown or not, are the terminal's messages")
    func diagnosticsCrossTheBoundary() {
        let screen = NSHScreen.terminal.name
        #expect(NSHSurface.check(screen: screen, message: TerminalNSHDiagnostics().message))
        #expect(NSHSurface.check(screen: screen, message: TerminalNSHDiagnostics(shown: true).message))
        var counters = MoshDiagnostics()
        counters.datagramsReceived = 412
        counters.channelState = "ready"
        counters.inputHeld = true
        let shown = TerminalNSHDiagnostics(shown: true, counters: counters)
        #expect(NSHSurface.check(screen: screen, message: shown.message))
        #expect((shown.message["counters"] as? [String: Any])?["datagrams"] as? UInt64 == 412)
        // Hidden: no counters go.
        #expect(TerminalNSHDiagnostics(shown: false, counters: counters).message["counters"] == nil)
    }

    @Test("an action sheet on top of a surface is found; anything else is not")
    func dialogAnchorFindsTheSheet() {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 400, height: 400))
        let root = UIViewController()
        window.rootViewController = root
        let surface = UIView(frame: window.bounds)
        root.view.addSubview(surface)
        #expect(NSHDialogAnchor.sheet(presentedBy: surface) == nil)
    }

    // MARK: Install Assist and a host folder

    /// The core's message for a plan (`installAssistMessage`), as the sheet
    /// posts it.
    private static func install(manager: String?, rechecking: Bool = false,
                                result: InstallAssistPlan.Recheck? = nil) -> [String: Any] {
        let json = installAssistMessage(packages: ["mosh"], tmux: false, packageManager: manager,
                                        rechecking: rechecking, result: result?.rawValue, copied: false)
        return (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]) ?? [:]
    }

    @Test("Install Assist's message, the core's, is the screen's whatever the re-check found")
    func installCrossesTheBoundary() {
        let screen = NSHScreen.installAssist.name
        let brew = Self.install(manager: "brew")
        #expect(NSHSurface.check(screen: screen, message: brew))
        #expect(brew["command"] as? String == "brew install mosh")
        #expect(brew["moshOnly"] as? Bool == true)
        for result in [InstallAssistPlan.Recheck.resolved, .stillMissing, .noChannel] {
            let message = Self.install(manager: "brew", rechecking: result == .stillMissing, result: result)
            #expect(NSHSurface.check(screen: screen, message: message), "\(result)")
            #expect(message["result"] as? String == result.rawValue)
        }
        let none = Self.install(manager: nil)
        #expect(NSHSurface.check(screen: screen, message: none))
        #expect(none["command"] == nil, "no command, and no key for one")
        #expect(none["manager"] as? Bool == false)
        #expect(!NSHSurface.check(screen: screen, message: ["op": "install", "result": "maybe"]))
    }

    static func folder() -> RemoteFolderFacts {
        var facts = RemoteFolderFacts()
        facts.name = "work"
        facts.path = "/srv/work"
        facts.parents = ["/srv", "/"]
        facts.media = [.init(path: "/srv/work/a.png", name: "a.png", dir: false, video: false, symbol: "photo",
                             meta: "2 min. ago · 1 MB", thumb: true, duration: nil, opening: 0.3)]
        facts.rows = [.init(path: "/srv/work/logs", name: "logs", dir: true, video: false, symbol: "folder",
                            meta: "", thumb: false, duration: nil, opening: nil)]
        return facts
    }

    @Test("a host folder's facts, its menu's picks and the prompt's answers are the screen's messages")
    func folderCrossesTheBoundary() {
        let screen = NSHScreen.remoteFolder.name
        var facts = Self.folder()
        #expect(NSHSurface.check(screen: screen, message: facts.message))
        facts.uploading = 0.5
        facts.failure = "Permission denied"
        facts.openFailure = "gone"
        facts.editFailure = .init(title: "Couldn't rename", message: "taken")
        facts.pushed = true
        #expect(NSHSurface.check(screen: screen, message: facts.message))
        for action in ["open", "share", "rename", "delete"] {
            #expect(NSHSurface.check(screen: screen, message: ["op": "act", "action": action, "path": "/a"]))
        }
        #expect(NSHSurface.check(screen: screen, message: NSHTextPrompt.answer(id: 1, text: "x")))
        #expect(!NSHSurface.check(screen: screen, message: ["op": "folder", "colour": 1]))
    }

    /// The same file failing again after its alert was answered: the page
    /// posts only a snapshot that differs, and only when it next draws, so a
    /// cleared failure set again in the same words before that used to be a
    /// snapshot equal to the last one, never posted, and never said.
    @Test("a failure set anew gets a new number, so its snapshot is posted and said")
    func folderFailuresAreNumbered() {
        let model = RemoteFolderModel(
            folder: RemoteFileEntry(path: "/srv/work", isDirectory: true, size: nil, modified: nil),
            files: RemoteFileAccess(scope: "test", channel: { throw CancellationError() }))
        model.openFailure = "The file is gone."
        let first = RemoteFolderFacts(model: model, pushed: false)
        #expect(first.openFailureId != 0)
        model.openFailure = "The file is gone."
        #expect(RemoteFolderFacts(model: model, pushed: false) == first, "the same failure still standing is not new")
        // Answered (the screen's dismiss), and the same words again before
        // the page drew the cleared state.
        model.openFailure = nil
        model.openFailure = "The file is gone."
        let again = RemoteFolderFacts(model: model, pushed: false)
        #expect(again != first, "the same words failing again made the same snapshot: never posted")
        #expect(again.message["openFailureId"] as? Int == again.openFailureId)
        #expect(NSHSurface.check(screen: NSHScreen.remoteFolder.name, message: again.message))
        model.editFailure = .init(title: "Couldn't rename", message: "taken")
        let edit = RemoteFolderFacts(model: model, pushed: false)
        model.editFailure = nil
        model.editFailure = .init(title: "Couldn't rename", message: "taken")
        #expect(RemoteFolderFacts(model: model, pushed: false) != edit)
    }

    @Test("a file's menu opens, shares, renames and deletes; a folder's renames and deletes")
    func folderMenus() {
        var picked: [(String, String)] = []
        let menus = RemoteFolderNSHModel.menus(for: Self.folder()) { picked.append(($0, $1)) }
        #expect(Set(menus.keys) == ["remote-folder-item-/srv/work/a.png", "remote-folder-item-/srv/work/logs"])
        let file = (menus["remote-folder-item-/srv/work/a.png"]?() ?? []).compactMap { ($0 as? UIAction)?.title }
        #expect(file == [String(localized: "Open"), String(localized: "Share"), String(localized: "Rename"),
                         String(localized: "Delete")])
        let folder = (menus["remote-folder-item-/srv/work/logs"]?() ?? []).compactMap { ($0 as? UIAction)?.title }
        #expect(folder == [String(localized: "Rename"), String(localized: "Delete")])
        _ = picked
    }

    // MARK: On the device

    /// B302: a finger on a row or a tile of the folder lands in it. UIKit's
    /// own hit test at its centre (what a touch reaches first; the screen's
    /// presses are gesture recognizers on the views it lands in): the rows
    /// card's border, a view the size of the card in front of the rows, took
    /// every tap, and no row ever pressed on a device while the headless
    /// press passed.
    @Test("on a real surface: a touch on a row or a tile of the folder lands in it")
    func folderRowsTakeTouches() async throws {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        window.isHidden = false
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        window.addSubview(surface)
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        #expect(surface.run(screen: NSHScreen.remoteFolder.name, params: NSHScreenParams.current()))
        var facts = Self.folder()
        facts.media = facts.media.map { var m = $0; m.thumb = false; m.opening = nil; return m }
        facts.rows += [
            .init(path: "/srv/work/notes.md", name: "notes.md", dir: false, video: false, symbol: "doc.text",
                  meta: "1 hr. ago · 2 KB", thumb: false, duration: nil, opening: nil),
            .init(path: "/srv/work/src", name: "src", dir: true, video: false, symbol: "folder",
                  meta: "", thumb: false, duration: nil, opening: nil),
        ]
        #expect(surface.post(facts.message))
        for path in ["/srv/work/logs", "/srv/work/notes.md", "/srv/work/src", "/srv/work/a.png"] {
            let item = try #require(await SettingsNSHTests.view(identified: "remote-folder-item-\(path)", in: surface),
                                    "\(path) never showed")
            // Laid out where it shows.
            let deadline = Date().addingTimeInterval(3)
            while item.bounds.isEmpty, Date() < deadline { try? await Task.sleep(for: .milliseconds(20)) }
            let center = item.convert(CGPoint(x: item.bounds.midX, y: item.bounds.midY), to: window)
            let hit = try #require(window.hitTest(center, with: nil), "nothing at \(path)'s centre")
            #expect(hit.isDescendant(of: item), "a touch on \(path) lands on a \(type(of: hit)) outside it")
        }
        // And everything else the folder presses (the bar, the + menu).
        await NSHTouchAudit.pressablesTakeTouches(in: surface)

    }

    /// A surface of `screen` in a window of the test host's scene.
    private static func surface(running screen: NSHScreen) throws -> (NSHSurface, () -> Void) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        window.addSubview(surface)
        window.isHidden = false
        #expect(surface.run(screen: screen.name, params: NSHScreenParams.current()))
        return (surface, {
            surface.removeFromSuperview()
            window.isHidden = true
        })
    }

    @Test("the pickers on a real surface: a touch on a row lands in it")
    func pickerSurface() async throws {
        for kind in [MuxPickerKind.windows, .sessions, .panes] {
            let (surface, done) = try Self.surface(running: .muxPicker)
            defer { done() }
            #expect(surface.post(MuxPickerState(kind: kind, mux: .tmux, rows: Self.rows, active: "@1").message))
            await NSHTouchAudit.pressablesTakeTouches(in: surface)

        }
    }

    @Test("Install Assist on a real surface: a touch on a button lands in it")
    func installSurface() async throws {
        let (surface, done) = try Self.surface(running: .installAssist)
        defer { done() }
        #expect(surface.post(Self.install(manager: "brew")))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)

    }

    // MARK: Pictures (R12's R18 part)

    /// Install Assist on the tokens (R12's R18 part), dark and light, drawn
    /// to the app's tmp folder as `ui-gallery-r12-install-<state>-<dark|light>.png`:
    /// the command with its actions, and a re-check that found it.
    @Test("Install Assist drawn dark and light at the iPhone's size")
    func installPictures() async throws {
        let states: [(String, [String: Any])] = [
            ("command", Self.install(manager: "brew")),
            ("resolved", Self.install(manager: "brew", result: .resolved)),
        ]
        for (name, message) in states {
            for light in [false, true] {
                let look = light ? "light" : "dark"
                try await ThemeNSHTests.picture(.installAssist, message, ready: "install-assist", light: light,
                                                name: "r12-install-\(name)-\(look)")
            }
        }
    }

    // MARK: Pictures (R13)

    /// R13's pages at the owner's iPhone's size, dark and light, drawn to
    /// the app's tmp folder as `ui-gallery-r13-<page>-<dark|light>.png`: a
    /// folder with a tile and rows, an empty one, the first listing's
    /// skeleton, and the windows picker.
    @Test("R13's pages drawn dark and light at the iPhone's size")
    func pictures() async throws {
        var folder = Self.folder()
        folder.media = folder.media.map { var m = $0; m.thumb = false; m.opening = nil; return m }
        folder.rows += [
            .init(path: "/srv/work/notes.md", name: "notes.md", dir: false, video: false, symbol: "doc.text",
                  meta: "1 hr. ago · 2 KB", thumb: false, duration: nil, opening: nil),
        ]
        var empty = Self.folder()
        empty.media = []
        empty.rows = []
        var loading = empty
        loading.loading = true
        let pages: [(NSHScreen, String, [String: Any], String)] = [
            (.remoteFolder, "folder", folder.message, "remote-folder-item-/srv/work/logs"),
            (.remoteFolder, "folder-empty", empty.message, "remote-folder-empty"),
            (.remoteFolder, "folder-loading", loading.message, "remote-folder-skeleton"),
            (.muxPicker, "picker", MuxPickerState(kind: .windows, mux: .tmux, rows: Self.rows, active: "@1").message,
             "mux-picker-row-@1"),
        ]
        for (screen, name, message, ready) in pages {
            for light in [false, true] {
                let look = light ? "light" : "dark"
                try await ThemeNSHTests.picture(screen, message, ready: ready, light: light,
                                                name: "r13-\(name)-\(look)")
            }
        }
    }
}
