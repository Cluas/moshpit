import Foundation
import Testing
import UIKit
@testable import Moshpit

/// When the Whisper model leaves memory. Nothing used to unload it: one
/// Whisper dictation and several hundred megabytes stayed resident until the
/// app exited, whatever the settings said afterwards.
@Suite("Whisper session ledger")
struct WhisperSessionLedgerTests {

    @Test("With no dictation open, an unload is immediate")
    func idleUnloadIsImmediate() {
        var ledger = WhisperSessionLedger()
        let dropNow = ledger.requestUnload()
        #expect(dropNow)
    }

    @Test("An unload during a dictation waits for it to end")
    func unloadWaitsForSession() {
        var ledger = WhisperSessionLedger()
        let session = ledger.open()
        let dropNow = ledger.requestUnload()
        let dropOnClose = ledger.close(session)
        #expect(!dropNow, "the model is in use")
        #expect(dropOnClose, "and goes once it is not")
    }

    @Test("Two sessions open: the model goes when the last one ends")
    func lastSessionDrops() {
        var ledger = WhisperSessionLedger()
        let first = ledger.open()
        let second = ledger.open()
        let dropNow = ledger.requestUnload()
        let dropOnFirst = ledger.close(first)
        let dropOnSecond = ledger.close(second)
        #expect(!dropNow)
        #expect(!dropOnFirst)
        #expect(dropOnSecond)
    }

    @Test("A session ending with no unload asked for keeps the model, and closing twice is harmless")
    func plainCloseKeeps() {
        var ledger = WhisperSessionLedger()
        let session = ledger.open()
        let dropOnClose = ledger.close(session)
        let dropOnSecondClose = ledger.close(session)
        #expect(!dropOnClose)
        #expect(!dropOnSecondClose)
        #expect(ledger.openSessions.isEmpty)
    }

    @Test("Only a real drop moves the generation a load in flight checks")
    func generationMovesOnDrop() {
        var ledger = WhisperSessionLedger()
        let session = ledger.open()
        _ = ledger.requestUnload()
        // Deferred: a session's own load must still be kept.
        #expect(ledger.generation == 0)
        _ = ledger.close(session)
        #expect(ledger.generation == 1)
        _ = ledger.requestUnload()
        #expect(ledger.generation == 2)
    }
}

@Suite("Whisper model residency")
@MainActor
struct WhisperModelResidencyTests {

    /// Counts unloads; an actor because the residency calls in from tasks.
    actor UnloadLog {
        private(set) var unloads = 0
        func record() { unloads += 1 }
    }

    /// Settings on a scratch suite, voice input on and set to Whisper.
    private static func makeSettings(suite: String) throws -> AppSettings {
        let settings = AppSettings(defaults: try #require(UserDefaults(suiteName: suite)))
        settings.voiceInputEnabled = true
        settings.voiceEngine = .whisper
        return settings
    }

    private static func dropSuite(_ suite: String) {
        UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite)
    }

    private func eventually(_ condition: @escaping () async -> Bool) async -> Bool {
        let deadline = ContinuousClock.now + .seconds(2)
        while ContinuousClock.now < deadline {
            if await condition() { return true }
            try? await Task.sleep(for: .milliseconds(5))
        }
        return await condition()
    }

    @Test("Moving dictation to Apple's engine, or switching voice input off, unloads the model")
    func settingsUnload() async throws {
        let suite = "test.whisper-residency.\(UUID().uuidString)"
        defer { Self.dropSuite(suite) }
        let settings = try Self.makeSettings(suite: suite)
        let log = UnloadLog()
        let residency = WhisperModelResidency(settings: settings, notifications: NotificationCenter()) {
            await log.record()
        }

        settings.voiceEngine = .apple
        #expect(await eventually { await log.unloads == 1 })

        // Back to Whisper: nothing to let go of.
        settings.voiceEngine = .whisper
        try await Task.sleep(for: .milliseconds(50))
        #expect(await log.unloads == 1)

        settings.voiceInputEnabled = false
        #expect(await eventually { await log.unloads == 2 })
        withExtendedLifetime(residency) {}
    }

    @Test("Settings that leave Whisper in use unload nothing")
    func unrelatedSettingsKeepModel() async throws {
        let suite = "test.whisper-residency.\(UUID().uuidString)"
        defer { Self.dropSuite(suite) }
        let settings = try Self.makeSettings(suite: suite)
        let log = UnloadLog()
        let residency = WhisperModelResidency(settings: settings, notifications: NotificationCenter()) {
            await log.record()
        }

        settings.whisperLanguage = "zh"
        settings.voiceEngine = .whisper
        try await Task.sleep(for: .milliseconds(50))
        #expect(await log.unloads == 0)
        withExtendedLifetime(residency) {}
    }

    @Test("A memory warning unloads the model")
    func memoryWarningUnloads() async throws {
        let suite = "test.whisper-residency.\(UUID().uuidString)"
        defer { Self.dropSuite(suite) }
        let settings = try Self.makeSettings(suite: suite)
        let log = UnloadLog()
        let center = NotificationCenter()
        let residency = WhisperModelResidency(settings: settings, notifications: center) {
            await log.record()
        }

        center.post(name: UIApplication.didReceiveMemoryWarningNotification, object: nil)
        #expect(await eventually { await log.unloads == 1 })
        withExtendedLifetime(residency) {}
    }
}
