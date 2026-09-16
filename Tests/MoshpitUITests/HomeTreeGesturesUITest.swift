import XCTest

/// Rig-only. The Home tree's rows are List rows of their own now, so a swipe
/// or a long-press on a WINDOW row must offer that window's actions, and the
/// connection's own actions must stay on the host head. Before this the whole
/// card was one row: every gesture on it, whichever tree row the finger was
/// on, was the connection's (report, 2026-09-16).
///
/// Needs the local rig (`/tmp/rc`: relay on 127.0.0.1:2222, isolated tmux
/// wrapper with a session whose first window is "alpha", seed key) — skips
/// everywhere else. Screenshots land in `/tmp/rc/home-tree-shots/` so the
/// layout can be eyeballed without a Simulator.app.
final class HomeTreeGesturesUITest: XCTestCase {
    private static let rigKey = "/tmp/rc/seedkey.b64"
    private static let shots = "/tmp/rc/home-tree-shots"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    private func shot(_ app: XCUIApplication, _ name: String) {
        let png = app.screenshot().pngRepresentation
        try? FileManager.default.createDirectory(atPath: Self.shots, withIntermediateDirectories: true)
        try? png.write(to: URL(fileURLWithPath: "\(Self.shots)/\(name).png"))
        let attachment = XCTAttachment(uniformTypeIdentifier: "public.png", name: "\(name).png",
                                       payload: png, userInfo: nil)
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// Seeded onto Home, connected from the card, live.
    @MainActor
    private func launchLive() throws -> XCUIApplication {
        guard let key = try? String(contentsOfFile: Self.rigKey, encoding: .utf8) else {
            throw XCTSkip("needs the local reconnect rig (/tmp/rc)")
        }
        let app = XCUIApplication()
        app.launchArguments += [
            "-MOSHPIT_RESET", "-MOSHPIT_SEED_HOME", "1",
            "-MOSHPIT_SEED_USER", (try? String(contentsOfFile: "/tmp/rc/seeduser", encoding: .utf8))?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? NSUserName(),
            "-MOSHPIT_SEED_KEY_B64", key.trimmingCharacters(in: .whitespacesAndNewlines),
            "-MOSHPIT_SEED_HOST", "127.0.0.1", "-MOSHPIT_SEED_PORT", "2222",
            "-MOSHPIT_SEED_NAME", "rc-lab", "-MOSHPIT_SEED_MUX", "tmux",
            "-MOSHPIT_SEED_TMUX_BIN", "/tmp/rc/tmux", "-MOSHPIT_SEED_QUIET", "1",
            "-MOSHPIT_SEED_TRUST_FP", "SHA256:qGbLaODy9ZRyfBIDjh6TVeU+Il5Nt+iqk37vKss29Ks",
        ]
        app.launch()
        let head = headRow(app)
        XCTAssertTrue(head.waitForExistence(timeout: 15), "home card missing")
        head.tap()   // offline → connect
        XCTAssertTrue(app.staticTexts["LIVE"].waitForExistence(timeout: 30), "never went live")
        return app
    }

    /// The host head — the one row the connection's actions belong to.
    @MainActor
    private func headRow(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'connection-card-'")).firstMatch
    }

    /// The tree row for the rig's first window; Home writes it "1: alpha …"
    /// (the terminal crumb's "1:alpha" has no space).
    @MainActor
    private func windowRow(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH '1: alpha'")).firstMatch
    }

    @MainActor
    func testWindowRowSwipeOffersTheWindowsActions() throws {
        let app = try launchLive()
        let row = windowRow(app)
        XCTAssertTrue(row.waitForExistence(timeout: 20), "window row never appeared")
        shot(app, "01-live-tree")

        row.swipeLeft()
        XCTAssertTrue(app.buttons["Kill Window"].waitForExistence(timeout: 3),
                      "a swipe on a window row offers Kill Window")
        XCTAssertTrue(app.buttons["Rename"].exists, "…and Rename")
        XCTAssertFalse(app.buttons["Delete"].exists,
                       "the connection's Delete must not ride a window row's swipe")
        shot(app, "02-window-swipe")

        // Rename opens Home's screen-sized card — a row can't host the
        // overlay itself (it would clip to the row).
        app.buttons["Rename"].tap()
        shot(app, "02b-after-rename-tap")
        XCTAssertTrue(app.staticTexts["Rename Window"].waitForExistence(timeout: 3),
                      "the rename card should open over the screen")
        shot(app, "03-rename-card")
        app.buttons["Cancel"].tap()
        XCTAssertFalse(app.staticTexts["Rename Window"].waitForExistence(timeout: 2))
    }

    @MainActor
    func testWindowRowLongPressOffersTheWindowsMenu() throws {
        let app = try launchLive()
        let row = windowRow(app)
        XCTAssertTrue(row.waitForExistence(timeout: 20), "window row never appeared")

        let newPane = app.buttons["New Pane"]
        pressForMenu(row, until: newPane, "a long-press on a window row opens the window's menu")
        XCTAssertTrue(app.buttons["Kill Window"].exists)
        XCTAssertFalse(app.buttons["Delete Connection"].exists,
                       "the connection's menu must not answer a window row")
        shot(app, "04-window-menu")
    }

    @MainActor
    func testHeadRowKeepsTheConnectionsActions() throws {
        let app = try launchLive()
        let head = headRow(app)
        head.swipeLeft()
        XCTAssertTrue(app.buttons["Delete"].waitForExistence(timeout: 3),
                      "the head row's swipe is the connection's")
        XCTAssertTrue(app.buttons["Disconnect"].exists)
        XCTAssertTrue(app.buttons["Edit"].exists)
        shot(app, "05-head-swipe")

        // Tapping the open row's content closes the tray instead of opening
        // the terminal. The visible part of the head is its trailing 3/4
        // once slid, so tap there rather than at the (clipped) centre.
        head.coordinate(withNormalizedOffset: CGVector(dx: 0.75, dy: 0.5)).tap()
        let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"),
                                             object: app.buttons["Delete"])
        XCTAssertEqual(XCTWaiter().wait(for: [gone], timeout: 3), .completed, "a tap on the open head closes it")
        XCTAssertTrue(head.waitForExistence(timeout: 2), "and we are still on Home")

        let deleteItem = app.buttons["Delete Connection"]
        pressForMenu(head, until: deleteItem, "the head row's long-press is the connection's menu")
        shot(app, "06-head-menu")
    }

