import XCTest

/// Walks the split appearance settings: the accent picker (including creating a
/// custom accent) and the independent app-icon gallery.
final class AppearanceFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    private func openSettings(_ app: XCUIApplication) {
        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testAccentAndIconAreSeparateRows() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        openSettings(app)

        // They used to be one list; the split is the feature.
        XCTAssertTrue(app.staticTexts["Accent"].exists)
        XCTAssertTrue(app.staticTexts["App Icon"].exists)
    }

    @MainActor
    func testCreateCustomAccentAppliesIt() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        openSettings(app)

        app.staticTexts["Accent"].tap()
        XCTAssertTrue(app.navigationBars["Accent"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Signal Room"].exists)
        XCTAssertTrue(app.staticTexts["No custom accents yet."].exists)

        // Built-in accents remain switchable.
        app.buttons["accent-row-terminal-green"].firstMatch.tap()

        // Picking must NOT navigate away — comparing accents is the point of
        // the gallery, and an immediate settings write used to rebuild the root
        // view and pop this screen.
        XCTAssertTrue(app.navigationBars["Accent"].exists,
                      "Picking an accent should keep the gallery on screen")

        // New custom accent — the editor opens on the current color.
        app.buttons["accent-add"].tap()
        XCTAssertTrue(app.navigationBars["My Accent"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Accent preview"].exists ||
                      app.otherElements["Accent preview"].exists,
                      "The editor previews the accent on app chrome")

        app.buttons["accent-editor-save"].tap()

        // Saving selects it, and it shows under MY ACCENTS.
        XCTAssertTrue(app.navigationBars["Accent"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["My Accent"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.staticTexts["No custom accents yet."].exists)
    }

    @MainActor
    func testIconGalleryOffersDistinctIconsAndSwitches() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        openSettings(app)

        app.staticTexts["App Icon"].tap()
        XCTAssertTrue(app.navigationBars["App Icon"].waitForExistence(timeout: 3))

        // The gallery includes the two non-mark figures, which is the point of
        // "the icon is its own choice".
        for id in ["default", "teal", "daylight", "mono", "cursor", "hail"] {
            XCTAssertTrue(app.buttons["app-icon-\(id)"].exists, "missing icon option \(id)")
        }

        // Switching the icon must not disturb the accent. Note the simulator
        // presents its own "you changed the icon" alert on success; dismiss it
        // if it appears so the run doesn't stall.
        app.buttons["app-icon-hail"].tap()
        let alert = app.alerts.firstMatch
        if alert.waitForExistence(timeout: 3) {
            alert.buttons.element(boundBy: alert.buttons.count - 1).tap()
        }

        app.navigationBars["App Icon"].buttons.firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 3))
        // Accent row still reads the built-in default, untouched by the icon change.
        XCTAssertTrue(app.staticTexts["Signal Room"].waitForExistence(timeout: 3))
    }
}
