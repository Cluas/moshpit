import XCTest

/// Adding a key as a person does it on the NSH pages: Settings › SSH Keys,
/// the bar's +, Add Key on Generate, a name, Add, and the key's row on SSH
/// Keys. Replaces SSHKeyFlowUITest's September walk of the SwiftUI key form.
/// Runs on CI's simulator: the simulator's keychain is a file
/// (`FileVaultKeychainBackend`) and has no Secure Enclave, so the key is a
/// software one.
///
/// `-MOSHPIT_RESET` leaves keys alone (they live in the keychain), so the
/// key is named for this run and found by that name, never by its place.
final class NSHKeyFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testGenerateAKeyAndItsRowShowsOnSSHKeys() throws {
        let app = NSHFlow.launch()
        app.openSettings()
        app.tapNSH("settings-ssh-keys")
        app.waitNSH("ssh-keys")

        app.tapNSH("ssh-keys-add")
        app.waitNSH("add-key")
        let name = "flow-\(Int(Date().timeIntervalSince1970) % 100_000)"
        app.typeNSH("add-key-name", name)
        let submit = app.waitNSH("add-key-submit")
        XCTAssertTrue(submit.isEnabled, "Add stays disabled with a name and Generate picked")
        submit.tap()

        // The sheet goes and the new key is listed under its name.
        let row = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'ssh-key-row-' AND label CONTAINS %@", name))
            .firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: NSHFlow.timeout), "no row for the generated key \(name)")
        XCTAssertFalse(app.nsh("add-key").exists, "Add Key is still up after Add")
    }
}
