import XCTest

/// Captures the theme screens as test attachments so the visual result can be
/// reviewed without driving the simulator by hand. Not a behavioural test — it
/// asserts only enough to fail loudly if a screen never appeared.
final class ThemeScreenshotUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    private func attach(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    @MainActor
    func testCaptureThemeScreens() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()

        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
        let themeRow = app.staticTexts["Theme"]
        if !themeRow.isHittable { app.swipeUp() }
        XCTAssertTrue(themeRow.waitForExistence(timeout: 3))
        attach(app, "01-settings-theme-row")

        themeRow.tap()
        XCTAssertTrue(app.navigationBars["Theme"].waitForExistence(timeout: 3))
        attach(app, "02-gallery-top")

        app.swipeUp()
        attach(app, "03-gallery-scrolled")

        // Editor, opened on a duplicate of a built-in.
        app.buttons["theme-row-dracula"].firstMatch.press(forDuration: 1.1)
        XCTAssertTrue(app.buttons["Duplicate & Edit"].waitForExistence(timeout: 3))
        app.buttons["Duplicate & Edit"].tap()
        XCTAssertTrue(app.navigationBars["Dracula Copy"].waitForExistence(timeout: 3))
        attach(app, "04-editor-preview")

        app.swipeUp()
        attach(app, "05-editor-ansi")

        // Bright overrides expanded.
        let brightToggle = app.switches["Override bright colors"]
        if brightToggle.waitForExistence(timeout: 2) {
            brightToggle.tap()
            attach(app, "06-editor-bright-expanded")
        }

        app.buttons["theme-editor-save"].tap()
        XCTAssertTrue(app.navigationBars["Theme"].waitForExistence(timeout: 3))
        attach(app, "07-gallery-with-custom")
    }

    @MainActor
    func testCaptureAppearanceScreens() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()

        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
        attach(app, "10-settings-appearance-rows")

        app.staticTexts["Accent"].tap()
        XCTAssertTrue(app.navigationBars["Accent"].waitForExistence(timeout: 3))
        attach(app, "11-accent-gallery")

        app.buttons["accent-add"].tap()
        XCTAssertTrue(app.navigationBars["My Accent"].waitForExistence(timeout: 3))
        attach(app, "12-accent-editor")
        app.buttons["accent-editor-save"].tap()
        XCTAssertTrue(app.navigationBars["Accent"].waitForExistence(timeout: 3))
        attach(app, "13-accent-with-custom")

        app.navigationBars["Accent"].buttons.firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 3))
        app.staticTexts["App Icon"].tap()
        XCTAssertTrue(app.navigationBars["App Icon"].waitForExistence(timeout: 3))
        attach(app, "14-icon-gallery")
    }
}
