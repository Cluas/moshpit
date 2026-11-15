import Foundation
import Testing
import UIKit
@testable import Moshpit

/// When the local model (Qwen3-ASR, ~1.2 GB resident while decoding; or a
/// Whisper variant) leaves memory (B678). The app holds live terminals; a
/// model left resident after dictation makes a jetsam kill — which takes
/// every live session with it — that much likelier.
@Suite("Local model lifecycle")
struct WhisperModelLifecycleTests {
    actor LoadCount {
        private(set) var loads = 0
        func record() { loads += 1 }
    }

    final class SilentModel: WhisperLoadedModel {
        func transcribe(samples _: [Float], language _: String?, hotwords _: [String]) async throws -> String { "" }
    }

    private static func store(idle: Duration, count: LoadCount) -> WhisperModelStore {
        WhisperModelStore(loader: WhisperModelLoader(
            isInstalled: { _ in true },
            load: { _ in
                await count.record()
                return SilentModel()
            }), idleRelease: idle)
    }

    private func eventually(timeout: Duration = .seconds(2), _ condition: () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if await condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return await condition()
    }

    @Test("The model goes a short idle time after the last dictation ends (B678)")
    func idleAfterDictation() async throws {
        let count = LoadCount()
        let store = Self.store(idle: .milliseconds(60), count: count)
        let session = await store.beginSession()
        try await store.warmUp(variant: "v")
        #expect(await store.loadedVariant == "v")
        await store.endSession(session)
        #expect(await eventually { await store.loadedVariant == nil })
    }

    @Test("Not while a dictation holds it, however long it runs")
    func keptDuringDictation() async throws {
        let count = LoadCount()
        let store = Self.store(idle: .milliseconds(40), count: count)
        let session = await store.beginSession()
        try await store.warmUp(variant: "v")
        try await Task.sleep(for: .milliseconds(200))
        #expect(await store.loadedVariant == "v")
        await store.endSession(session)
    }

    @Test("A second dictation inside the idle window reuses the loaded model")
    func secondDictationReuses() async throws {
        let count = LoadCount()
        let store = Self.store(idle: .milliseconds(300), count: count)
        let first = await store.beginSession()
        try await store.warmUp(variant: "v")
        await store.endSession(first)
        let second = await store.beginSession()
        try await store.warmUp(variant: "v")
        #expect(await count.loads == 1)
        // The idle window that started with the first session's end is over
        // by now; the second session still holds the model.
        try await Task.sleep(for: .milliseconds(450))
        #expect(await store.loadedVariant == "v")
        await store.endSession(second)
        #expect(await eventually { await store.loadedVariant == nil })
    }

    @Test("An install's warm-up load goes too, if no dictation takes it")
    func installLoadGoes() async throws {
        let count = LoadCount()
        let store = Self.store(idle: .milliseconds(60), count: count)
        try await store.warmUp(variant: "v")  // what install does once the bytes are in
        #expect(await store.loadedVariant == "v")
        #expect(await eventually { await store.loadedVariant == nil })
    }

    @MainActor
    @Test("Going to the background unloads the model (B678)")
    func backgroundUnloads() async throws {
        actor Log { private(set) var unloads = 0; func record() { unloads += 1 } }
        let suite = "test.model-lifecycle.\(UUID().uuidString)"
        defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
        let settings = AppSettings(defaults: try #require(UserDefaults(suiteName: suite)))
        settings.voiceInputEnabled = true
        settings.voiceEngine = .whisper
        let center = NotificationCenter()
        let log = Log()
        let residency = WhisperModelResidency(settings: settings, notifications: center) { await log.record() }
        center.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
        #expect(await eventually { await log.unloads == 1 })
        withExtendedLifetime(residency) {}
    }
}
