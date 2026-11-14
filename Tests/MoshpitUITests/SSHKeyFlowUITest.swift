import XCTest

/// Generate a key from inside Add Connection, then delete it from SSH Keys.
///
/// Covers what the 2026-10-10 SwiftUI review found broken: a key generated from
/// the picker was never selected (`onCreated` was never called), the generate
/// form offered an algorithm it cannot generate (ECDSA-sk) and a passphrase it
/// never used, and Delete Key removed a key with no confirmation.
final class SSHKeyFlowUITest: XCTestCase {

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
    func testGenerateFromPickerThenDeleteWithConfirmation() throws {
        try skipUntilRewrittenForNSH("key form")
        let keyName = "qa-\(UUID().uuidString.prefix(6).lowercased())"
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_RESET", "-MOSHPIT_AUTOCARE_OFF"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Moshpit"].waitForExistence(timeout: 5))

        // Add Connection → SSH Key → key picker → generate.
        app.buttons["home-add"].tap()
        XCTAssertTrue(app.navigationBars["Add Connection"].waitForExistence(timeout: 3))
        app.buttons["SSH Key"].firstMatch.tap()
        let pickerRow = app.buttons["key-picker-row"].firstMatch
        XCTAssertTrue(app.reveal(pickerRow))
        pickerRow.tap()
        let add = app.buttons["key-picker-add"].firstMatch
        let emptyGenerate = app.buttons["key-picker-empty-generate"].firstMatch
        XCTAssertTrue(add.waitForExistence(timeout: 3) || emptyGenerate.exists)
        (emptyGenerate.exists ? emptyGenerate : add).tap()

        let addKeyBar = app.navigationBars["Add Key"]
        XCTAssertTrue(addKeyBar.waitForExistence(timeout: 3))
        // Generate offers only what SSHKeyFactory.generate can make, and no
        // passphrase it would never use.
        XCTAssertTrue(app.buttons["RSA-4096"].firstMatch.exists, "positive control: the picker is on screen")
        XCTAssertFalse(app.buttons["ECDSA-sk"].exists)
        XCTAssertFalse(app.staticTexts["ECDSA-sk"].exists)
        XCTAssertFalse(app.secureTextFields["Passphrase"].exists)
        XCTAssertFalse(app.secureTextFields["Confirm Passphrase"].exists)
        attach(app, "01-add-key-generate")

        // Add Connection's own Name field is still in the tree behind the
        // sheet; the Add Key one is the hittable one.
        let nameFields = app.textFields.matching(NSPredicate(format: "placeholderValue == 'Name'"))
        let name = try XCTUnwrap(nameFields.allElementsBoundByIndex.last { $0.isHittable },
                                 "Add Key's Name field should be on screen")
        name.tap()
        name.typeText(keyName)
        // The simulator has no enrolled biometry to bind the Keychain item to.
        let faceID = app.switches["Require Face ID"].firstMatch
        XCTAssertTrue(app.reveal(faceID))
        if (faceID.value as? String) == "1" { faceID.tap() }
        addKeyBar.buttons["Generate"].tap()

        // Back on Add Connection with the new key already chosen.
        XCTAssertTrue(app.navigationBars["Add Connection"].waitForExistence(timeout: 5),
                      "generating from the picker should land back on Add Connection")
        XCTAssertFalse(addKeyBar.exists)
        let selected = app.buttons["key-picker-row"].firstMatch
        XCTAssertTrue(app.reveal(selected))
        let label = [selected.label, selected.value as? String ?? ""].joined(separator: " ")
        XCTAssertTrue(label.contains(keyName), "the generated key should be selected, got '\(label)'")
        attach(app, "02-add-connection-key-selected")
        app.navigationBars["Add Connection"].buttons["Cancel"].tap()

        // Settings → SSH Keys → long-press → Delete Key → confirm card.
        app.buttons["home-settings"].tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 5))
        let keysRow = app.staticTexts["SSH Keys"].firstMatch
        XCTAssertTrue(app.reveal(keysRow))
        keysRow.tap()
        XCTAssertTrue(app.navigationBars["SSH Keys"].waitForExistence(timeout: 3))

        let row = app.staticTexts[keyName].firstMatch
        XCTAssertTrue(app.reveal(row))
        let deleteItem = app.buttons["Delete Key"]
        pressForMenu(row, until: deleteItem, "the key row should offer Delete Key")
        deleteItem.tap()

        let cardTitle = app.staticTexts["Delete \(keyName)?"]
        XCTAssertTrue(cardTitle.waitForExistence(timeout: 3), "Delete Key should ask first")
        XCTAssertTrue(app.staticTexts[keyName].exists, "nothing is deleted before the answer")
        attach(app, "03-delete-key-card")

        // Cancel keeps it.
        app.buttons["Cancel"].firstMatch.tap()
        XCTAssertFalse(cardTitle.waitForExistence(timeout: 1))
        XCTAssertTrue(app.staticTexts[keyName].exists)

        // Delete removes it.
        pressForMenu(app.staticTexts[keyName].firstMatch, until: deleteItem)
        deleteItem.tap()
        XCTAssertTrue(cardTitle.waitForExistence(timeout: 3))
        app.buttons["Delete"].firstMatch.tap()
        XCTAssertFalse(app.staticTexts[keyName].waitForExistence(timeout: 2),
                       "the confirmed key should leave the list")
        attach(app, "04-after-delete")
    }
}
