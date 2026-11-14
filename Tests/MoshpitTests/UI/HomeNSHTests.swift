import Foundation
import Observation
import Testing
import UIKit
@testable import Moshpit

/// Home in NSH: its switches (it shows only with sessions on the core,
/// where its record is read), the data path between the host and the
/// screen — the page asks the core for Home's record (`app::home`, whose
/// tests and `crates/app/harness/home`'s cross-check hold what it says)
/// with every saved connection and its session on the core, reads it again
/// after every batch of the sessions' events, and posts it as the core
/// wrote it; what the screen tells the host is an intent or the trees'
/// disclosure the host keeps — and a real surface on the device that runs
/// the screen, shows the cards and their trees, and reports what the user
/// does.
@Suite("NSH Home")
@MainActor
struct HomeNSHTests {

    static let studio = UUID(uuidString: "C0FFEE00-0000-4000-8000-00000000F001")!
    static let edge = UUID(uuidString: "C0FFEE00-0000-4000-8000-00000000F004")!

    /// A live tmux card with a two-session tree and an agent waiting, and a
    /// saved mosh host: Home's record as the core writes it.
    static func message() -> [String: Any] {
        ["op": "home", "version": "v1.0.9 (1)", "sidebar": false, "connections": [
            ["id": studio.uuidString, "name": "studio", "initial": "S", "user": "cluas",
             "host": "192.0.2.22", "mosh": false, "udpPort": 60001, "link": "live", "transport": "live",
             "degraded": false, "roaming": false, "disconnecting": false,
             // Three hours ago, as a device shot reads it (not "498498h up").
             "connectedAt": Date(timeIntervalSinceNow: -(3 * 3600 + 25 * 60)).timeIntervalSince1970,
             "control": [
                "mux": "tmux", "attached": true, "refreshing": false,
                "sessions": [["id": "$0", "name": "infra"], ["id": "$1", "name": "moshpit"]],
                "windows": [["id": "@0", "session": "$0", "index": 1, "name": "k3s", "paneCount": 1],
                            ["id": "@2", "session": "$1", "index": 1, "name": "agents", "paneCount": 2]],
                "panes": [["id": "%0", "window": "@0", "index": 0, "command": "codex"],
                          ["id": "%2", "window": "@2", "index": 0, "command": "2.1.269", "path": "/srv/my app"],
                          ["id": "%3", "window": "@2", "index": 1, "command": "zsh"]],
                "activeSession": "$1", "activeWindow": "@2", "activePane": "%2",
                "hooks": [["pane": "%2", "state": "attention", "agent": "claude", "since": 1_000.0]],
                "worktrees": [] as [Any],
             ] as [String: Any]] as [String: Any],
            ["id": edge.uuidString, "name": "edge", "initial": "E", "user": "root",
             "host": "edge.example.com", "mosh": true, "udpPort": 60001, "link": "saved",
             "transport": "live", "degraded": false, "roaming": false, "disconnecting": false] as [String: Any],
        ] as [Any]]
    }

