import Foundation
import Testing
@testable import Moshpit

/// The tmux parser switch (``TmuxParserBackend``) and what happens when the
/// Rust parser fails. The parsers themselves are tested by the tmux suites,
/// which run against both (`TmuxParserUnderTest`).
@Suite("tmux parser switch")
struct TmuxParserBackendTests {

    @Test("the switch picks the Rust parser until it has failed once in this process")
    func choice() {
        #expect(TmuxParserBackend.chosen(switchOn: false, rustFailed: false) == .swift)
        #expect(TmuxParserBackend.chosen(switchOn: true, rustFailed: false) == .rust)
        #expect(TmuxParserBackend.chosen(switchOn: true, rustFailed: true) == .swift)
        #expect(TmuxParserBackend.chosen(switchOn: false, rustFailed: true) == .swift)
    }

    @Test("the Rust parser is in this build")
    func rustIsLinked() {
        #expect(TmuxParserBackend.rustAvailable)
    }

    @Test("a client runs the parser it is given, by default the current one")
    func clientsRunTheirParser() async {
        #expect(await TmuxControlClient(backend: .rust).backend == .rust)
        #expect(await TmuxControlClient(backend: .swift).backend == .swift)
        for backend in TmuxParserBackend.allCases {
            await TmuxParserBackend.$taskOverride.withValue(backend) {
                #expect(TmuxParserBackend.current == backend)
                #expect(await TmuxControlClient().backend == backend)
            }
        }
    }

    @Test("a failed Rust parser hands the stream to the Swift parser, says so, and keeps new clients off Rust")
    func failureFallsBackToSwift() async {
        let latch = TmuxParserBackend.Latch()
        let client = TmuxControlClient(backend: .rust)
        let seen = Seen()
        await client.setCallbacks(
            onWindowAdd: { seen.add("add \($0)") },
            onProtocolError: { error in
                if case .malformedLine(let text) = error { seen.add("error \(text)") }
            },
            onParserFailure: { seen.add("failure \($0)") })
        await client.feed(Data("%begin 1 1 0\n%end 1 1 0\n%window-add @1\n%output %0 par".utf8))

        await client.rustParserFailed(reason: "injected", latch: latch)
        #expect(latch.isSet)
        #expect(await client.backend == .swift)
        #expect(TmuxParserBackend.chosen(switchOn: true, rustFailed: latch.isSet) == .swift)

        // The Swift parser starts clean, as after a reset: the Rust parser's
        // partial line is gone and the next block is a boot block again.
        await client.feed(Data("tial\n%begin 2 2 0\n%end 2 2 0\n%window-add @2\n".utf8))
        #expect(seen.all == ["add @1", "error injected", "failure injected", "add @2"])
    }
}

private final class Seen: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String] = []

    var all: [String] {
        lock.lock(); defer { lock.unlock() }
        return items
    }

    func add(_ item: String) {
        lock.lock(); defer { lock.unlock() }
        items.append(item)
    }
}
