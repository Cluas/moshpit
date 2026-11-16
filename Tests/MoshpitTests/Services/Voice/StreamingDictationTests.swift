import Foundation
import Testing
@testable import Moshpit

/// A dictation on a streaming recognizer: partial text while listening, the
/// hand-off to the final text, cancelling mid-utterance (B763).
@Suite("Streaming dictation")
struct StreamingDictationTests {
    /// A recognizer that hears "n samples" and records what happened to it.
    final class FakeStream: StreamingRecognizing, @unchecked Sendable {
        private let lock = NSLock()
        private(set) var fed: [Float] = []
        private(set) var events: [String] = []
        let finalText: String

        init(finalText: String = "streamed text") { self.finalText = finalText }

        var closed: Bool { lock.withLock { events.contains("close") } }

        func feed(_ samples: [Float]) { lock.withLock { fed += samples } }
        func partialText() async -> String { lock.withLock { fed.isEmpty ? "" : "heard \(fed.count)" } }
        func finish() async -> String { lock.withLock { events.append("finish"); return finalText } }
        func close() { lock.withLock { events.append("close") } }
        func note(_ event: String) { lock.withLock { events.append(event) } }
        var log: [String] { lock.withLock { events } }
        var heard: [Float] { lock.withLock { fed } }
    }

    /// What the core showed while listening.
    final class Shown: @unchecked Sendable {
        private let lock = NSLock()
        private var texts: [String] = []
        func add(_ text: String) { lock.withLock { texts.append(text) } }
        var all: [String] { lock.withLock { texts } }
    }

    /// Distinct samples, so order can be checked.
    static func chunk(_ index: Int, count: Int = 1_600) -> [Float] {
        (0 ..< count).map { Float(index * 10_000 + $0) }
    }

    /// Wait until `condition` holds, up to `seconds`.
    static func eventually(_ seconds: Double = 3, _ condition: () -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return condition()
    }

    @Test("Words show while listening, and every sample reaches the recognizer once, in order")
    func partialsWhileListening() async {
        let fake = FakeStream()
        let shown = Shown()
        let core = StreamingDictationCore(load: { fake }, finalPass: nil, tick: .milliseconds(5)) { shown.add($0) }
        core.begin()
        var expected: [Float] = []
        for index in 0 ..< 6 {
            let samples = Self.chunk(index)
            expected += samples
            core.append(samples)
            try? await Task.sleep(for: .milliseconds(15))
        }
        #expect(await Self.eventually { shown.all.last == "heard \(expected.count)" })
        #expect(fake.heard == expected)
        // Only changes are shown.
        #expect(Set(shown.all).count == shown.all.count)
        #expect(await core.finish() == .text("streamed text"))
        #expect(fake.closed)
    }

    @Test("Audio heard while the model loads is fed in order once it is there")
    func audioWaitsForTheModel() async {
        let fake = FakeStream()
        let core = StreamingDictationCore(load: {
            try? await Task.sleep(for: .milliseconds(200))
            return fake
        }, finalPass: nil, tick: .milliseconds(5)) { _ in }
        core.begin()
        var expected: [Float] = []
        for index in 0 ..< 8 {
            let samples = Self.chunk(index)
            expected += samples
            core.append(samples)
        }
        #expect(await Self.eventually { fake.heard.count == expected.count })
        #expect(fake.heard == expected)
        #expect(await core.finish() == .text("streamed text"))
    }

    @Test("Without a final pass, the streaming model's text is inserted, even when it is still loading")
    func streamingTextIsTheText() async {
        let fake = FakeStream(finalText: "git checkout main")
        let core = StreamingDictationCore(load: {
            try? await Task.sleep(for: .milliseconds(100))
            return fake
        }, finalPass: nil, tick: .milliseconds(5)) { _ in }
        core.begin()
        let samples = Self.chunk(1, count: 16_000)
        core.append(samples)
        // Finish before the load lands: the core waits for the model.
        #expect(await core.finish() == .text("git checkout main"))
        #expect(fake.heard == samples)
        #expect(fake.closed)
    }

    @Test("With a final pass, the streaming text shows first, the model goes, then the final text replaces it")
    func finalPassReplacesTheStream() async {
        let fake = FakeStream(finalText: "cargo test 挂了")
        let shown = Shown()
        let core = StreamingDictationCore(load: { fake }, finalPass: { samples in
            fake.note("final pass \(samples.count)")
            return "cargo test 挂了。"
        }, tick: .milliseconds(5)) { shown.add($0) }
        core.begin()
        core.append(Self.chunk(1, count: 16_000))
        #expect(await Self.eventually { !fake.heard.isEmpty })
        #expect(await core.finish() == .text("cargo test 挂了。"))
        #expect(shown.all.last == "cargo test 挂了")
        // Never both models at once: closed before the final pass started.
        #expect(fake.log == ["finish", "close", "final pass 16000"])
    }

