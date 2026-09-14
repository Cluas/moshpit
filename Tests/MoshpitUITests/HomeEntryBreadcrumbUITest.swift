import XCTest

/// Rig-only regression test for the build 1.0.4 (4) top bar going deaf on iOS
/// 27: enter the terminal the way a person does — connect from the home card,
/// then enter (card again, or a session-tree row) — and tap the window crumb
/// straight away. The Windows sheet must open.
///
/// The shipped code showed the connecting poster for the one frame before
/// `active` resolved and dropped it in the very update that hosted the pane;
/// on iOS 27 that left every SwiftUI control in the top bar ignoring taps
/// (the UIKit terminal underneath still worked). The seeded direct-to-terminal
/// launch never hit it, which is why the suites stayed green.
///
/// Needs the local rig (`/tmp/rc`: relay on 127.0.0.1:2222, isolated tmux
/// wrapper, seed key) — skips everywhere else. Run on an iOS 27 simulator to
/// reproduce the original failure against 0943c03.
final class HomeEntryBreadcrumbUITest: XCTestCase {
    private static let rigKey = "/tmp/rc/seedkey.b64"

    private func shot(_ app: XCUIApplication, _ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot())
        a.name = name; a.lifetime = .keepAlways; add(a)
    }

    @MainActor
    private func launchOnHome() throws -> XCUIApplication {
        guard let key = try? String(contentsOfFile: Self.rigKey, encoding: .utf8) else {
            throw XCTSkip("needs the local reconnect rig (/tmp/rc)")
        }
        let app = XCUIApplication()
        app.launchArguments += [
            "-MOSHPIT_RESET", "-MOSHPIT_SEED_HOME", "1",
            "-MOSHPIT_SEED_USER", NSUserName(),
            "-MOSHPIT_SEED_KEY_B64", key.trimmingCharacters(in: .whitespacesAndNewlines),
            "-MOSHPIT_SEED_HOST", "127.0.0.1", "-MOSHPIT_SEED_PORT", "2222",
            "-MOSHPIT_SEED_NAME", "rc-lab", "-MOSHPIT_SEED_MUX", "tmux",
            "-MOSHPIT_SEED_TMUX_BIN", "/tmp/rc/tmux", "-MOSHPIT_SEED_QUIET", "1",
            "-MOSHPIT_SEED_TRUST_FP", "SHA256:qGbLaODy9ZRyfBIDjh6TVeU+Il5Nt+iqk37vKss29Ks",
        ]
        app.launch()
        return app
    }

    /// The breadcrumb's window crumb reads "1:alpha" (no space); the home
    /// session-tree row reads "1: alpha" (with one) — keep them apart.
    @MainActor
    private func windowCrumb(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label MATCHES '^[0-9]+:[^ ].*'")).firstMatch
    }

    @MainActor
    private func connectFromHome(_ app: XCUIApplication) -> XCUIElement {
        let card = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'connection-card-'")).firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 15), "home card missing")
        card.tap()   // offline → connect
        XCTAssertTrue(app.staticTexts["LIVE"].waitForExistence(timeout: 20), "never went live")
        return card
    }

    @MainActor
    private func tapCrumbExpectWindowsSheet(_ app: XCUIApplication) {
        let crumb = windowCrumb(app)
        XCTAssertTrue(crumb.waitForExistence(timeout: 20), "window crumb never appeared")
        shot(app, "terminal")
        crumb.tap()
        let opened = app.staticTexts["Swipe to switch"].waitForExistence(timeout: 5)
        shot(app, "after-tap")
        XCTAssertTrue(opened, "the window crumb did not open the Windows sheet")
    }

    @MainActor
    func testEnterViaCardThenCrumbOpensSheet() throws {
        let app = try launchOnHome()
        let card = connectFromHome(app)
        if card.exists { card.tap() }   // live → enter
        tapCrumbExpectWindowsSheet(app)
    }

    @MainActor
    func testEnterViaSessionTreeRowThenCrumbOpensSheet() throws {
        let app = try launchOnHome()
        _ = connectFromHome(app)
        let row = app.buttons.matching(NSPredicate(format: "label MATCHES '^[0-9]+: .*'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "session-tree window row missing")
        row.tap()
        tapCrumbExpectWindowsSheet(app)
    }
}