    static func json(_ message: [String: Any]) -> String {
        let data = try! JSONSerialization.data(withJSONObject: message, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }

    /// The studio's tree as its controller has it.
    static func tree() -> TmuxSnapshot {
        var tmux = TmuxSnapshot()
        tmux.sessions = ["$0": SessionInfo(id: "$0", name: "infra"),
                         "$1": SessionInfo(id: "$1", name: "moshpit", isAttached: true)]
        tmux.windows = [
            "@0": WindowInfo(id: "@0", sessionId: "$0", name: "k3s", index: 1),
            "@2": WindowInfo(id: "@2", sessionId: "$1", name: "agents", index: 1, paneCount: 2),
        ]
        tmux.panes = [
            "%0": PaneInfo(id: "%0", windowId: "@0", index: 0, command: "codex"),
            "%2": PaneInfo(id: "%2", windowId: "@2", index: 0, command: "2.1.269"),
            "%3": PaneInfo(id: "%3", windowId: "@2", index: 1, command: "zsh"),
        ]
        tmux.activeSessionId = "$1"
        tmux.activeWindowId = "@2"
        tmux.activePaneId = "%2"
        tmux.isAttached = true
        tmux.everAttached = true
        return tmux
    }

    /// The record's cards as the menus take them.
    static func rows() -> [HomeNSHMenus.Row] {
        [HomeNSHMenus.Row(id: studio, link: .live, tree: HomeNSHMenus.Tree(mux: .tmux, snapshot: tree())),
         HomeNSHMenus.Row(id: edge, link: .saved, tree: nil)]
    }

    @Test("Home has its own switch, off unless set")
    func switches() {
        #expect(NSHScreen.home.switchKey == "MOSHPIT_NSH_HOME")
        #expect(NSHScreen.home.name == "home")
        let registered = UserDefaults.standard.volatileDomain(forName: UserDefaults.registrationDomain)
        #expect(registered[NSHScreen.home.switchKey] == nil)
        #expect(Set(NSHScreen.allCases.map(\.switchKey)).count == NSHScreen.allCases.count)
        #expect(NSHScreen.chosen(switchOn: false, available: { true }) == false)
    }

    @Test("NSH Home shows only with sessions on the core, where its record is read")
    func needsSessionsOnTheCore() {
        #expect(HomeNSHPage.shows(nsh: true, rustSessions: true))
        #expect(!HomeNSHPage.shows(nsh: true, rustSessions: false),
                "sessions on the Swift path: the SwiftUI list, whose cards read them")
        #expect(!HomeNSHPage.shows(nsh: false, rustSessions: true))
        #expect(!HomeNSHPage.shows(nsh: false, rustSessions: false))
    }

    /// Two saved connections, the studio with a session on the core.
    static func store() -> [ServerConnection] {
        var studio = ServerConnection(host: "192.0.2.22", username: "cluas")
        studio.id = Self.studio
        studio.name = "studio"
        studio.multiplexer = .tmux
        var edge = ServerConnection(host: "edge.example.com", username: "root")
        edge.id = Self.edge
        edge.name = "edge"
        edge.connectionProtocol = .mosh
        edge.moshPortRangeStart = 60100
        return [studio, edge]
    }

