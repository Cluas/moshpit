import Foundation
import Testing
import UIKit
@testable import Moshpit

/// R7 on the device: a host's sessions page (`ui/src/sessions`,
/// docs/nsh/UI-DESIGN.md §6.2) runs on a real surface at the owner's
/// iPhone's size in both appearances, from a host as Home's snapshot posts
/// it: its session cards, its windows, a window of two panes folded out with
/// an agent waiting in one. Each appearance is drawn to the app's tmp folder
/// as `ui-gallery-<dark|light>-sessions.png`, beside the gallery's shots. (`ui/tests/sessions.rs` checks the page
/// headless: what it tells the host, its disclosure, its states.)
@Suite("NSH sessions page on the device")
@MainActor
struct SessionsNSHTests {
    static let host = "0F0E0D0C-0000-4000-8000-000000000001"

    /// A tmux host as Home's record carries its card (``HomeRecord``,
    /// `ui/src/home/model.rs`'s `Connection`): `work` (in use)
    /// with an editor and an agent's window of two panes, the agent waiting;
    /// `ops` with one shell.
    static var card: [String: Any] {
        [
            "id": host, "name": "devbox", "initial": "D", "user": "dev",
            "host": "devbox.example.com", "mosh": false, "udpPort": 60000,
            "link": "live", "transport": "live", "degraded": false, "roaming": false,
            "disconnecting": false,
            "control": [
                "mux": "tmux", "attached": true, "refreshing": false,
                "sessions": [["id": "$0", "name": "work"], ["id": "$1", "name": "ops"]],
                "windows": [
                    ["id": "@1", "session": "$0", "index": 1, "name": "api", "paneCount": 1],
                    ["id": "@2", "session": "$0", "index": 2, "name": "claude", "paneCount": 2],
                    ["id": "@3", "session": "$1", "index": 1, "name": "zsh", "paneCount": 1],
                ],
                "panes": [
                    ["id": "%1", "window": "@1", "index": 0, "command": "nvim", "path": "/Users/dev/src/api"],
                    ["id": "%2", "window": "@2", "index": 0, "command": "claude", "path": "/Users/dev/src/trainer"],
                    ["id": "%3", "window": "@2", "index": 1, "command": "zsh", "path": "/Users/dev/src/trainer"],
                    ["id": "%4", "window": "@3", "index": 0, "command": "zsh", "path": "/var/log"],
                ],
                "hooks": [["pane": "%2", "state": "attention", "agent": "claude"]],
                "worktrees": [] as [Any],
                "activeSession": "$0", "activeWindow": "@1", "activePane": "%1",
            ] as [String: Any],
        ]
    }

    @Test("the sessions page runs dark and light from a host as Home posts it")
    func sessions() async throws {
        for light in [false, true] {
            let look = light ? "light" : "dark"
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            let window = scene.map { UIWindow(windowScene: $0) }
                ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
            var frame = NavBarNSHTests.phone
            frame.size.height = min(frame.height, window.bounds.height)
            let surface = try #require(NSHSurface.make(frame: frame))
            HomeNSHComponents.register(on: surface)
            window.addSubview(surface)
            window.isHidden = false
            defer {
                surface.removeFromSuperview()
                window.isHidden = true
            }
            #expect(surface.run(screen: "sessions", params: UIGallerySurface.params(light: light)), "\(look)")
            surface.post(["op": "sessions", "connection": Self.card])
            // The agent's window folded out, as the host would give it back.
            surface.post(["op": "trees", "trees": [
                ["connection": Self.host, "collapsed": [] as [String], "expanded": ["@2"]],
            ]])
            for id in ["sessions", "sessions-new",
                       "home-session-\(Self.host)-$0", "home-session-\(Self.host)-$1",
                       "home-window-\(Self.host)-@1", "home-window-\(Self.host)-@2",
                       "home-pane-\(Self.host)-%2", "home-pane-\(Self.host)-%3"] {
                #expect(await SettingsNSHTests.view(identified: id, in: surface) != nil, "\(look): \(id)")
            }
            try? await Task.sleep(for: .milliseconds(600))
            UIGalleryNSHTests.shoot(surface, "\(look)-sessions")
        }
    }
}
