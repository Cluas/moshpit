import XCTest

/// Smoke test for the prototype-v2 UI: Attach home → Add Connection form →
/// Settings (display/cursor groups) → Shortcuts editor → SSH Keys list.
final class MainFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testMainFlowSmoke() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]   // empty store → + always opens the form
        app.launch()

        // 1. Home: large "Attach" title
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 5),
                      "Moshpit title should be visible at launch")

        // 2. ＋ opens Add Connection
        let addButton = app.buttons["home-add"]
        XCTAssertTrue(addButton.waitForExistence(timeout: 3))
        addButton.tap()

        XCTAssertTrue(app.navigationBars["Add Connection"].waitForExistence(timeout: 3))

        // 3. Save is disabled until Name + Host are filled
        let saveButton = app.buttons["save-connection"]
        XCTAssertTrue(saveButton.waitForExistence(timeout: 2))
        XCTAssertFalse(saveButton.isEnabled, "Save must be disabled while the form is empty")

        // 4. Mosh group present — and progressively disclosed: the tuning
        // rows (roam, ports, prediction) only exist while the toggle is ON.
        // "Roam on Cellular: ON" under a disabled Mosh was a lie about what
        // would happen, so their absence here is the designed state.
        XCTAssertTrue(app.staticTexts["Use Mosh"].exists)
        XCTAssertFalse(app.staticTexts["Roam on Cellular"].exists,
                       "mosh tuning rows must stay hidden while Use Mosh is off")
        app.switches["toggle-use-mosh"].tap()
        XCTAssertTrue(app.staticTexts["Roam on Cellular"].waitForExistence(timeout: 3),
                      "enabling Use Mosh should reveal the tuning rows")
        app.switches["toggle-use-mosh"].tap()

        // 5. SSH Key auth reveals the PEM editor
        app.buttons["SSH Key"].firstMatch.tap()
        XCTAssertTrue(app.textViews["private-key-editor"].waitForExistence(timeout: 3),
                      "PEM editor should appear after switching to SSH Key auth")

        // 6. Cancel back to home
        app.navigationBars["Add Connection"].buttons["Cancel"].tap()
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 3))

        // 7. Gear opens Settings with prototype groups
        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Font"].exists)
        XCTAssertTrue(app.staticTexts["Trail on predict"].exists)

        // 8. Shortcuts editor
        app.staticTexts["Shortcuts"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Shortcuts"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Escape"].exists)

        // 8b. Create a custom Key Combo shortcut end-to-end
        app.navigationBars["Shortcuts"].buttons["plus"].tap()
        XCTAssertTrue(app.navigationBars["Add Shortcut"].waitForExistence(timeout: 3))

        let keyField = app.textFields["shortcut-key"]
        XCTAssertTrue(keyField.waitForExistence(timeout: 2))
        keyField.tap()
        keyField.typeText("b")

        let chipField = app.textFields["shortcut-chip-label"]
        chipField.tap()
        chipField.typeText("C-b")

        let descField = app.textFields["shortcut-description"]
        descField.tap()
        descField.typeText("tmux prefix")

        let saveShortcut = app.navigationBars["Add Shortcut"].buttons["Save"]
        XCTAssertTrue(saveShortcut.isEnabled, "Save must enable once key + chip label are set")
        saveShortcut.tap()

        // Lands in the CUSTOM group of the editor
        XCTAssertTrue(app.staticTexts["tmux prefix"].waitForExistence(timeout: 3),
                      "custom shortcut should appear in the list after saving")
        app.navigationBars["Shortcuts"].buttons["Done"].tap()

        // 9. SSH Keys list
        app.staticTexts["SSH Keys"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["SSH Keys"].waitForExistence(timeout: 3))
        app.navigationBars["SSH Keys"].buttons["Done"].tap()

        // 10. Done returns home
        app.navigationBars["Settings"].buttons["Done"].tap()
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 3))
    }

    /// Adds a connection, then deletes it through the card's long-press
    /// context menu — the only edit/delete path for a saved-but-offline
    /// server. Guards the regression where offline cards were undeletable.
    @MainActor
    func testAddAndDeleteConnection() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET"]   // empty store, under the free cap
        app.launch()
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 5))

        // Add a connection named "QA-Delete".
        app.buttons["home-add"].tap()
        XCTAssertTrue(app.navigationBars["Add Connection"].waitForExistence(timeout: 3))

        let name = app.textFields.firstMatch
        name.tap()
        name.typeText("QA-Delete")

        // Second text field is Host.
        let host = app.textFields.element(boundBy: 1)
        host.tap()
        host.typeText("qa.example.com")

        let save = app.buttons["save-connection"]
        XCTAssertTrue(save.isEnabled)
        save.tap()

        // Card appears on Home.
        let card = app.buttons["connection-card-QA-Delete"]
        XCTAssertTrue(card.waitForExistence(timeout: 3), "new connection card should appear")

        // Long-press → context menu → Delete Connection → confirm.
        card.press(forDuration: 1.1)
        let deleteItem = app.buttons["Delete Connection"]
        XCTAssertTrue(deleteItem.waitForExistence(timeout: 3), "context menu should offer delete")
        deleteItem.tap()

        // confirmationDialog's destructive button is labelled just "Delete".
        let confirm = app.buttons["Delete"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 3))
        confirm.tap()

        // Card is gone.
        XCTAssertFalse(app.buttons["connection-card-QA-Delete"].waitForExistence(timeout: 3),
                       "deleted connection should disappear from Home")
    }

}
