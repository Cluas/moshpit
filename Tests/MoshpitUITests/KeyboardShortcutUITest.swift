import UIKit
import XCTest

/// Hardware-keyboard shortcuts (iPad with a keyboard, Mac). `typeKey` puts
/// the chord on the device the way a keyboard would — the Simulator app's
/// own menu shortcuts (⌘K, ⌘W, ⌘1…) never see it, which is why this cannot
/// be driven from the Mac side and lives here.
///
/// Run on the iPad simulator. The iPhone 17 Pro simulator on this machine
/// stopped delivering `typeKey` chords to the app mid-session (the same
/// build kept passing on the iPad), so a red here on an iPhone destination
/// is the device, not the feature — the feature is an iPad one anyway.
final class KeyboardShortcutUITest: XCTestCase {
    override func setUpWithError() throws {
        continueAfterFailure = false
        // The class comment is the rule: this suite runs on the iPad
        // simulator. On an iPhone destination the chords stop reaching the
        // app, so a red there says nothing about the feature; skip instead.
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad,
                          "hardware-keyboard shortcuts are exercised on the iPad simulator")
    }

    @MainActor
    func testHomeShortcutsOpenSettingsAndAddConnection() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        XCTAssertTrue(app.navigationBars["Moshpit"].waitForExistence(timeout: 5))

        app.typeKey(",", modifierFlags: .command)
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 3),
                      "⌘, opens Settings")
        app.navigationBars["Settings"].buttons["Done"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForNonExistence(timeout: 3))

        app.typeKey("n", modifierFlags: .command)
        XCTAssertTrue(app.navigationBars["Add Connection"].waitForExistence(timeout: 3),
                      "⌘N opens Add Connection")
        app.navigationBars["Add Connection"].buttons["Cancel"].tap()
        XCTAssertTrue(app.navigationBars["Add Connection"].waitForNonExistence(timeout: 3))
    }

    /// Needs a live host: run with
    /// `TEST_RUNNER_MOSHPIT_SEED_USER=<user> TEST_RUNNER_MOSHPIT_SEED_KEY_B64=<pem b64>`
    /// (and optionally `TEST_RUNNER_MOSHPIT_SEED_TMUX_BIN`) against a tmux
    /// server with at least two windows. Skips otherwise.
    @MainActor
    func testTerminalShortcutsSwitchWindowsAndLeave() throws {
        let env = ProcessInfo.processInfo.environment
        guard let user = env["MOSHPIT_SEED_USER"], let key = env["MOSHPIT_SEED_KEY_B64"] else {
            throw XCTSkip("no MOSHPIT_SEED_USER / MOSHPIT_SEED_KEY_B64 in the test runner's environment")
        }
        let app = XCUIApplication()
        app.launchArguments += [
            "-MOSHPIT_RESET", "-MOSHPIT_AUTOCARE_OFF", "1", "-MOSHPIT_SEED_HOME", "1",
            "-MOSHPIT_SEED_USER", user, "-MOSHPIT_SEED_KEY_B64", key,
            "-MOSHPIT_SEED_HOST", env["MOSHPIT_SEED_HOST"] ?? "127.0.0.1",
            "-MOSHPIT_SEED_PORT", env["MOSHPIT_SEED_PORT"] ?? "22",
            "-MOSHPIT_SEED_NAME", "local-mac", "-MOSHPIT_SEED_TMUX", "1", "-MOSHPIT_SEED_QUIET", "1",
        ]
        if let tmux = env["MOSHPIT_SEED_TMUX_BIN"] {
            app.launchArguments += ["-MOSHPIT_SEED_TMUX_BIN", tmux]
        }
        app.launch()

        // Connect from the card; trust the host key the first time.
        let card = app.buttons["connection-card-local-mac"]
        XCTAssertTrue(card.waitForExistence(timeout: 5))
        card.tap()
        let trust = app.buttons["Trust"]
        if trust.waitForExistence(timeout: 8) { trust.tap() }
        // The tmux tree shows once attached; the card head then opens the terminal.
        XCTAssertTrue(app.buttons["1: shell, 2p"].waitForExistence(timeout: 20),
                      "the seeded tmux server should report its first window")
        card.tap()
        // iPhone shows Back; the iPad split layout shows the sidebar toggle.
        let bar = app.buttons.matching(
            NSPredicate(format: "identifier IN {'terminal-back', 'terminal-sidebar'}")).firstMatch
        XCTAssertTrue(bar.waitForExistence(timeout: 8), "the terminal should be on screen")
        XCTAssertTrue(app.buttons["1:shell"].waitForExistence(timeout: 8),
                      "the breadcrumb should name the active window")

        // ⌘K with the keyboard down: nothing is first responder, so UIKit
        // starts its key-command walk at the view controller — the case a
        // hidden SwiftUI shortcut button never covered.
        app.typeKey("k", modifierFlags: .command)
        XCTAssertTrue(app.navigationBars["Windows"].waitForExistence(timeout: 4), "⌘K opens Windows")
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH '2: logs'")).firstMatch.tap()
        XCTAssertTrue(app.buttons["2:logs"].waitForExistence(timeout: 6),
                      "picking a window should move the breadcrumb")

        // ⌘1 with the terminal focused: the chord now goes through SwiftTerm's
        // pressesBegan first, which must hand it on.
        app.buttons["keyboard-toggle"].tap()
        app.typeKey("1", modifierFlags: .command)
        XCTAssertTrue(app.buttons["1:shell"].waitForExistence(timeout: 6), "⌘1 selects window 1")

        // ⌘W: leave the terminal; the session stays alive on Home.
        app.typeKey("w", modifierFlags: .command)
        XCTAssertTrue(app.navigationBars["Moshpit"].waitForExistence(timeout: 5), "⌘W goes back to Home")
        XCTAssertTrue(app.buttons["1: shell, 2p"].waitForExistence(timeout: 5),
                      "the session is still attached after leaving")
    }
}
