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
        // Stand on the mainland side of the storefront line, so the Settings
        // footer check below is a positive control for the filing number.
        app.launchArguments += [MainlandStorefrontArgs.mainland, MainlandStorefrontArgs.code]
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
        // Settings is a lazy Form: rows below the fold don't exist to XCUI
        // until scrolled into view.
        XCTAssertTrue(app.reveal(app.staticTexts["Trail on predict"]))
        // MIIT wants the APP filing number visible inside the app for copies
        // from the mainland store; it lives in the Settings footer next to the
        // build identity. (This launch is on the mainland side — see above.)
        XCTAssertTrue(app.reveal(app.descendants(matching: .any).matching(identifier: "settings-icp").firstMatch),
                      "APP filing number must be present in Settings for the mainland storefront")

        // 8. Shortcuts editor
        XCTAssertTrue(app.reveal(app.staticTexts["Shortcuts"].firstMatch))
        app.staticTexts["Shortcuts"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Shortcuts"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Escape"].exists)

        // 8b. Create a custom Key Combo shortcut end-to-end
        app.navigationBars["Shortcuts"].buttons["plus"].tap()
        XCTAssertTrue(app.navigationBars["Add Shortcut"].waitForExistence(timeout: 3))

        // The fields sit under the quick-key grid; the form is lazy, so each
        // one is scrolled into view before it is touched.
        let keyField = app.textFields["shortcut-key"]
        XCTAssertTrue(app.reveal(keyField))
        keyField.tap()
        keyField.typeText("b")

        let chipField = app.textFields["shortcut-chip-label"]
        XCTAssertTrue(app.reveal(chipField))
        chipField.tap()
        chipField.typeText("C-b")

        let descField = app.textFields["shortcut-description"]
        XCTAssertTrue(app.reveal(descField))
        descField.tap()
        descField.typeText("tmux prefix")

        let saveShortcut = app.navigationBars["Add Shortcut"].buttons["Save"]
        XCTAssertTrue(saveShortcut.isEnabled, "Save must enable once key + chip label are set")
        saveShortcut.tap()

        // Lands in the editor twice: a new custom shortcut goes straight
        // onto the bar, so it shows under IN TOOLBAR and under CUSTOM. The
        // query is ambiguous by design — `firstMatch`, or `isHittable` throws.
        XCTAssertTrue(app.navigationBars["Shortcuts"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.reveal(app.staticTexts["tmux prefix"].firstMatch),
                      "custom shortcut should appear in the list after saving")
        app.navigationBars["Shortcuts"].buttons["Done"].tap()

        // 9. SSH Keys list
        XCTAssertTrue(app.reveal(app.staticTexts["SSH Keys"].firstMatch))
        app.staticTexts["SSH Keys"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["SSH Keys"].waitForExistence(timeout: 3))
        app.navigationBars["SSH Keys"].buttons["Done"].tap()

        // 10. Done returns home
        app.navigationBars["Settings"].buttons["Done"].tap()
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 3))
    }

    /// The other side of the storefront line: bought anywhere but the mainland
    /// store, Settings carries no filing number. Without this the positive
    /// check above would also pass for a footer that simply shows it to all.
    @MainActor
    func testFilingNumberHiddenOutsideMainland() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET", MainlandStorefrontArgs.mainland, "USA"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 5))

        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 3))
        // The build-identity line proves the footer rendered before we assert
        // on what is missing from it.
        XCTAssertTrue(app.reveal(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Moshpit ' AND label CONTAINS '('"))
                        .firstMatch),
                      "footer version line should render")
        XCTAssertFalse(app.descendants(matching: .any).matching(identifier: "settings-icp").firstMatch
                        .waitForExistence(timeout: 2),
                       "APP filing number must stay hidden outside the mainland storefront")
        app.navigationBars["Settings"].buttons["Done"].tap()
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
        let deleteItem = app.buttons["Delete Connection"]
        pressForMenu(card, until: deleteItem, "context menu should offer delete")
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

/// The DEBUG launch argument MainlandStorefront reads; mirrored here because
/// the UI-test bundle cannot import the app module.
enum MainlandStorefrontArgs {
    static let mainland = "-MOSHPIT_STOREFRONT"
    static let code = "CHN"
}
