import XCTest

/// Settings as a person walks it on the NSH pages: from Home's bar to the
/// hub, into a part, a pick on a gallery, back, and the hub's row saying
/// what was picked. Replaces the September flows written for the SwiftUI
/// sheet (ThemeFlowUITest, AppearanceFlowUITest, the Settings half of
/// MainFlowUITest). Runs on CI's simulator: no host, no rig.
///
/// What a page does with a press is the Rust UI tests' (`ui/tests`); these
/// prove the press gets there on a device, through the Swift host, and that
/// what it changed is what the app reads back.
final class NSHSettingsFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// The hub opens each part, and each part has the rows it is for.
    @MainActor
    func testHubOpensAppearanceAndTerminal() throws {
        let app = NSHFlow.launch()
        app.openSettings()

        app.openSettingsPart("appearance")
        app.waitNSH("settings-accent")
        app.waitNSH("settings-app-icon")
        app.backNSH()

        app.openSettingsPart("terminal")
        app.waitNSH("settings-theme")
        app.waitNSH("settings-font-size")
        app.backNSH()
        app.waitNSH("settings-open-appearance")
    }

    /// Picking a theme selects it on the gallery, and Terminal's Theme row
    /// says its name when the gallery is left.
    @MainActor
    func testPickingAThemeSelectsItAndTheRowSaysSo() throws {
        let app = NSHFlow.launch()
        app.openSettings()
        app.openSettingsPart("terminal")
        app.tapNSH("settings-theme")
        app.waitNSH("theme")

        app.tapNSH("theme-row-dracula")
        let row = app.nsh("theme-row-dracula")
        XCTAssertTrue(row.isSelected || row.isOn == true,
                      "the picked theme is the one checked (\(row.spoken))")
        // Picking keeps the gallery up: comparing themes is what it is for.
        XCTAssertTrue(app.nsh("theme").exists, "picking a theme left the gallery")

        app.backNSH()
        let theme = app.waitNSH("settings-theme")
        XCTAssertTrue(theme.spoken.contains("Dracula"),
                      "Terminal's Theme row should name the picked theme, not \(theme.spoken)")
    }

    /// An accent picked on Appearance's gallery keeps the gallery up and
    /// becomes the Accent row's value.
    @MainActor
    func testPickingAnAccentKeepsTheGalleryAndTheRowSaysSo() throws {
        let app = NSHFlow.launch()
        app.openSettings()
        app.openSettingsPart("appearance")
        let before = app.waitNSH("settings-accent").spoken
        XCTAssertTrue(before.contains("Signal Room"), "a reset app's accent is Signal Room, not \(before)")

        app.tapNSH("settings-accent")
        app.waitNSH("accent")
        app.tapNSH("accent-row-terminal-green")
        XCTAssertTrue(app.nsh("accent").exists, "picking an accent left the gallery")

        app.backNSH()
        let after = app.waitNSH("settings-accent").spoken
        XCTAssertTrue(after.contains("Terminal Green"), "the Accent row should say Terminal Green, not \(after)")
    }

    /// The App Icon page offers the nine icons, and picking one does not
    /// touch the accent. The simulator cannot switch icons (lsd's
    /// confirmation plugin fails); a device shows iOS's alert, dismissed
    /// here so the run doesn't stall.
    @MainActor
    func testIconGalleryOffersTheNineIcons() throws {
        let app = NSHFlow.launch()
        app.openSettings()
        app.openSettingsPart("appearance")
        app.tapNSH("settings-app-icon")
        app.waitNSH("app-icon")

        for id in ["default", "pit", "island", "wq", "prefix", "localhost", "prompt", "cursor", "hail"] {
            XCTAssertTrue(app.nsh("app-icon-\(id)").waitForExistence(timeout: 3), "missing icon \(id)")
        }
        app.tapNSH("app-icon-hail")
        let alert = app.alerts.firstMatch
        if alert.waitForExistence(timeout: 3) {
            alert.buttons.element(boundBy: alert.buttons.count - 1).tap()
        }

        app.backNSH()
        let accent = app.waitNSH("settings-accent").spoken
        XCTAssertTrue(accent.contains("Signal Room"), "picking an icon changed the accent: \(accent)")
    }

    /// A switch on Terminal (cursor blink) flips, and the flip is what the
    /// next launch reads: the press reached the setting the app stores,
    /// not only the switch's own drawing.
    @MainActor
    func testASwitchIsStillFlippedAfterARelaunch() throws {
        var app = NSHFlow.launch()
        app.openSettings()
        app.openSettingsPart("terminal")
        let blink = app.waitNSH("settings-cursor-blink")
        let start = try XCTUnwrap(blink.isOn, "the switch reports no state: \(blink.spoken)")
        app.tapNSH("settings-cursor-blink")
        XCTAssertEqual(app.nsh("settings-cursor-blink").isOn, !start, "the tap did not flip the switch")

        app.terminate()
        app = NSHFlow.launch(reset: false)
        app.openSettings()
        app.openSettingsPart("terminal")
        XCTAssertEqual(app.waitNSH("settings-cursor-blink").isOn, !start, "the flip did not outlive a relaunch")

        // Put it back for the next flow.
        app.tapNSH("settings-cursor-blink")
        XCTAssertEqual(app.nsh("settings-cursor-blink").isOn, start)
    }

    /// Bought in the mainland store, Settings carries the filing number;
    /// bought anywhere else, it doesn't. The version row proves the hub's
    /// foot rendered before the second asserts what is missing from it.
    @MainActor
    func testFilingNumberOnlyForTheMainlandStorefront() throws {
        var app = NSHFlow.launch([MainlandStorefrontArgs.mainland, MainlandStorefrontArgs.code])
        app.openSettings()
        let icp = app.nsh("settings-icp")
        XCTAssertTrue(icp.waitForExistence(timeout: NSHFlow.timeout), "the mainland storefront's Settings has no filing number")
        app.terminate()

        app = NSHFlow.launch([MainlandStorefrontArgs.mainland, "USA"])
        app.openSettings()
        let version = app.waitNSH("settings-version")
        app.reveal(version)
        XCTAssertFalse(app.nsh("settings-icp").waitForExistence(timeout: 2),
                       "the filing number shows outside the mainland storefront")
    }

    /// The font size's stepper changes the size the row says, by a point.
    @MainActor
    func testFontSizeStepsByAPoint() throws {
        let app = NSHFlow.launch()
        app.openSettings()
        app.openSettingsPart("terminal")
        let size = { () -> Int? in
            let words = app.nsh("settings-font-size").spoken
            return words.split(whereSeparator: { !$0.isNumber }).compactMap { Int($0) }.first
        }
        app.waitNSH("settings-font-size")
        let before = try XCTUnwrap(size(), "no size in \(app.nsh("settings-font-size").spoken)")

        app.tapNSH("settings-font-larger")
        XCTAssertEqual(size(), before + 1)
        app.tapNSH("settings-font-smaller")
        XCTAssertEqual(size(), before)
    }
}
