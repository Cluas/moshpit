import XCTest

/// Rig-only, half of a two-device drill (the other half is a driver script
/// launching a second simulator against the same tmux server, see
/// `/tmp/rc/lease-e2e.sh`): this device attaches and owns the window; the
/// driver then brings a second Moshpit device onto it, which takes the window
/// (nobody was typing here). The veil must name that device; tapping it takes
/// the window back and the veil must go. Files under `/tmp/rc/lease-e2e/` are
/// the handshake with the driver.
///
/// Skips unless the rig's seed key is present.
final class SizeLeaseTakeoverUITest: XCTestCase {
    private let handshake = "/tmp/rc/lease-e2e"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testTapOnVeilTakesWindowBack() throws {
        guard let raw = try? String(contentsOfFile: "/tmp/rc/seedkey.b64", encoding: .utf8) else {
            throw XCTSkip("rig not running (no /tmp/rc/seedkey.b64)")
        }
        let key = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        // The rig names the account the key belongs to; `NSUserName()` inside
        // the simulator runner is not reliably the host account.
        let user = (try? String(contentsOfFile: "/tmp/rc/seeduser", encoding: .utf8))?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? NSUserName()
        let app = XCUIApplication()
        app.launchArguments = [
            "-MOSHPIT_RESET",
            "-MOSHPIT_SEED_USER", user,
            "-MOSHPIT_SEED_KEY_B64", key,
            "-MOSHPIT_SEED_HOST", "127.0.0.1",
            "-MOSHPIT_SEED_PORT", "2222",
            "-MOSHPIT_SEED_NAME", "rc-lab",
            "-MOSHPIT_SEED_MUX", "tmux",
            "-MOSHPIT_SEED_TMUX_BIN", "/tmp/rc/tmux",
            "-MOSHPIT_SEED_QUIET", "1",
            "-MOSHPIT_SEED_TRUST_FP", "SHA256:qGbLaODy9ZRyfBIDjh6TVeU+Il5Nt+iqk37vKss29Ks",
        ]
        app.launch()
        // Live = the window breadcrumb is up ("1:alpha"; the Home tree's rows
        // read "1: alpha" with a space, which the pattern excludes).
        let crumb = app.buttons.matching(NSPredicate(format: "label MATCHES '^[0-9]+:[^ ].*'")).firstMatch
        XCTAssertTrue(crumb.waitForExistence(timeout: 90), "seed connect")
        signal("live")

        // The driver launches the other device now; it takes the window and
        // our pane goes under the follower veil (one combined button).
        let veil = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'take over'")).firstMatch
        XCTAssertTrue(veil.waitForExistence(timeout: 240), "the other device should have taken the window")
        XCTAssertTrue(veil.label.uppercased().contains("FOLLOWING IPHONE"),
                      "the veil names the holder, got: \(veil.label)")
        signal("veiled")

        veil.tap()
        let gone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: veil)
        XCTAssertEqual(XCTWaiter().wait(for: [gone], timeout: 30), .completed, "the tap takes the window back")
        signal("took")

        // Stay alive while the driver checks the other device is following us.
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'take over'"))
            .firstMatch.waitForExistence(timeout: 25), "we hold the window; no veil comes back on its own")
        signal("done")
    }

    private func signal(_ name: String) {
        try? FileManager.default.createDirectory(atPath: handshake, withIntermediateDirectories: true)
        try? Date().description.write(toFile: "\(handshake)/\(name)", atomically: true, encoding: .utf8)
    }
}
