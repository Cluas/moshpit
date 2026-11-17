import XCTest

/// Users' reports, replayed on a live host (E2E-3): what each report saw is
/// what the scenario checks, on the parts only the app shows (the split
/// view's geometry, the edit menu and the pasteboard, a tap on a link, the
/// reconnect cover, the page a dead tmux leaves).
///
/// Each one is red on the commit before its fix: CI's `app_ref` dispatch
/// (.github/workflows/ci.yml) builds the app as it was at that commit with
/// these tests and this staging, so the scenario is seen failing there and
/// passing after, with no branch for the old commit.
///
/// The host is the one CI stages (scripts/shots/stage-host.sh), handed over
/// as `TEST_RUNNER_MOSHPIT_SEED_*`; without it every scenario skips. The
/// scenarios that kill tmux do it on a server of their own
/// (`MOSHPIT_SEED_TMUX_BIN_E2E`, no server until a scenario makes one), so
/// the staged "work" every other flow opens is never touched; B841's pane
/// is a third (`_LINK`).
final class ReportsUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    private static var env: [String: String] { ProcessInfo.processInfo.environment }

    @MainActor private static var phone: Bool { UIDevice.current.userInterfaceIdiom == .phone }

    /// The seeded session, its tmux the staged wrapper `wrapper` names
    /// (`MOSHPIT_SEED_TMUX_BIN` for "work"), at the terminal's default size.
    private static func session(tmux wrapper: String = "MOSHPIT_SEED_TMUX_BIN") throws -> [String] {
        guard var args = NSHFlow.seededSession(), let bin = env[wrapper] else {
            throw XCTSkip("needs the staged host: TEST_RUNNER_MOSHPIT_SEED_* and \(wrapper)")
        }
        if let i = args.firstIndex(of: "-MOSHPIT_SEED_TMUX_BIN") {
            args[i + 1] = bin
        } else {
            args += ["-MOSHPIT_SEED_TMUX_BIN", bin]
        }
        return args + ["-moshpit.settings.fontSize", "\(Int(TerminalCells.defaultSize))"]
    }

    // MARK: B843

    /// B843 (iPad, landscape): the terminal beside Home's sidebar is laid
    /// out in its own column and shows its pane. Before e2e54adb the
    /// terminal's page was the window's whole width, under the sidebar: its
    /// crumb and the start of every row behind it, the pane black in the
    /// store shot.
    @MainActor
    func testB843TheTerminalBesideTheSidebarShowsItsPane() throws {
        guard !Self.phone else { throw XCTSkip("iPad only: the split view") }
        let session = try Self.session()
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = NSHFlow.launch(session + ["-MOSHPIT_SEED_HOME", "1"])
        let name = Self.env["MOSHPIT_SEED_NAME"] ?? "devbox"
        // Home connects the saved card in place; its head then opens the
        // terminal (NSHSessionFlowUITest's path).
        let head = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", name)).firstMatch
        XCTAssertTrue(head.waitForExistence(timeout: 30), "Home has no card for \(name)")
        if !app.nsh("home-card-\(name)").exists { head.tap() }
        XCTAssertTrue(app.nsh("home-card-\(name)").waitForExistence(timeout: 60), "\(name) never connected")
        head.tap()
        try waitLive(app)
        // The sidebar back beside it, if opening the terminal hid it.
        if !app.nsh("home").exists || app.nsh("home").frame.width < 1 { app.tapNSH("terminal-sidebar") }
        let sidebar = app.nsh("home")
        XCTAssertTrue(sidebar.waitForExistence(timeout: 10), "no sidebar beside the terminal")
        settle(1.5)

        // The claude pane's prompt row, read off the terminal's rows.
        let row = app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'payments-api'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 15), "the terminal shows none of its pane's text")
        let column = sidebar.frame, rows = row.frame, crumb = app.nsh("terminal-crumb-window").frame
        // A sidebar folded away (its frame off the screen) would let any
        // terminal pass.
        XCTAssertTrue(column.minX >= -1 && column.width > 200 && column.maxX < app.frame.width * 0.6,
                      "the sidebar isn't on screen beside the terminal: \(column) in \(app.frame)")
        XCTAssertGreaterThanOrEqual(rows.minX, column.maxX - 1,
                                    "B843: the pane's rows start at \(rows.minX), under the sidebar (\(column))")
        XCTAssertGreaterThan(crumb.minX, column.maxX,
                             "B843: the terminal's crumb \(crumb) is under the sidebar \(column)")
        XCTAssertLessThanOrEqual(rows.maxX, app.frame.maxX + 1, "the pane's rows run off the screen: \(rows)")
    }

    // MARK: B841

    /// The address Claude Code broke over two rows (stage/link-turn.sh).
    private static let brokenURL =
        "https://example.com/docx/ZqLmOFywpjhAb3dEfGh/edit?usp=sharing&ouid=112233445566778899&rtpof=true&sd=true"

    /// B841: a drag from an address Claude Code broke over two rows to past
    /// its end, then Copy, gives the address whole: no break, no indent.
    /// Before 92929f47 the copy kept the break and the indent the cursor
    /// jumped ("…usp=shar\n  ing&ouid=…"), and the pasted link went
    /// nowhere.
    @MainActor
    func testB841CopyingAnAddressBrokenOverRowsGivesItWhole() throws {
        guard Self.phone else { throw XCTSkip("iPhone: the edit menu and the paste read-back") }
        let app = NSHFlow.launch(try Self.session(tmux: "MOSHPIT_SEED_TMUX_BIN_LINK"))
        try waitLive(app)
        let first = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH '⏺ https://example.com/docx/'")).firstMatch
        let second = app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'ing&ouid='")).firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 15), "the link's first row isn't on the screen")
        XCTAssertTrue(second.waitForExistence(timeout: 5), "the link's second row isn't on the screen")
        let firstLabel = first.label, secondLabel = second.label
        XCTAssertTrue(secondLabel.hasPrefix("  ing&ouid="),
                      "the address isn't broken where Claude Code breaks it: \(firstLabel) / \(secondLabel)")

        // From the address's first character ("⏺ " before it) to a column
        // past the end of its second row.
        let (startX, columns) = TerminalCells.x(ofColumn: 2.5, in: first, of: app)
        let (endX, _) = TerminalCells.x(ofColumn: CGFloat(secondLabel.count) + 1.5, in: second, of: app)
        XCTContext.runActivity(named: "Drag from column 2 of \(columns) to past column \(secondLabel.count)") { _ in
            first.coordinate(withNormalizedOffset: CGVector(dx: startX, dy: 0.5))
                .press(forDuration: 0.8, thenDragTo: second.coordinate(withNormalizedOffset: CGVector(dx: endX, dy: 0.5)))
        }
        let copy = app.menuItems["Copy"]
        XCTAssertTrue(copy.waitForExistence(timeout: 5), "no Copy after the drag")
        copy.tap()
        XCTAssertEqual(pasted(app), Self.brokenURL, "B841: the copy isn't the address whole")
    }

    /// What the pasteboard holds, read back through the app's own New
    /// Session prompt (the test runner reading UIPasteboard would raise
    /// the system's paste prompt): pasted into its name field, read, and
    /// the prompt cancelled.
    @MainActor
    private func pasted(_ app: XCUIApplication) -> String? {
        app.tapNSH("terminal-crumb-session")
        app.tapNSH("mux-picker-new")
        let field = app.nsh("nsh-prompt-field")
        guard field.waitForExistence(timeout: 10) else {
            XCTFail("the New Session prompt never came")
            return nil
        }
        field.tap()
        // The edit menu can come and go while the field settles. And the
        // Paste over the alert's field has no activation point XCTest will
        // accept: an element tap on it reads the alert as an interruption and
        // its handler cancels our prompt (CI, 4f70e8bf). So the tap goes to
        // the item's frame, once the item has stayed.
        let paste = app.menuItems["Paste"]
        var pastedIn = false
        for _ in 0 ..< 3 {
            if paste.waitForExistence(timeout: 3) {
                settle(0.3)
                let frame = paste.frame
                if paste.exists, !frame.isEmpty {
                    app.coordinate(withNormalizedOffset: .zero)
                        .withOffset(CGVector(dx: frame.midX, dy: frame.midY)).tap()
                    pastedIn = true
                    break
                }
            }
            field.tap()
        }
        guard pastedIn else {
            XCTFail("the prompt's field offers no Paste")
            return nil
        }
        settle(0.5)
        let text = field.value as? String
        app.alerts.buttons["Cancel"].firstMatch.tap()
        return text
    }

    // MARK: B817

    /// B817: a tap on a path an agent reported opens its preview. The pane
    /// is work's claude ("Wrote bench/latency.png", the chart staged in the
    /// repo); the tap lands on the path's own cells.
    @MainActor
    func testB817ATapOnAReportedPathOpensItsPreview() throws {
        guard Self.phone else { throw XCTSkip("iPhone: the preview's sheet") }
        let app = NSHFlow.launch(try Self.session())
        try waitLive(app)
        // The path is a link once the host's probe confirmed it.
        settle(4)
        let row = app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'bench/latency.png'")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 15), "the pane doesn't show the path")
        let path = (row.label as NSString).range(of: "bench/latency.png")
        let (x, columns) = TerminalCells.middle(ofColumns: path.location, path.length, in: row, of: app)
        XCTContext.runActivity(named: "Tap the path, columns \(path.location)–\(path.location + path.length - 1) of \(columns), at x \(String(format: "%.3f", x))") { _ in
            row.coordinate(withNormalizedOffset: CGVector(dx: x, dy: 0.5)).tap()
        }
        let preview = app.descendants(matching: .any).matching(NSPredicate(
            format: "label == 'latency.png' OR identifier == 'QLPreviewControllerView'")).firstMatch
        XCTAssertTrue(preview.waitForExistence(timeout: 20), "B817: the tap on the path opened no preview")
    }

    // MARK: B786, B855 / B788 (a tmux of their own)

    /// B786: the tmux server killed under an open pane. Over SSH the page
    /// becomes the empty state (no sessions, New Session) within seconds,
    /// not the last frame of a server that's gone. Before 203b6b5 the
    /// control channel never closed (`tmux -CC attach; exec $SHELL -l`),
    /// %exit only read as "not attached", and the page stayed on the dead
    /// tree.
    @MainActor
    func testB786AServerKilledUnderAnOpenPaneLeavesTheEmptyState() throws {
        let tmux = try XCTUnwrap(Self.env["MOSHPIT_SEED_TMUX_BIN_E2E"], "no MOSHPIT_SEED_TMUX_BIN_E2E")
        let app = NSHFlow.launch(try Self.session(tmux: "MOSHPIT_SEED_TMUX_BIN_E2E"))
        let marker = try shell(app)
        command(app, "\(tmux) kill-server")
        XCTAssertTrue(app.nsh("terminal-empty-state").waitForExistence(timeout: 15),
                      "B786: 15 s after kill-server the page isn't the empty state")
        XCTAssertFalse(app.staticTexts[marker].exists, "B786: the dead pane's last frame (\(marker)) is still up")
    }

    /// B855 (and B788): New Session from the empty state a killed server
    /// left. It lands in the new session, and the reconnect cover (its veil
    /// or its capsule) never shows on the way: looked for about every
    /// 100 ms from the tap until 5 s after it landed, as a flash lasts a
    /// redial. Before 029a3186 the first session's exec made the leg drop
    /// to reattach (Connected, Reconnecting, Connected: the flash); before
    /// bad23e33 the exec ran a bare `tmux` and made nothing (B788).
    @MainActor
    func testB855NewSessionFromTheEmptyStateNeverShowsTheReconnect() throws {
        let tmux = try XCTUnwrap(Self.env["MOSHPIT_SEED_TMUX_BIN_E2E"], "no MOSHPIT_SEED_TMUX_BIN_E2E")
        let app = NSHFlow.launch(try Self.session(tmux: "MOSHPIT_SEED_TMUX_BIN_E2E"))
        // The empty state: a session left by another scenario is killed
        // under the page first, as the user's was.
        if !emptyOrLive(app) {
            _ = try shell(app)
            command(app, "\(tmux) kill-server")
        }
        let create = app.nsh("multiplexer-create-first-session")
        XCTAssertTrue(create.waitForExistence(timeout: 20), "no New Session on the empty state")
        create.tap()

        let cover = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier IN %@", ["terminal-reconnect-veil", "terminal-reconnect-capsule"])).firstMatch
        let start = Date()
        var seen: [String] = []
        var landed: Date?
        while Date() < (landed.map { $0.addingTimeInterval(5) } ?? start.addingTimeInterval(30)) {
            if cover.exists {
                seen.append(String(format: "%@ at %.1f s", cover.identifier, Date().timeIntervalSince(start)))
            }
            if landed == nil, inSession(app) { landed = Date() }
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTAssertNotNil(landed, "B788: New Session never landed in a session (30 s)")
        XCTAssertTrue(seen.isEmpty, "B855: the reconnect cover showed after New Session: \(seen.prefix(5))")
    }

    // MARK: Helpers

    /// Lets the page's own animation finish once its id is there.
    private func settle(_ seconds: TimeInterval = 1.5) {
        Thread.sleep(forTimeInterval: seconds)
    }

    @MainActor
    private func waitEnabled(_ element: XCUIElement, _ timeout: TimeInterval) -> Bool {
        let enabled = expectation(for: NSPredicate(format: "isEnabled == true"), evaluatedWith: element)
        return XCTWaiter().wait(for: [enabled], timeout: timeout) == .completed
    }

    /// The terminal live with nothing over it (ShotsUITest's wait): the
    /// window segment answers, and the reconnect veil isn't there.
    @MainActor
    private func waitLive(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) throws {
        let window = app.nsh("terminal-crumb-window")
        guard window.waitForExistence(timeout: 60), waitEnabled(window, 30) else {
            XCTFail("the seeded session never went live", file: file, line: line)
            throw XCTSkip("not live")
        }
        XCTAssertTrue(app.nsh("terminal-reconnect-veil").waitForNonExistence(timeout: 20),
                      "terminal-reconnect-veil is still up", file: file, line: line)
        settle(0.8)
    }

    /// In a session: the empty state gone and the window segment answering.
    @MainActor
    private func inSession(_ app: XCUIApplication) -> Bool {
        !app.nsh("terminal-empty-state").exists && app.nsh("terminal-crumb-window").isEnabled
    }

    /// Waits until the page is either the empty state (true) or a live
    /// session (false).
    @MainActor
    private func emptyOrLive(_ app: XCUIApplication) -> Bool {
        let end = Date().addingTimeInterval(60)
        while Date() < end {
            if app.nsh("terminal-empty-state").exists { return true }
            if app.nsh("terminal-crumb-window").exists, inSession(app) { return false }
            Thread.sleep(forTimeInterval: 0.25)
        }
        XCTFail("the page is neither a session nor the empty state after 60 s")
        return false
    }

    /// A live pane running a shell, on the seeded tmux: the session there
    /// is, or the one the empty state's New Session makes (tmux starts its
    /// default shell in it). An echo of a marker proves the shell answers;
    /// the marker's row is what a stale frame would keep.
    @MainActor
    private func shell(_ app: XCUIApplication) throws -> String {
        if emptyOrLive(app) {
            app.tapNSH("multiplexer-create-first-session", timeout: 20)
            let end = Date().addingTimeInterval(30)
            while Date() < end, !inSession(app) { Thread.sleep(forTimeInterval: 0.25) }
            XCTAssertTrue(inSession(app), "New Session made no session to type in")
        }
        try waitLive(app)
        app.tapNSH("keyboard-toggle")
        _ = app.keyboards.firstMatch.waitForExistence(timeout: 15)
        let marker = "e2e-\(UUID().uuidString.prefix(8).lowercased())"
        command(app, "echo \(marker)")
        XCTAssertTrue(app.staticTexts[marker].waitForExistence(timeout: 20),
                      "the pane's shell never answered (no row reading \(marker))")
        return marker
    }

    /// Types a command and Return into the terminal (a Return on its own:
    /// a line inserted with its newline reads as a paste).
    @MainActor
    private func command(_ app: XCUIApplication, _ text: String) {
        app.typeText(text)
        app.typeText("\n")
    }
}
