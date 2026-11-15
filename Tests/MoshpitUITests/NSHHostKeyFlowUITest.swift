import XCTest

/// The host key's question on a live host (E6c: its own NSH sheet,
/// `ui/src/host_key.rs`), over the seeded terminal:
///
/// - a key never seen: the sheet shows its fingerprint; Cancel
///   (`host-key-decline`) ends the connect, and the terminal never goes
///   live;
/// - a key that changed (the seed trusts a fingerprint the host doesn't
///   have): the sheet shows the stored one and the one offered
///   (`host-key-stored`, `host-key-offered`); Disconnect ends it;
/// - a key never seen, trusted (`host-key-trust`): the session gets in.
///
/// Needs a host: the one CI stages (`scripts/shots/stage-host.sh`) or the
/// local rig, as `TEST_RUNNER_MOSHPIT_SEED_*`; skips without one. Trust is
/// kept (the app's known hosts), so the trusting case runs last: the cases
/// run in name order.
final class NSHHostKeyFlowUITest: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// The seeded session with no trust answered beforehand, or with
    /// `fingerprint` trusted in place of the host's own.
    private func session(trusting fingerprint: String? = nil) throws -> [String] {
        guard var args = NSHFlow.seededSession() else {
            throw XCTSkip("needs a host: TEST_RUNNER_MOSHPIT_SEED_USER and _KEY_B64 (the local rig, or a staged one)")
        }
        if let i = args.firstIndex(of: "-MOSHPIT_SEED_TRUST_FP") { args.removeSubrange(i...(i + 1)) }
        if let fingerprint { args += ["-MOSHPIT_SEED_TRUST_FP", fingerprint] }
        return args
    }

    /// The terminal's window segment answering: the session is live.
    @MainActor
    private func live(_ app: XCUIApplication, within timeout: TimeInterval) -> Bool {
        let crumb = app.nsh("terminal-crumb-window")
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if crumb.exists && crumb.isEnabled { return true }
            Thread.sleep(forTimeInterval: 0.5)
        } while Date() < deadline
        return false
    }

    @MainActor
    func test1ANewKeyDeclinedEndsTheConnect() throws {
        let app = NSHFlow.launch(try session())
        app.waitNSH("host-key", timeout: 45, "a host never trusted asked nothing")
        XCTAssertTrue(app.nsh("host-key-fingerprint").exists, "the question shows no fingerprint")
        XCTAssertFalse(app.nsh("host-key-stored").exists, "a new key was asked about as a changed one")
        app.tapNSH("host-key-decline")
        XCTAssertTrue(app.nsh("host-key").waitForNonExistence(timeout: 10), "Cancel left the question up")
        XCTAssertFalse(live(app, within: 10), "the session went live after its key was declined")
    }

    @MainActor
    func test2AChangedKeyShowsBothAndDisconnects() throws {
        // A fingerprint the staged host's key never has.
        let stored = "SHA256:" + String(repeating: "A", count: 43)
        let app = NSHFlow.launch(try session(trusting: stored))
        app.waitNSH("host-key", timeout: 45, "a changed key asked nothing")
        let was = app.nsh("host-key-stored")
        XCTAssertTrue(was.exists, "the question doesn't show the stored key")
        XCTAssertTrue(was.label.contains("AAAA") || (was.value as? String ?? "").contains("AAAA"),
                      "the stored key isn't the one trusted: \(was.label)")
        XCTAssertTrue(app.nsh("host-key-offered").exists, "the question doesn't show the key offered")
        app.tapNSH("host-key-decline")
        XCTAssertTrue(app.nsh("host-key").waitForNonExistence(timeout: 10), "Disconnect left the question up")
        XCTAssertFalse(live(app, within: 10), "the session went live on a changed key")
    }

    @MainActor
    func test3ANewKeyTrustedGetsIn() throws {
        let app = NSHFlow.launch(try session())
        app.waitNSH("host-key", timeout: 45, "a host never trusted asked nothing")
        app.tapNSH("host-key-trust")
        XCTAssertTrue(app.nsh("host-key").waitForNonExistence(timeout: 10), "Trust left the question up")
        XCTAssertTrue(live(app, within: 60), "the session never went live after its key was trusted")
    }
}
