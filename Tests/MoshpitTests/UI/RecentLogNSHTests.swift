import Foundation
import Testing
import UIKit
@testable import Moshpit

/// Settings › Recent Log in NSH: the data path between the host and the
/// screen — what the Swift side posts is what the Rust screen
/// takes (`ui/src/recent_log.rs`'s tests and `ui/tests/recent_log.rs` cover
/// the screen itself), and a real surface on the device runs the screen,
/// shows what is posted and tells the host about the back button.
@Suite("NSH Recent Log")
struct RecentLogNSHTests {

    private static let entry = DiagnosticsLog.Entry(
        date: Date(timeIntervalSince1970: 1_731_300_000.25),
        category: "layout",
        message: "resize 80x24 → 70x40")

    @Test("this build hosts NSH screens")
    func nshIsLinked() {
        #expect(NSHSurface.available)
    }

    @Test("every state the host posts is a message the Rust screen takes")
    func messagesCrossTheBoundary() {
        let states: [RecentLogModel.State] = [
            .loading,
            .failed("Couldn't read the log: The operation couldn’t be completed."),
            .loaded([]),
            .loaded([Self.entry, Self.entry]),
        ]
        for state in states {
            let message = RecentLogModel.message(for: state)
            #expect(NSHSurface.check(screen: NSHScreen.recentLog.name, message: message), "\(message)")
        }
    }

    @Test("an entry carries its time as the SwiftUI page shows it, and Copy's line is DiagnosticsLog.line")
    func entryShape() throws {
        let entry = RecentLogModel.entry(Self.entry)
        let time = try #require(entry["time"] as? String)
        #expect(time == DiagnosticsLog.clock.string(from: Self.entry.date))
        #expect(entry["category"] as? String == "layout")
        #expect(entry["message"] as? String == "resize 80x24 → 70x40")
        // The screen's Copy writes `time  category  message` per entry
        // (`recent_log::line`): the SwiftUI page's line.
        #expect(DiagnosticsLog.line(Self.entry) == "\(time)  layout  resize 80x24 → 70x40")
    }

    @Test("a screen is told the app's language and the theme's colors")
    @MainActor func params() throws {
        let params = NSHScreenParams.current()
        #expect(params["locale"] as? String == Bundle.main.preferredLocalizations.first)
        #expect(params["bar"] == nil, "no bar measured, none passed")
        let measured = try #require(NSHScreenParams.current(barHeight: 56)["bar"] as? [String: CGFloat])
        #expect(measured == ["height": 56])
        let theme = try #require(params["theme"] as? [String: String])
        #expect(Set(theme.keys) == ["accent", "accentPressed", "screenBG", "terminalBG"])
        for value in theme.values {
            #expect(value.count == 7 && value.hasPrefix("#"), "\(value)")
        }
        #expect(theme["accent"] == "#" + AppThemeCatalog.current.accentHex)
        #expect(NSHScreenParams.hex(AppThemeCatalog.signalRoom.screenBG) == "#06070A")
    }

    @Test("a surface on the device runs the screen, shows what is posted and hears the back button")
    @MainActor func surfaceRoundTrip() async throws {
        // A window of the test host's scene, as a page's would be.
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        let surface = try #require(NSHSurface.make(frame: window.bounds))
        window.addSubview(surface)
        window.isHidden = false
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        var heard: [[String: Any]] = []
        surface.onMessage = { heard.append($0) }

        #expect(!surface.run(screen: "no-such-screen", params: [:]))
        #expect(!surface.post(RecentLogModel.message(for: .loading)), "no screen runs yet")
        #expect(surface.run(screen: NSHScreen.recentLog.name, params: NSHScreenParams.current()))
        #expect(surface.post(RecentLogModel.message(for: .loading)))
        #expect(!surface.post(["op": "focus"]))
        #expect(await Self.view(identified: "recent-log-loading", in: surface) != nil)

        #expect(surface.post(RecentLogModel.message(for: .loaded([Self.entry]))))
        #expect(await Self.view(identified: "recent-log-entry", in: surface) != nil)
        #expect(await Self.view(identified: "recent-log-copy", in: surface) != nil)
        await NSHTouchAudit.pressablesTakeTouches(in: surface)

        let back = try #require(await Self.view(identified: "nav-bar-back", in: surface))
        #expect(back.accessibilityActivate())
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while heard.isEmpty, Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(heard.first?["ev"] as? String == "back")
    }

    /// The view under `root` with accessibility identifier `id`, waiting up
    /// to five seconds for the screen to show it.
    @MainActor
    private static func view(identified id: String, in root: UIView) async -> UIView? {
        func find(_ view: UIView) -> UIView? {
            if view.accessibilityIdentifier == id { return view }
            for child in view.subviews {
                if let hit = find(child) { return hit }
            }
            return nil
        }
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while Date() < deadline {
            if let hit = find(root) { return hit }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return nil
    }
}
