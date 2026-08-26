import XCTest

/// Captures the host-setup screen in each of its states as test attachments, so
/// the visual result can be reviewed without driving a real host.
///
/// This test exists because the screen cannot otherwise be reached in a
/// simulator at all: Settings gates its row on a live SSH session, so the row is
/// disabled and the sheet's content is empty. That is the screen's own
/// precondition, not a limitation of the automation — which is why an earlier
/// attempt to reach it by tapping got nowhere.
///
/// Driving it from `-MOSHPIT_HOSTSETUP_DEMO` is also better than reaching a real
/// host would be. A host is in one state at a time, and the two most worth
/// looking at — an out-of-date install, and a relay that does not know this
/// phone — are the two hardest to arrange deliberately.
///
/// Not a behavioural test: it asserts only enough to fail loudly if a state
/// never rendered.
final class HostSetupScreenshotUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    private func attach(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// Launch straight into one state, confirm it rendered, and photograph it.
    @MainActor
    private func capture(_ scenario: String, _ name: String,
                         expect identifier: String? = nil) {
        let app = XCUIApplication()
        app.launchArguments = ["-MOSHPIT_RESET", "-MOSHPIT_HOSTSETUP_DEMO", scenario]
        app.launch()

        // The sheet's own title, so a blank screen fails here rather than
        // producing a screenshot of nothing.
        XCTAssertTrue(app.staticTexts["Set up this host"].waitForExistence(timeout: 10),
                      "\(scenario): the setup screen never appeared")
        if let identifier {
            XCTAssertTrue(app.descendants(matching: .any)[identifier].waitForExistence(timeout: 5),
                          "\(scenario): expected \(identifier) to be on screen")
        }
        attach(app, name)
        app.terminate()
    }

    @MainActor
    func testCaptureHostSetupStates() throws {
        // Nothing installed: what a new host looks like.
        capture("fresh", "01-fresh")

        // Both current — the state the old sheet could report and nothing else.
        capture("installed", "02-installed")

        // Out of date. Before the manifest existed there was no mechanism to
        // detect this at all, so no screen ever showed it.
        capture("stale", "03-stale")

        // Paired, and nothing on the host will ever fire a push. The failure the
        // previous design shipped with no name for it.
        capture("nothingWillPush", "04-paired-but-nothing-will-push")

        // The host is fine; this phone is not registered with the relay. Every
        // other row on the screen reports the host and cannot see this.
        capture("relayUnreachable", "05-relay-unreachable",
                expect: "hostsetup-relay-error")

        // A host missing openssl and curl: refused up front instead of installed
        // and left to fail at 3am.
        capture("missingTools", "06-missing-tools")
    }
}
