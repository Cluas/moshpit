import Foundation
import Testing
@testable import Moshpit

/// `HerdrControlClient.agentNames()` reads whatever `herdr server
/// agent-manifests --json` printed, login noise and all, and must never trap
/// on it: it sliced from the first `[` to the last `]`, and a reply whose
/// last `]` came before its first `[` took the app down as the New agent
/// task form opened (crates/herdr divergence `manifest-range-trap`).
@Suite("herdr agent manifests")
@MainActor
struct HerdrManifestTests {

    final class Runner: HerdrCommandRunner, @unchecked Sendable {
        private let lock = NSLock()
        private var _reply = ""
        func set(_ reply: String) { lock.withLock { _reply = reply } }
        func run(_ command: String) async throws -> String { lock.withLock { _reply } }
    }

    private func names(_ reply: String) async -> [String] {
        let runner = Runner()
        runner.set(reply)
        return await HerdrControlClient(runner: runner, customPath: nil).agentNames()
    }

    @Test("A closing bracket before the opening one is no list, not a crash")
    func bracketsOutOfOrder() async {
        #expect(await names("a]b[") == [])
        #expect(await names("] [{\"agent\":\"claude\"}") == [])
        #expect(await names("motd: [ok]\n]\n[") == [])
    }

    @Test("The list is still read out of the noise around it")
    func listInNoise() async {
        let list = #"[{"agent":"pi","active_version":"1"},{"agent":"claude"},{"agent":"codex"}]"#
        #expect(await names(list) == ["claude", "codex", "pi"])
        #expect(await names("Last login: today\n" + list + "\nbye") == ["claude", "codex", "pi"])
        #expect(await names("") == [])
        #expect(await names("[") == [])
        #expect(await names("]") == [])
        #expect(await names("[]") == [])
        #expect(await names("noise [1,2] tail") == [])
    }

    @Test("No arrangement of brackets and noise traps")
    func anyShape() async {
        let pieces = ["[", "]", "{", "}", "\"agent\"", ":", ",", "\"x\"", "a", " ", "\n", "\u{301}", "[\u{301}", "]\u{301}"]
        let runner = Runner()
        let client = HerdrControlClient(runner: runner, customPath: nil)
        var state: UInt64 = 0x00AE_5019
        func next(_ bound: Int) -> Int {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Int((state >> 33) % UInt64(bound))
        }
        for _ in 0..<2_000 {
            let reply = (0..<next(10)).map { _ in pieces[next(pieces.count)] }.joined()
            runner.set(reply)
            _ = await client.agentNames()
        }
    }
}