    @Test("the record is the core's: every saved connection with its session on the core, posted as written")
    func record() throws {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        fake.home = Self.json(Self.message())
        let record = HomeRecord.now(core: core, connections: Self.store(),
                                    session: { $0.id == Self.studio ? 7 : nil }, agents: nil,
                                    disconnecting: [Self.edge], sidebar: true, udpPort: 60200)
        #expect(record.message == fake.home, "as the core wrote it")
        let asked = try #require(fake.homeAsked.last)
        #expect(asked.connections.map(\.id) == [Self.studio.uuidString, Self.edge.uuidString])
        #expect(asked.connections.map(\.session) == [7, nil])
        #expect(asked.connections.map(\.name) == ["studio", "edge"])
        #expect(asked.connections.map(\.tmux) == [true, false])
        #expect(asked.connections.map(\.mosh) == [false, true])
        // Settings' first port on every card, the one a connect asks for,
        // not the 60100 the edge saved (B430).
        #expect(asked.connections.map(\.udpPort) == [60200, 60200])
        #expect(asked.connections.map(\.disconnecting) == [false, true])
        #expect(asked.sidebar == true)
        #expect(asked.version == HomeFooter.line)
        #expect(asked.locale == NSHScreenParams.locale)
        #expect(record.cards == [HomeRecord.Card(id: Self.studio, link: .live),
                                 HomeRecord.Card(id: Self.edge, link: .saved)])
        #expect(HomeRecord(message: "not json").cards.isEmpty)
    }

    @Test("Home is read again after every batch of the sessions' events, not for the registry's own")
    func readAgain() {
        let fake = FakeCoreSessions()
        let core = RustCore(sessions: fake, credentials: RustCredentialHost())
        final class Changes: @unchecked Sendable { var count = 0 }
        let changed = Changes()
        func watch() {
            withObservationTracking {
                _ = HomeRecord.now(core: core, connections: Self.store(), session: { _ in nil },
                                   agents: nil, disconnecting: [], sidebar: false)
            } onChange: { changed.count += 1 }
        }
        watch()
        core.deliver([SessionEvent(session: 0, kind: .flushed(token: 1))])
        #expect(changed.count == 0, "a flush changes no session")
        core.deliver([SessionEvent(session: 7, kind: .treeChanged),
                      SessionEvent(session: 7, kind: .agentHooksUpdated)])
        #expect(changed.count == 1)
        watch()
        core.deliver([SessionEvent(session: 9, kind: .attachStalled)])
        #expect(changed.count == 2, "a session no page listens to is still Home's")
    }

    @Test("Home's messages are the screen's; bad ones are refused")
    func messages() throws {
        let screen = NSHScreen.home.name
        let message = Self.message()
        #expect(NSHSurface.check(screen: screen, message: message), "\(message)")
        #expect(NSHSurface.check(screen: screen, json: Self.json(message)))
        var sidebar = message
        sidebar["sidebar"] = true
        #expect(NSHSurface.check(screen: screen, message: sidebar), "the iPad's sidebar")
        var bad = message
        bad["connections"] = [["id": "x", "name": "x", "initial": "X", "user": "u", "host": "h",
                               "mosh": false, "udpPort": 1, "link": "asleep"]]
        #expect(!NSHSurface.check(screen: screen, message: bad))
        bad = message
        bad.removeValue(forKey: "version")
        #expect(!NSHSurface.check(screen: screen, message: bad))
        #expect(!NSHSurface.check(screen: screen, json: "{\"op\":\"home\""))
        let model = HomeNSHModel()
        _ = model.handle(["ev": "tree", "connection": Self.studio.uuidString,
                          "collapsed": ["$0"], "expanded": ["@2"]])
        #expect(NSHSurface.check(screen: screen, message: model.treesMessage))
    }

    @Test("what the screen tells the host: an intent, or the trees' disclosure it keeps")
    func intents() {
        let id = Self.studio.uuidString
        #expect(HomeIntent(["ev": "settings"]) == .settings)
        #expect(HomeIntent(["ev": "add"]) == .add)
        #expect(HomeIntent(["ev": "toggle-sidebar"]) == .toggleSidebar)
        #expect(HomeIntent(["ev": "enter", "connection": id]) == .enter(Self.studio))
        #expect(HomeIntent(["ev": "sessions", "connection": id]) == .sessions(Self.studio))
        #expect(HomeIntent(["ev": "connect", "connection": id]) == .connect(Self.studio))
        #expect(HomeIntent(["ev": "window", "connection": id, "window": "@2"]) == .window(Self.studio, "@2"))
        #expect(HomeIntent(["ev": "pane", "connection": id, "pane": "%3"]) == .pane(Self.studio, "%3"))
        #expect(HomeIntent(["ev": "kill-session", "connection": id, "session": "$1"])
                == .killSession(Self.studio, "$1"))
        #expect(HomeIntent(["ev": "new-pane", "connection": id, "window": "@2"]) == .newPane(Self.studio, "@2"))
        #expect(HomeIntent(["ev": "kill-pane", "connection": id, "pane": "%3"]) == .killPane(Self.studio, "%3"))
        #expect(HomeIntent(["ev": "sessions", "connection": id]) == .sessions(Self.studio))
        #expect(HomeIntent(["ev": "window", "connection": id]) == nil, "a window intent names its window")
        #expect(HomeIntent(["ev": "enter", "connection": "not-a-uuid"]) == nil)
        #expect(HomeIntent(["ev": "jump", "connection": id]) == nil)

        let model = HomeNSHModel()
        #expect(model.handle(["ev": "tree", "connection": id, "collapsed": ["$0"], "expanded": [],
                              "idle": true]) == nil)
        #expect(model.trees[id]?.collapsed == ["$0"])
        #expect(model.trees[id]?.idle == true, "Home's idle group, kept for a page made again")
        // Home sends only its idle group, the sessions page only its
        // sessions and windows: neither clears the other's.
        #expect(model.handle(["ev": "tree", "connection": id, "idle": false]) == nil)
        #expect(model.trees[id]?.collapsed == ["$0"], "Home's message leaves the sessions alone")
        #expect(model.handle(["ev": "tree", "connection": id, "collapsed": [], "expanded": ["@2"]]) == nil)
        #expect(model.trees[id]?.idle == false && model.trees[id]?.expanded == ["@2"])
        #expect(model.handle(["ev": "tree", "connection": id, "idle": true]) == nil)
        #expect(model.handle(["ev": "refresh", "connection": id]) == .refresh(Self.studio))
        let trees = model.treesMessage["trees"] as? [[String: Any]]
        #expect(trees?.first?["connection"] as? String == id)
        #expect(trees?.first?["idle"] as? Bool == true)
    }

    @Test("the name cards refuse what the multiplexer would not get as typed")
    func cards() {
        #expect(HomeNSHActions.refusal("ok") == nil)
        #expect(HomeNSHActions.refusal("a\nb") != nil)
        let rename = HomeNSHActions.rename(title: "Rename Session", currentName: "work") { _ in }
        #expect(rename.initialText == "work")
        var created: String?? = .none
        let create = HomeNSHActions.create(noun: "Window") { created = $0 }
        if case .input(let input) = create.kind {
            input.apply("   ")
            #expect(created == .some(nil), "blank lets the multiplexer name it")
        } else {
            Issue.record("a name card")
        }
    }

    @Test("the rows' long-press menus carry the SwiftUI card's actions, in its words")
    func menus() {
        let menus = HomeNSHMenus.menus(for: Self.rows()) { _ in }
        let id = Self.studio.uuidString
        let titles = { (key: String) -> [String] in
            (menus[key]?() ?? []).flatMap { ($0 as? UIMenu)?.children ?? [$0] }.map(\.title)
        }
        let tmux = Multiplexer.tmux.vocabulary
        #expect(titles("home-head-\(id)") == [String(localized: "Open"), String(localized: "Edit…"),
                                              String(localized: "Disconnect"),
                                              String(localized: "Delete…")])
        #expect(titles("home-head-\(Self.edge.uuidString)") == [String(localized: "Connect"),
                                                                String(localized: "Edit…"),
                                                                String(localized: "Delete…")])
        #expect(titles("home-session-\(id)-$1") == [
            String(localized: "Rename"), String(localized: "New \(tmux.window)"),
            String(localized: "\(tmux.killVerb) \(tmux.session)"),
        ])
        #expect(titles("home-window-\(id)-@2") == [
            String(localized: "Rename"), String(localized: "New Pane"),
            String(localized: "\(tmux.killVerb) \(tmux.window)"),
            String(localized: "\(tmux.killVerb) \(String(localized: "Pane"))"),
        ])
        #expect(titles("home-pane-\(id)-%3") == [String(localized: "\(tmux.killVerb) \(String(localized: "Pane"))")])
        #expect(menus["home-session-\(Self.edge.uuidString)-$1"] == nil, "a saved host has no tree")
        #expect(HomeNSHMenus.corners(for: Self.rows())["home-window-\(id)-@2"] == 7)
        var herdr = Self.rows()[0]
        herdr.tree = HomeNSHMenus.Tree(mux: .herdr, snapshot: Self.tree(), worktrees: ["$1"])
        let worktree = HomeNSHMenus.menus(for: [herdr]) { _ in }
        // Built outside #expect: the macro's expansion can't take an
        // optional call (`?()`) inside a coalesced subexpression.
        let inWorktree = (worktree["home-session-\(id)-$1"]?() ?? []).map(\.title)
        let notWorktree = (worktree["home-session-\(id)-$0"]?() ?? []).map(\.title)
        #expect(inWorktree.contains(String(localized: "Remove Worktree")), "a worktree's workspace")
        #expect(!notWorktree.contains(String(localized: "Remove Worktree")))
        var dead = Self.rows()[0]
        dead.link = .dead
        #expect(HomeNSHMenus.menus(for: [dead]) { _ in }["home-session-\(id)-$1"] == nil,
                "a dead card's tree has no menus")
    }

    // MARK: On the device

    /// A surface in a window of the test host's scene, running Home with
    /// its own components registered.
    static func surface() throws -> (NSHSurface, SettingsNSHTests.Heard, () -> Void) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        HomeNSHComponents.register(on: surface)
        window.addSubview(surface)
        window.isHidden = false
        let heard = SettingsNSHTests.Heard()
        surface.onMessage = { heard.append($0) }
        #expect(surface.run(screen: NSHScreen.home.name, params: NSHScreenParams.current()))
        return (surface, heard, {
            surface.removeFromSuperview()
            window.isHidden = true
        })
    }

    @Test("on a real surface: the cards and their tree, the host's menu, and what the user does reaches the host")
    func surface() async throws {
        let (surface, heard, done) = try Self.surface()
        defer { done() }
        #expect(surface.post(json: Self.json(Self.message())))
        let head = try #require(await SettingsNSHTests.view(identified: "connection-card-studio", in: surface))
        await NSHTouchAudit.pressablesTakeTouches(in: surface)
        #expect(head.accessibilityActivate())
        let enter = try #require(await heard.next())
        #expect(enter["ev"] as? String == "enter")
        #expect(enter["connection"] as? String == Self.studio.uuidString)

        let saved = try #require(await SettingsNSHTests.view(identified: "connection-card-edge", in: surface))
        #expect(saved.accessibilityActivate())
        #expect(try #require(await heard.next())["ev"] as? String == "connect")

        // The waiting agent's row, in NEEDS YOU (R6): its pane.
        let agent = try #require(await SettingsNSHTests.view(
            identified: "home-pane-\(Self.studio.uuidString)-%2", in: surface))
        #expect(agent.accessibilityActivate())
        let opened = try #require(await heard.next())
        #expect(opened["ev"] as? String == "pane")
        #expect(opened["pane"] as? String == "%2")

        // The ⋯ menu is UIKit's, with the head's actions (UI-DESIGN §6.1).
        let menu = try #require(await SettingsNSHTests.view(identified: "card-menu-studio", in: surface)
            as? UIButton)
        #expect(menu.showsMenuAsPrimaryAction)
        let titles = menu.menu?.children.compactMap { $0 as? UIMenu }.flatMap(\.children).map(\.title)
        #expect(titles == [String(localized: "Open"), String(localized: "Edit…"),
                           String(localized: "Disconnect"), String(localized: "Delete…")])
        #expect(menu.accessibilityLabel == String(localized: "More"))

        // The stage's grid is the host's.
        #expect(surface.post(json: #"{"op":"home","connections":[],"version":"v1.0.9 (1)"}"#))
        #expect(await SettingsNSHTests.view(identified: "home-add-empty", in: surface) != nil)
    }

    /// R5's acceptance fixture as Home's message (`ui/tests/home.rs` writes
    /// it), its clocks moved from the fixture's 4000 to now: the waiting
    /// agent 50 minutes in, build-box up three hours, edge lost 25 minutes
    /// ago.
    static func acceptance() throws -> [String: Any] {
        let data = try TestFixtures.data("home-acceptance.json")
        var message = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let shift = Date().timeIntervalSince1970 - 4000
        let moved = { (value: Any?) in (value as? Double).map { $0 + shift } }
        var cards = message["connections"] as? [[String: Any]] ?? []
        for i in cards.indices {
            for key in ["connectedAt", "lostAt"] { cards[i][key] = moved(cards[i][key]) ?? cards[i][key] }
            guard var control = cards[i]["control"] as? [String: Any] else { continue }
            var hooks = control["hooks"] as? [[String: Any]] ?? []
            for h in hooks.indices { hooks[h]["since"] = moved(hooks[h]["since"]) ?? hooks[h]["since"] }
            control["hooks"] = hooks
            cards[i]["control"] = control
        }
        message["connections"] = cards
        return message
    }

    /// Every label under `root`.
    static func labels(in root: UIView) -> [String] {
        func all(_ v: UIView) -> [UIView] { v.subviews.flatMap { [$0] + all($0) } }
        return all(root).compactMap { ($0 as? UILabel)?.text ?? $0.accessibilityLabel }
    }

    @Test("R6 on the owner's iPhone: the acceptance fixture, dark and light, Large and AX3, drawn")
    func redesign() async throws {
        let message = try Self.acceptance()
        for (look, style) in [("dark", UIUserInterfaceStyle.dark), ("light", .light)] {
            for (size, scale) in [("large", nil), ("ax3", 40.0 / 17.0)] as [(String, Double?)] {
                let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
                let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: NavBarNSHTests.phone)
                // The phone's size (the iPad's scene in landscape is 744 pt
                // tall), and the app's appearance where the app puts it (R18).
                window.frame = NavBarNSHTests.phone
                window.overrideUserInterfaceStyle = style
                let surface = try #require(NSHSurface.make(frame: NavBarNSHTests.phone))
                HomeNSHComponents.register(on: surface)
                window.addSubview(surface)
                window.isHidden = false
                defer {
                    surface.removeFromSuperview()
                    window.isHidden = true
                }
                // The system's appearance chosen: the page follows the window.
                var params = NSHScreenParams.current()
                params["appearance"] = "system"
                if let scale { params["fontScale"] = scale }
                #expect(surface.run(screen: NSHScreen.home.name, params: params), "\(look) \(size)")
                #expect(surface.post(message), "\(look) \(size): the fixture is Home's message")
                #expect(await SettingsNSHTests.view(identified: "home-needs-you", in: surface) != nil)
                try? await Task.sleep(for: .milliseconds(600))
                let name = "home-r6-\(look)-\(size)"
                NavBarNSHTests.shoot(surface, "\(name)-top")
                let list = try #require(NavBarNSHTests.scrollView(in: surface))
                // Two screens at most (§8 R6) at the default size.
                if scale == nil {
                    #expect(list.contentSize.height <= 2 * NavBarNSHTests.phone.height,
                            "\(look): \(list.contentSize.height) pt")
                }
                await NavBarNSHTests.scroll(list, to: NavBarNSHTests.phone.height - 200)
                NavBarNSHTests.shoot(surface, "\(name)-more")
                await NavBarNSHTests.scroll(list, to: max(0, list.contentSize.height - NavBarNSHTests.phone.height))
                NavBarNSHTests.shoot(surface, "\(name)-end")
                // No version, no tagline on a Home with hosts (§6.1).
                let words = Self.labels(in: surface)
                #expect(!words.contains { $0.contains("v1.0.9") }, "\(look) \(size): a version")
                #expect(!words.contains { $0.localizedCaseInsensitiveContains("pit never closes") })
                // ⋯ on each card's head, three cards.
                for name in ["gpu-01", "build-box", "edge"] {
                    #expect(await SettingsNSHTests.view(identified: "card-menu-\(name)", in: surface) != nil,
                            "\(name)'s ⋯")
                }
            }
        }
    }
}
