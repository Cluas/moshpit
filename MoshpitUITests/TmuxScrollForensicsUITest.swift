import XCTest

/// Forensics driver, not a regression test — reproduces the reported
/// "scroll up/down a few times and the screen overlaps into garble" on a
/// REAL device against the user's REAL saved connection, with evidence on
/// both ends:
///   - visual: a screenshot attachment per phase (exported from the
///     .xcresult afterwards),
///   - byte-level: `-MOSHPIT_CC_TAP documents` makes the app record the raw
///     -CC stream, every command↔response pairing, and every frame decision
///     (FEED-HELD / FRAME / FRAME-REJECT) into its Documents directory,
///     pulled afterwards via `devicectl device copy from`.
///
/// It only ever taps the connection card and swipes the terminal — it never
/// types, so nothing reaches the remote shell beyond the wheel events the
/// user's own gesture would send.
final class TmuxScrollForensicsUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = true   // forensics: keep collecting evidence
    }

    @MainActor
    func testScrollUpDownForensics() throws {
        let app = XCUIApplication()
        app.launchArguments += ["-MOSHPIT_CC_TAP", "documents"]
        app.launch()

        // First saved connection card (the device's real config — no reset).
        let card = app.descendants(matching: .any)
            .matching(NSPredicate(format: "identifier BEGINSWITH 'connection-card-'"))
            .firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 15), "no saved connection on this device")
        shot(app, "01-home")
        card.tap()   // offline → connect; live → enter

        // Give attach time; the card shows the tmux tree once live. A second
        // tap on the head honors the chevron and enters the active pane.
        sleep(8)
        shot(app, "02-after-connect")
        if card.exists { card.tap() }
        sleep(6)     // terminal push + first resync
        shot(app, "03-terminal")

        // The reported gesture: scroll up (older) and back down, repeatedly.
        // Drags stay in the middle band of the screen, clear of the status
        // bar, keyboard accessory, and home indicator.
        let window = app.windows.firstMatch
        func drag(fromY: CGFloat, toY: CGFloat) {
            let from = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: fromY))
            let to = window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: toY))
            from.press(forDuration: 0.05, thenDragTo: to,
                       withVelocity: .default, thenHoldForDuration: 0.05)
        }
        for cycle in 1...3 {
            for _ in 1...3 { drag(fromY: 0.35, toY: 0.65) }   // finger down = older
            shot(app, String(format: "%02d-cycle%d-up", 3 + cycle * 2 - 1, cycle))
            for _ in 1...3 { drag(fromY: 0.65, toY: 0.35) }   // finger up = newer
            shot(app, String(format: "%02d-cycle%d-down", 3 + cycle * 2, cycle))
            sleep(1)
        }

        // The reported shape (2026-08-19): "scrolling down is fine at first,
        // KEEP scrolling and the overlap appears" — a sustained same-direction
        // run without pauses, deep enough to cross several repaint cycles.
        for burst in 1...4 {
            for _ in 1...6 { drag(fromY: 0.65, toY: 0.35) }   // sustained newer
            shot(app, String(format: "10-sustained-down%d", burst))
        }
        for burst in 1...2 {
            for _ in 1...6 { drag(fromY: 0.35, toY: 0.65) }   // sustained older
            shot(app, String(format: "14-sustained-up%d", burst))
        }

        // Let repairs/settles land, then the closing states.
        sleep(4)
        shot(app, "16-final")
    }

    @MainActor
    private func shot(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
