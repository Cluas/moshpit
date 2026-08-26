import Foundation
import Testing
@testable import Moshpit

/// The shell Moshpit installs on other people's machines.
///
/// Two copies of each script exist — the file under `scripts/` and a Swift
/// literal — because the app cannot read a repo file at runtime on a phone. The
/// canonical copy is the one the shell and Go tests execute; if they drift, the
/// thing that ships is not the thing that was tested. These tests are the guard.
@Suite("Host scripts")
struct HostScriptsTests {

    /// #filePath is the compile-time source path, readable from a simulator
    /// because it shares the host filesystem. A device run cannot see the
    /// checkout, so a missing file is skipped rather than failed — the same
    /// guard also runs in `scripts/verify-push-e2e.sh`, which always has it.
    private func repoScript(_ name: String) -> String? {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // Services
            .deletingLastPathComponent()   // MoshpitTests
            .deletingLastPathComponent()   // repo root
        return try? String(contentsOf: root.appendingPathComponent("scripts/\(name)"),
                           encoding: .utf8)
    }

    @Test("the stamp script matches scripts/moshpit-stamp.sh")
    func stampMatchesRepo() {
        guard let onDisk = repoScript("moshpit-stamp.sh") else { return }
        #expect(HostScripts.stamp == onDisk.trimmingCharacters(in: .newlines),
                "run scripts/gen-host-scripts.py")
    }

    @Test("the sender matches scripts/moshpit-push.sh")
    func senderMatchesRepo() {
        guard let onDisk = repoScript("moshpit-push.sh") else { return }
        #expect(HostScripts.sender == onDisk.trimmingCharacters(in: .newlines),
                "run scripts/gen-host-scripts.py")
    }

    @Test("both scripts keep the two properties that make them safe in a hook")
    func hookSafety() {
        for (name, script) in [("stamp", HostScripts.stamp), ("sender", HostScripts.sender)] {
            #expect(script.hasPrefix("#!/bin/sh"), "\(name) lost its shebang")
            // A hook runs inside an agent's turn: a non-zero exit becomes an
            // error the agent shows the user.
            #expect(script.contains("exit 0"), "\(name) must always exit 0")
        }
        // The stamp script must hand off without waiting — a slow network must
        // cost the agent nothing. Both arms detach: the done push directly, the
        // attention push behind its grace sleep.
        #expect(HostScripts.stamp.contains(">/dev/null 2>&1 & )"))
        #expect(HostScripts.stamp.contains(") >/dev/null 2>&1 &"))
        // The grace window is real and overridable for tests.
        #expect(HostScripts.stamp.contains("MOSHPIT_NOTIFY_GRACE"))
        // The grace fork re-checks that the SAME question still stands before
        // sending — state and episode both.
        #expect(HostScripts.stamp.contains(#"[ "$CUR" = "attention|$EPISODE" ] || exit 0"#))
        // An idle reminder on a parked pane is not a question: a Notification
        // arriving on a `done` pane must never stamp attention or push.
        #expect(HostScripts.stamp.contains("waiting for your input"))
        #expect(HostScripts.stamp.contains("done) IDLE=1"))
        // Only the two states worth waking a phone for ever reach the sender.
        #expect(HostScripts.sender.contains("attention|done"))
    }

    @Test("the sender self-test carries the reserved label the app matches on")
    func selfTestLabel() {
        #expect(HostScripts.sender.contains(HostCommands.selfTestAgent))
        #expect(HostCommands.selfTestAgent == "moshpit-selftest")
    }

    @Test("digests are content-addressed, so a change cannot go unnoticed")
    func digests() {
        let stamp = try! #require(HostScripts.digest(of: .stamp))
        let sender = try! #require(HostScripts.digest(of: .sender))
        #expect(stamp != sender)
        #expect(stamp.count == 64)
        #expect(stamp == ContentDigest.of(HostScripts.stamp))
        #expect(HostScripts.digest(of: .pairing(conn: "any")) == nil, "pairing content is per-connection")
        #expect(HostScripts.digest(of: .hooks(agent: "claude")) == nil)
    }

    @Test("the scripts are delivered as files, so nothing needs escaping")
    func noEscapingNeeded() {
        // The flow this replaces required the stamp script to hold NO single
        // quote, because it was embedded inside `sh -c '...'`. Delivered as a
        // file over an exec channel, that constraint is gone — and this test
        // exists so nobody reintroduces it by "fixing" a quote.
        #expect(HostScripts.sender.contains("'"), "the sender uses single quotes freely")
        #expect(!HostScripts.sender.contains(#"'\''"#), "no paste-era escaping should remain")
    }
}