    @Test("A final pass that fails leaves the streaming text to insert")
    func failedFinalPassFallsBack() async {
        struct Broke: Error {}
        let fake = FakeStream(finalText: "打开 AppSettings.swift")
        let core = StreamingDictationCore(load: { fake }, finalPass: { _ in throw Broke() },
                                          tick: .milliseconds(5)) { _ in }
        core.begin()
        core.append(Self.chunk(1, count: 16_000))
        #expect(await Self.eventually { !fake.heard.isEmpty })
        #expect(await core.finish() == .text("打开 AppSettings.swift"))
    }

    @Test("Cancelling mid-utterance lets the model go and shows nothing more")
    func cancelMidUtterance() async {
        let fake = FakeStream()
        let shown = Shown()
        let core = StreamingDictationCore(load: { fake }, finalPass: { _ in "never" },
                                          tick: .milliseconds(5)) { shown.add($0) }
        core.begin()
        core.append(Self.chunk(1))
        #expect(await Self.eventually { !shown.all.isEmpty })
        core.cancel()
        #expect(fake.closed)
        let before = shown.all.count
        core.append(Self.chunk(2))
        try? await Task.sleep(for: .milliseconds(60))
        #expect(shown.all.count == before)
        #expect(await core.finish() == .cancelled)
        #expect(!fake.log.contains("finish"))
    }

    @Test("A model that loads after a cancel is let go at once")
    func lateLoadAfterCancelCloses() async {
        let fake = FakeStream()
        let core = StreamingDictationCore(load: {
            try? await Task.sleep(for: .milliseconds(100))
            return fake
        }, finalPass: nil, tick: .milliseconds(5)) { _ in }
        core.begin()
        core.cancel()
        #expect(await Self.eventually { fake.closed })
        #expect(fake.heard.isEmpty)
    }

    @Test("A final pass doesn't wait for a model still loading, and the late model is let go")
    func finalPassDoesNotWaitForTheStream() async {
        let fake = FakeStream()
        let core = StreamingDictationCore(load: {
            try? await Task.sleep(for: .milliseconds(150))
            return fake
        }, finalPass: { _ in "final" }, tick: .milliseconds(5)) { _ in }
        core.begin()
        core.append(Self.chunk(1, count: 16_000))
        #expect(await core.finish() == .text("final"))
        #expect(await Self.eventually { fake.closed })
        #expect(fake.heard.isEmpty)
    }

    @Test("Under 0.6 s is a mis-tap: nothing to insert")
    func tooShortIsNothing() async {
        let fake = FakeStream()
        let core = StreamingDictationCore(load: { fake }, finalPass: { _ in "never" },
                                          tick: .milliseconds(5)) { _ in }
        core.begin()
        core.append(Self.chunk(1, count: 4_000))
        #expect(await core.finish() == .text(""))
    }

    @Test("A model that won't load fails the dictation alone, and is no loss with a final pass")
    func loadFailure() async {
        struct Missing: Error {}
        let alone = StreamingDictationCore(load: { throw Missing() }, finalPass: nil,
                                           tick: .milliseconds(5)) { _ in }
        alone.begin()
        alone.append(Self.chunk(1, count: 16_000))
        if case .failed = await alone.finish() {} else { Issue.record("expected a failure") }

        let backed = StreamingDictationCore(load: { throw Missing() }, finalPass: { _ in "Qwen3" },
                                            tick: .milliseconds(5)) { _ in }
        backed.begin()
        backed.append(Self.chunk(1, count: 16_000))
        #expect(await backed.finish() == .text("Qwen3"))
    }

    @Test("A model that won't load is said while listening, unless a final pass will write the text (B862)")
    func loadFailureSaidAtOnce() async {
        struct Missing: Error {}
        let said = Shown()
        let alone = StreamingDictationCore(load: { throw Missing() }, finalPass: nil, tick: .milliseconds(5),
                                           onLoadFailure: { said.add($0) }) { _ in }
        alone.begin()
        alone.append(Self.chunk(1, count: 16_000))
        #expect(await Self.eventually { said.all.count == 1 }, "the dictation listened on, for nothing")

        let backed = StreamingDictationCore(load: { throw Missing() }, finalPass: { _ in "Qwen3" },
                                            tick: .milliseconds(5), onLoadFailure: { said.add($0) }) { _ in }
        backed.begin()
        let cancelled = StreamingDictationCore(load: {
            try? await Task.sleep(for: .milliseconds(50))
            throw Missing()
        }, finalPass: nil, tick: .milliseconds(5), onLoadFailure: { said.add($0) }) { _ in }
        cancelled.begin()
        cancelled.cancel()
        try? await Task.sleep(for: .milliseconds(200))
        #expect(said.all.count == 1)
    }
}
