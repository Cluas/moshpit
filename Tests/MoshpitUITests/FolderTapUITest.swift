import XCTest

/// Rig-only (B302): a real tap on a row of the host folder sheet opens it.
/// The app is launched with `MOSHPIT_T10_ARGS` (the private rig's seed and
/// the director's `sheets` mode showing only the folder sheet, held), which
/// the run's environment carries; skipped without it. `MOSHPIT_T10_DIR` is
/// the folder the pane is in: it holds a folder `out` with `a.png`, and a
/// file `note.txt`.
final class FolderTapUITest: XCTestCase {
    private func shot(_ app: XCUIApplication, _ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot())
        a.name = name; a.lifetime = .keepAlways; add(a)
    }

    private func launch() throws -> (XCUIApplication, String) {
        let env = ProcessInfo.processInfo.environment
        guard let args = env["MOSHPIT_T10_ARGS"], let dir = env["MOSHPIT_T10_DIR"] else {
            throw XCTSkip("needs the private rig (MOSHPIT_T10_ARGS, MOSHPIT_T10_DIR)")
        }
        let app = XCUIApplication()
        app.launchArguments += args.split(separator: " ").map(String.init)
        app.launch()
        return (app, dir)
    }

    private func item(_ app: XCUIApplication, _ path: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: "remote-folder-item-\(path)").firstMatch
    }

    /// A folder's row: a tap pushes its page.
    @MainActor
    func testTappingAFolderRowOpensIt() throws {
        let (app, dir) = try launch()
        let folder = item(app, "\(dir)/out")
        XCTAssertTrue(folder.waitForExistence(timeout: 60), "the folder sheet never listed out/")
        sleep(1)
        shot(app, "folder-sheet")
        folder.tap()
        let inside = item(app, "\(dir)/out/a.png")
        let opened = inside.waitForExistence(timeout: 8)
        shot(app, opened ? "folder-pushed" : "folder-not-pushed")
        XCTAssertTrue(opened, "tapping out/ did not open it")
    }

    /// A file's row: a tap previews it (QuickLook shows its name).
    @MainActor
    func testTappingAFileRowPreviewsIt() throws {
        let (app, dir) = try launch()
        let file = item(app, "\(dir)/note.txt")
        XCTAssertTrue(file.waitForExistence(timeout: 60), "the folder sheet never listed note.txt")
        sleep(1)
        file.tap()
        let preview = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS 'hello from t10' OR value CONTAINS 'hello from t10'")).firstMatch
        let quickLook = app.navigationBars.matching(NSPredicate(format: "identifier CONTAINS 'note'")).firstMatch
        let shown = preview.waitForExistence(timeout: 10) || quickLook.exists
        shot(app, shown ? "file-previewed" : "file-not-previewed")
        XCTAssertTrue(shown, "tapping note.txt did not preview it")
    }
}
