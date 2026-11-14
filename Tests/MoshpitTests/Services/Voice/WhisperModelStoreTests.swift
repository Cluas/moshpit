import Foundation
import Testing
@testable import Moshpit

/// The two promises `WhisperModelStore` makes about its one model — never two
/// transcriptions on it at once, never two loads of it at once — held with a
/// scripted model in place of hundreds of megabytes of CoreML.
///
/// An actor does not keep either promise by itself: it lets the next call in
/// at every await, and both a transcription and a load are long awaits.
@Suite("Whisper model store: one at a time")
struct WhisperModelTurnTests {

    // MARK: Fakes

    /// The inside of the model: who is transcribing, how many at once, and
    /// each one held there until the test lets it go.
    actor Inside {
        private(set) var count = 0
        private(set) var most = 0
        private(set) var entered: [String] = []
        private var held: [CheckedContinuation<Void, Never>] = []

        func enter(_ label: String) async {
            count += 1
            most = max(most, count)
            entered.append(label)
            // Not cancellable: WhisperKit mid-window doesn't notice either.
            await withCheckedContinuation { held.append($0) }
            count -= 1
        }

        func releaseFirst() {
            guard !held.isEmpty else { return }
            held.removeFirst().resume()
        }

        func releaseAll() {
            let waiting = held
            held.removeAll()
            for waiter in waiting { waiter.resume() }
        }
    }

    /// Answers with the label it was asked under (passed as the language),
    /// after a turn inside.
    final class ScriptedModel: WhisperLoadedModel {
        let inside: Inside

        init(inside: Inside) {
            self.inside = inside
        }

        func transcribe(samples: [Float], language: String?) async throws -> String {
            let label = language ?? ""
            await inside.enter(label)
            return label
        }
    }

    /// Counts loads, and holds each one until the test finishes them.
    actor Loads {
        private(set) var started = 0
        private var pending: [CheckedContinuation<Void, Never>] = []

        func begin() async {
            started += 1
            await withCheckedContinuation { pending.append($0) }
        }

        func finishAll() {
            let waiting = pending
            pending.removeAll()
            for waiter in waiting { waiter.resume() }
        }
    }

    private static func store(inside: Inside, loads: Loads? = nil) -> WhisperModelStore {
        WhisperModelStore(loader: WhisperModelLoader(
            isInstalled: { _ in true },
            load: { _ in
                if let loads { await loads.begin() }
                return ScriptedModel(inside: inside)
            }))
    }

    private static let speech = [Float](repeating: 0, count: WhisperModelStore.minimumSamples)

    /// Polls until `condition` holds or `timeout` passes.
    private func eventually(timeout: Duration = .seconds(2),
                            _ condition: () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if await condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return await condition()
    }

    // MARK: Transcriptions take turns

    @Test("A cancelled preview still decoding and the final pass don't share the model")
    func previewAndFinalTakeTurns() async throws {
        let inside = Inside()
        let store = Self.store(inside: inside)

        let preview = Task { try await store.transcribe(samples: Self.speech, language: "preview", variant: "v") }
        #expect(await eventually { await inside.count == 1 })

        // The mic stops: the preview is cancelled, but it is mid-window and
        // decodes on, while the engine starts its final pass.
        preview.cancel()
        let final = Task { try await store.transcribe(samples: Self.speech, language: "final", variant: "v") }

        // Every chance to get in beside the preview.
        try await Task.sleep(for: .milliseconds(200))
        #expect(await inside.most == 1, "the final pass decoded beside the cancelled preview")
        #expect(await inside.entered == ["preview"])

        await inside.releaseFirst()
        #expect(await eventually { await inside.entered == ["preview", "final"] })
        await inside.releaseAll()
        #expect(try await final.value == "final")
        _ = try? await preview.value
        #expect(await inside.most == 1)
    }

    @Test("A pass cancelled while it waits its turn never runs")
    func cancelledWhileWaitingNeverRuns() async throws {
        let inside = Inside()
        let store = Self.store(inside: inside)

        let first = Task { try await store.transcribe(samples: Self.speech, language: "first", variant: "v") }
        #expect(await eventually { await inside.count == 1 })
        let stale = Task { try await store.transcribe(samples: Self.speech, language: "stale", variant: "v") }
        try await Task.sleep(for: .milliseconds(100))
        stale.cancel()
        let last = Task { try await store.transcribe(samples: Self.speech, language: "last", variant: "v") }
        try await Task.sleep(for: .milliseconds(100))

        await inside.releaseFirst()
        #expect(await eventually { await inside.entered.count == 2 })
        #expect(await inside.entered == ["first", "last"])
        await inside.releaseAll()
        #expect(try await first.value == "first")
        #expect(try await last.value == "last")
        await #expect(throws: CancellationError.self) { try await stale.value }
    }

    // MARK: One load at a time

    @Test("The mic tapped during the post-download load waits for it instead of loading again")
    func loadIsSingleFlight() async throws {
        let inside = Inside()
        let loads = Loads()
        let store = Self.store(inside: inside, loads: loads)

        // The download finished (the weights are "on disk"), so install goes
        // straight to loading the model once.
        let install = Task { try await store.install(variant: "v") { _ in } }
        #expect(await eventually { await loads.started == 1 })

        // The mic key, while that load is still running.
        let session = await store.beginSession()
        let warmUp = Task { try await store.warmUp(variant: "v") }
        try await Task.sleep(for: .milliseconds(200))
        #expect(await loads.started == 1, "a second copy of the model started loading beside the first")

        await loads.finishAll()
        try await install.value
        try await warmUp.value
        #expect(await loads.started == 1)

        // The one load is the model from here on.
        let text = Task { try await store.transcribe(samples: Self.speech, language: "after", variant: "v") }
        #expect(await eventually { await inside.count == 1 })
        await inside.releaseAll()
        #expect(try await text.value == "after")
        #expect(await loads.started == 1)
        await store.endSession(session)
    }

