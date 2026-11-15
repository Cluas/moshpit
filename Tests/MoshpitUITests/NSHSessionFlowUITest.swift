import XCTest

/// The path a person takes most, on a live host: the seeded connection
/// connects, its terminal comes up, a tap on the breadcrumb's window
/// segment opens the pane overview (since b4b43be1) and Done closes it, a
/// long press opens the window picker on the window the terminal shows,
/// picking it closes the picker on the same terminal, and Back is Home with
/// the host's card.
/// Replaces HomeEntryBreadcrumbUITest and SheetFocusUITest, which looked
/// for the SwiftUI Windows sheet's "Swipe to switch".
///
/// Needs a host: the one CI stages before the tests
/// (`scripts/shots/stage-host.sh`: sshd on loopback, tmux on a private
/// socket), or the local rig, handed over as `TEST_RUNNER_MOSHPIT_SEED_*`
/// (user, key, host, port, the host key's fingerprint, name, `tmux` and its
/// binary). Skips without one.
final class NSHSessionFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testConnectOpenTheWindowPickerAndGoBackHome() throws {
        guard let session = NSHFlow.seededSession() else {
            throw XCTSkip("needs a host: TEST_RUNNER_MOSHPIT_SEED_USER and _KEY_B64 (the local rig, or a staged one)")
        }
        // Home first (`-MOSHPIT_SEED_HOME 1`): without it the seeded
        // terminal is the app's root and Back has nowhere to go (run 11).
        let app = NSHFlow.launch(session + ["-MOSHPIT_SEED_HOME", "1"])
        let name = ProcessInfo.processInfo.environment["MOSHPIT_SEED_NAME"] ?? "devbox"

        // The connection connects from Home; its card's head opens its
        // terminal.
        let head = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", name)).firstMatch
        XCTAssertTrue(head.waitForExistence(timeout: 30), "Home has no card for \(name)")
        head.tap()
        app.waitNSH("terminal-screen", timeout: 30, "the seeded session's terminal never came up")
        let window = app.waitNSH("terminal-crumb-window", timeout: 30, "no window segment on the breadcrumb")
        XCTAssertTrue(window.isEnabled, "the window segment is inert on a live session")

        // A tap: the overview of every window and pane; Done closes it.
        window.tap()
        app.waitNSH("terminal-overview", "a tap on the window segment didn't open the overview")
        app.tapNSH("terminal-overview-done")
        XCTAssertTrue(app.nsh("terminal-overview").waitForNonExistence(timeout: 5), "Done left the overview up")

        // A long press: the picker, on the window the terminal shows.
        window.press(forDuration: 0.8)
        app.waitNSH("mux-picker", "a long press on the window segment didn't open the picker")
        let current = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'mux-picker-row-' AND selected == true"))
            .firstMatch
        XCTAssertTrue(current.waitForExistence(timeout: 5), "the picker opened with no window marked current")

        // Picking it closes the picker on the same terminal.
        current.tap()
        XCTAssertTrue(app.nsh("mux-picker").waitForNonExistence(timeout: 5), "picking the current window left the picker up")
        XCTAssertTrue(app.nsh("terminal-screen").exists)

        // Back is Home, with the host's card.
        app.tapNSH("terminal-back")
        app.waitNSH("home-settings", "Back from the terminal didn't reach Home")
        XCTAssertTrue(app.nsh(anyOf: ["home-card-\(name)", "connection-card-\(name)"])
                        .waitForExistence(timeout: NSHFlow.timeout),
                      "Home has no card for \(name)")
    }
}
