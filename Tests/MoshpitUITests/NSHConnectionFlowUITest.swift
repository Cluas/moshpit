import XCTest

/// Adding a host and taking it away again, on the NSH pages: Home's +, the
/// connection form (name, host, user, a password, which Save needs), the
/// saved row on Home, then its long-press menu's Delete… and the
/// confirmation. Replaces MainFlowUITest's SwiftUI walk of the form. Runs
/// on CI's simulator: the host is example.com's and never dialed (a saved
/// row connects only when tapped).
final class NSHConnectionFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// Save waits for what a connection needs; Use Mosh shows its rows (the
    /// prediction, the mosh-server path) only while it is on: rows for a
    /// Mosh that is off would say something untrue.
    @MainActor
    func testSaveWaitsForAHostAndMoshShowsItsRowsOnlyWhenOn() throws {
        let app = NSHFlow.launch()
        app.tapNSH("home-add")
        app.waitNSH("connection-form")
        XCTAssertFalse(app.waitNSH("save-connection").isEnabled, "Save is enabled on an empty form")

        // A new connection starts with Mosh as Settings' default says
        // (run 11: on), so the flow reads the switch first and checks both
        // ways from there.
        let mosh = app.waitNSH("toggle-use-mosh")
        if !mosh.isHittable { app.reveal(mosh) }
        let rows = app.nsh("connection-form-predict")
        let startsOn = try XCTUnwrap(mosh.isOn, "Use Mosh reports no state: \(mosh.spoken)")
        XCTAssertEqual(rows.exists, startsOn, "Mosh's rows don't follow Use Mosh (on: \(startsOn))")
        mosh.tap()
        if startsOn {
            XCTAssertTrue(rows.waitForNonExistence(timeout: 5), "Mosh's rows stayed after turning it off")
        } else {
            XCTAssertTrue(rows.waitForExistence(timeout: 5), "turning Use Mosh on showed none of its rows")
        }
        app.nsh("toggle-use-mosh").tap()
        XCTAssertEqual(app.nsh("toggle-use-mosh").isOn, startsOn, "Use Mosh didn't flip back")

        app.tapNSH("connection-form-cancel")
        app.waitNSH("home-settings", "Cancel didn't go back to Home")
    }

    @MainActor
    func testAddAHostThenDeleteIt() throws {
        let app = NSHFlow.launch()
        let name = "flow-box"

        app.tapNSH("home-add")
        app.waitNSH("connection-form")
        app.typeNSH("connection-form-name", name)
        app.typeNSH("connection-form-host", "devbox.example.com")
        app.typeNSH("connection-form-username", "dev")
        let password = app.nsh("connection-form-password")
        if !password.waitForExistence(timeout: 3) {
            // A form that opened on Key: Password is the auth control's
            // first segment.
            app.waitNSH("connection-form-auth").buttons.element(boundBy: 0).tap()
        }
        app.tapNSH("connection-form-password")
        app.nsh("connection-form-password").typeText("not-a-real-password")

        let save = app.waitNSH("save-connection")
        XCTAssertTrue(save.isEnabled, "Save stays disabled with a host, a user and a password")
        save.tap()

        // Home lists it, saved and not connected.
        let row = app.waitNSH("connection-card-\(name)", "the saved host never showed on Home")
        XCTAssertTrue(row.spoken.contains("devbox.example.com") || row.spoken.contains(name),
                      "the saved row says neither its name nor its host: \(row.spoken)")

        // Its menu deletes it, once confirmed. The row's swipe tray has a
        // Delete… too, out of sight: the menu's is the one on screen.
        let delete = app.buttons.matching(NSPredicate(format: "label == 'Delete…'"))
        pressForMenu(row, until: delete.firstMatch, "the saved row's menu never opened")
        let shown = (0..<delete.count).map { delete.element(boundBy: $0) }.first { $0.isHittable }
        try XCTUnwrap(shown, "no Delete… on screen").tap()
        let confirm = app.buttons.matching(NSPredicate(format: "label == 'Delete'")).firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), "deleting asked nothing")
        confirm.tap()
        XCTAssertTrue(row.waitForNonExistence(timeout: NSHFlow.timeout), "the host is still on Home after Delete")
    }
}