    /// A vertical drag that begins on a tray row must still scroll the list:
    /// the tray's drag is high-priority over the row's own tap, not over the
    /// list. Needs a tree taller than the screen — the driver script
    /// (/tmp/rc/tree-scroll-test.sh) pads the rig session with windows
    /// first; on the plain rig the footer is already on screen and the test
    /// skips.
    @MainActor
    func testVerticalDragOnATrayRowScrollsTheList() throws {
        let app = try launchLive()
        let row = windowRow(app)
        XCTAssertTrue(row.waitForExistence(timeout: 20), "window row never appeared")
        // The tree fills in after LIVE; give the footer a moment to be pushed
        // off-screen before deciding the list is too short to scroll.
        let footer = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'v1.'")).firstMatch
        let pushedOff = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isHittable == false"), object: footer)
        if XCTWaiter().wait(for: [pushedOff], timeout: 4) != .completed {
            throw XCTSkip("tree too short to scroll — run via /tmp/rc/tree-scroll-test.sh")
        }
        let before = row.frame.minY
        // A deliberate finger drag rather than `swipeUp()`: the flick is so
        // quick the row sees it as down/up with no movement.
        let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5))
        let to = from.withOffset(CGVector(dx: 0, dy: -260))
        from.press(forDuration: 0.05, thenDragTo: to, withVelocity: .default, thenHoldForDuration: 0.1)
        sleep(1)
        shot(app, "07-scrolled")
        // Still on Home: the drag was not taken for a tap (the terminal's
        // crumb reads "1:alpha", no space).
        let crumb = app.buttons.matching(NSPredicate(format: "label MATCHES '^[0-9]+:[^ ].*'")).firstMatch
        XCTAssertFalse(crumb.exists, "a vertical drag on a tray row must not open the terminal")
        // Scrolled = the row moved up a good way or went off-screen entirely;
        // one flick need not reach the footer on a long tree.
        XCTAssertTrue(!row.exists || row.frame.minY < before - 120,
                      "a vertical drag starting on a tray row scrolls the list (row y \(before) -> \(row.exists ? row.frame.minY : -1))")
        XCTAssertFalse(app.buttons["Kill Window"].exists, "the vertical drag did not open the tray")
    }
}