    @Test("A model removed while it loads is not kept once the load lands (B124)")
    func removedWhileLoading() async throws {
        let inside = Inside()
        let loads = Loads()
        let store = Self.store(inside: inside, loads: loads)
        let warmUp = Task { try await store.warmUp(variant: "v") }
        #expect(await eventually { await loads.started == 1 })

        // Settings › Model › Remove, while the model is still loading.
        try await store.remove(variant: "v")
        await loads.finishAll()
        try await warmUp.value

        // Nothing was kept: the next use loads it again.
        let again = Task { try await store.warmUp(variant: "v") }
        #expect(await eventually { await loads.started == 2 })
        await loads.finishAll()
        try await again.value
        // And that one is kept, as any load is.
        try await store.warmUp(variant: "v")
        #expect(await loads.started == 2)
    }
}

/// LAST ERROR on the Model page — the SwiftUI one and the NSH one — is the
/// download centre's most recent failure still standing.
@Suite("Whisper downloads: the last error")
@MainActor
struct WhisperDownloadFailureTests {

    @Test("With several downloads failed, LAST ERROR is the most recent one (B207)")
    func lastErrorIsTheMostRecent() {
        let center = WhisperDownloadCenter()
        let variants = (1...8).map { "v\($0)" }
        for variant in variants { center.recordFailure("\(variant) failed", for: variant) }
        // Retried one by one from the newest: the one that failed before it
        // is the last error then.
        for variant in variants.reversed() {
            #expect(center.lastFailure == "\(variant) failed",
                    "LAST ERROR showed an older failure: \(center.lastFailure ?? "none")")
            center.clearFailure(for: variant)
        }
        #expect(center.lastFailure == nil)
    }

    /// Pause cleared the progress but told no page to read the catalog
    /// again: the row said "Download" until the page was left, with the
    /// bytes kept and Resume nowhere to be seen.
    @Test("Pausing a download has the pages read what is on disk again")
    func pauseRereads() {
        let center = WhisperDownloadCenter()
        let before = center.completions
        center.pause("small")
        #expect(center.completions != before, "no page re-read the catalog after Pause")
        #expect(center.progress["small"] == nil)
    }

    /// Pause clears the row's progress, and the run it stopped goes on until
    /// it notices. The load step at the end is the plain case: its `1`
    /// arrives once the task has been cancelled, and the cancelled task
    /// returns before clearing anything — the row said "downloading" again,
    /// with Pause for its button, for a download that had stopped.
    @Test("A download paused during its last step leaves no progress behind")
    func pauseDuringTheLoadStep() async {
        let gate = Gate()
        let center = WhisperDownloadCenter(install: { _, onProgress in
            onProgress(0.9)
            await gate.wait()
            onProgress(1)
        })
        center.start("small")
        #expect(await settles { center.progress["small"] == 0.9 })

        center.pause("small")
        gate.open()

        #expect(!(await settles { center.progress["small"] != nil }),
                "a paused download shows \(center.progress["small"] ?? 0) again")
    }

    @Test("Paused and started again: the old run's reports are not the new run's")
    func pausedRunDoesNotReportIntoTheNext() async {
        let gate = Gate()
        let second = Gate()
        defer { second.open() }
        let calls = Calls()
        let center = WhisperDownloadCenter(install: { _, onProgress in
            if calls.next() == 1 {
                await gate.wait()
                onProgress(0.8)
            } else {
                onProgress(0.1)
                await second.wait()
            }
        })
        center.start("small")
        center.pause("small")
        center.start("small")
        #expect(await settles { center.progress["small"] == 0.1 })
        gate.open()
        #expect(!(await settles { center.progress["small"] == 0.8 }),
                "the paused run's progress showed under the new one")
    }

    /// Polls for up to half a second.
    private func settles(_ condition: @MainActor () -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .milliseconds(500)
        while ContinuousClock.now < deadline {
            if condition() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return condition()
    }

    final class Gate: @unchecked Sendable {
        private let lock = NSLock()
        private var waiter: CheckedContinuation<Void, Never>?
        private var opened = false
        func wait() async {
            await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                let go = lock.withLock { () -> Bool in
                    if !opened { waiter = continuation }
                    return opened
                }
                if go { continuation.resume() }
            }
        }
        func open() {
            let waiting = lock.withLock { () -> CheckedContinuation<Void, Never>? in
                opened = true
                defer { waiter = nil }
                return waiter
            }
            waiting?.resume()
        }
    }

    final class Calls: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        func next() -> Int { lock.withLock { count += 1; return count } }
    }

    @Test("A download that fails again is the most recent failure again")
    func failingAgainIsTheMostRecent() {
        let center = WhisperDownloadCenter()
        center.recordFailure("offline", for: "small")
        center.recordFailure("disk full", for: "large")
        center.recordFailure("timed out", for: "small")
        #expect(center.lastFailure == "timed out")
        center.clearFailure(for: "small")
        #expect(center.lastFailure == "disk full")
    }
}
