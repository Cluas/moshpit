import XCTest

/// End-to-end walk of the terminal-theme feature: gallery → duplicate a
/// built-in → edit it → save → confirm it became the selection, then import a
/// pasted palette and delete a theme.
///
/// Driven through accessibility identifiers rather than coordinates: the theme
/// rows are a scrolling list whose positions shift as custom themes are added,
/// and this suite has to keep passing when they do.
final class ThemeFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// Settings → Theme, returning the gallery's navigation bar.
    @MainActor
    private func openGallery(_ app: XCUIApplication) -> XCUIElement {
        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))

        let themeRow = app.staticTexts["Theme"]
        // The DISPLAY group sits below the fold on smaller devices.
        if !themeRow.isHittable {
            app.swipeUp()
        }
        XCTAssertTrue(themeRow.waitForExistence(timeout: 3))
        themeRow.tap()

        let gallery = app.navigationBars["Theme"]
        XCTAssertTrue(gallery.waitForExistence(timeout: 3), "Theme row should push the gallery")
        return gallery
    }

    @MainActor
    func testGalleryListsBuiltInsAndSwitchesSelection() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        _ = openGallery(app)

        // Every built-in is offered, by name.
        XCTAssertTrue(app.staticTexts["GitHub Dark"].exists)
        XCTAssertTrue(app.staticTexts["Dracula"].exists)
        XCTAssertTrue(app.staticTexts["Tokyo Night"].exists)
        XCTAssertTrue(app.staticTexts["No custom themes yet."].exists,
                      "A fresh install has no custom themes")

        // Picking one selects it: back in Settings the row shows the new name.
        app.buttons["theme-row-dracula"].firstMatch.tap()
        app.navigationBars["Theme"].buttons.firstMatch.tap()      // back
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Dracula"].waitForExistence(timeout: 3),
                      "Settings' Theme row should reflect the picked theme")
    }

    @MainActor
    func testDuplicateEditAndSaveBecomesSelection() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        _ = openGallery(app)

        // Long-press a built-in → Duplicate & Edit (the only route from a
        // read-only built-in to something editable).
        app.buttons["theme-row-nord"].firstMatch.press(forDuration: 1.1)
        let duplicate = app.buttons["Duplicate & Edit"]
        XCTAssertTrue(duplicate.waitForExistence(timeout: 3))
        duplicate.tap()

        // The editor opens on a copy, pre-named and previewing live.
        XCTAssertTrue(app.navigationBars["Nord Copy"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Background"].exists)
        XCTAssertFalse(app.staticTexts["Bright Black"].exists,
                       "Bright overrides start collapsed for a theme that has none")
        XCTAssertTrue(app.staticTexts["Override bright colors"].exists)

        // Rename it so the assertion below can't accidentally match the source.
        let nameField = app.textFields["Theme name"]
        XCTAssertTrue(nameField.waitForExistence(timeout: 3))
        nameField.tap()
        nameField.press(forDuration: 1.2)
        if app.menuItems["Select All"].waitForExistence(timeout: 2) {
            app.menuItems["Select All"].tap()
        }
        nameField.typeText("Test Palette")

        app.buttons["theme-editor-save"].tap()

        // Saving both stores it and makes it the active theme.
        XCTAssertTrue(app.navigationBars["Theme"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Test Palette"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.staticTexts["No custom themes yet."].exists)

        app.navigationBars["Theme"].buttons.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Test Palette"].waitForExistence(timeout: 3),
                      "Editing a theme implies selecting it")
    }

    @MainActor
    func testImportPastedPaletteCreatesSelectableTheme() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        _ = openGallery(app)

        app.buttons["theme-add"].tap()
        let importButton = app.buttons["Import JSON…"]
        XCTAssertTrue(importButton.waitForExistence(timeout: 3))
        importButton.tap()

        let editor = app.textViews["theme-import-editor"]
        XCTAssertTrue(editor.waitForExistence(timeout: 3))
        editor.tap()
        // Shorthand keys and no id — the shape someone pastes from a gist.
        editor.typeText("""
        {"name":"Pasted VS","bg":"#1E1E1E","fg":"#D4D4D4","black":"#000000",\
        "red":"#CD3131","green":"#0DBC79","yellow":"#E5E510","blue":"#2472C8",\
        "magenta":"#BC3FBC","cyan":"#11A8CD","white":"#E5E5E5"}
        """)
        app.navigationBars["Import Theme"].buttons["Import"].tap()

        XCTAssertTrue(app.staticTexts["Pasted VS"].waitForExistence(timeout: 4),
                      "An imported theme should appear under MY THEMES and be selected")
    }

    @MainActor
    func testDeleteCustomThemeFallsBackToDefault() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]
        app.launch()
        _ = openGallery(app)

        // Make a custom theme to delete.
        app.buttons["theme-row-monokai"].firstMatch.press(forDuration: 1.1)
        app.buttons["Duplicate & Edit"].tap()
        XCTAssertTrue(app.navigationBars["Monokai Copy"].waitForExistence(timeout: 3))
        app.buttons["theme-editor-save"].tap()

        let copyRow = app.staticTexts["Monokai Copy"]
        XCTAssertTrue(copyRow.waitForExistence(timeout: 3))

        copyRow.press(forDuration: 1.1)
        let deleteButton = app.buttons["Delete"]
        XCTAssertTrue(deleteButton.waitForExistence(timeout: 3))
        deleteButton.tap()
        // Confirm in the app's own modal (the redesign replaced UIKit alerts
        // with the in-app modal system — XCUI's `app.alerts` no longer sees
        // it). The title wait doubles as proof the modal is up and the
        // context menu's identically-labelled Delete is gone.
        XCTAssertTrue(app.staticTexts["Delete Monokai Copy?"].waitForExistence(timeout: 3))
        app.buttons["Delete"].firstMatch.tap()

        XCTAssertTrue(app.staticTexts["No custom themes yet."].waitForExistence(timeout: 3))
        // Deleting the *selected* theme has to move the selection, or the
        // terminal would silently fall back with no visible reason.
        app.navigationBars["Theme"].buttons.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["GitHub Dark"].waitForExistence(timeout: 3),
                      "Selection should fall back to the default theme")
    }
}
