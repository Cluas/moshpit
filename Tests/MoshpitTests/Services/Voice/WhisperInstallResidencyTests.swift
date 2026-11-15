import Foundation
import Testing
@testable import Moshpit

/// A model loaded by its install stays only if it is the one in use (B675).
@Suite("Whisper install residency")
@MainActor
struct WhisperInstallResidencyTests {
    actor UnloadLog {
        private(set) var unloads = 0
        func record() { unloads += 1 }
    }

    private static func settings(suite: String) throws -> AppSettings {
        let settings = AppSettings(defaults: try #require(UserDefaults(suiteName: suite)))
        settings.voiceInputEnabled = true
        settings.voiceEngine = .whisper
        settings.whisperModelId = "openai_whisper-small"
        return settings
    }

    private func installed(_ variant: String, on center: NotificationCenter) {
        center.post(name: WhisperModelResidency.modelInstalled, object: nil,
                    userInfo: [WhisperModelResidency.variantKey: variant])
    }

    private func settle() async { try? await Task.sleep(for: .milliseconds(80)) }

    @Test("Installed while dictation moved to Apple's engine: unloaded")
    func installedWhileApple() async throws {
        let suite = "test.whisper-install.\(UUID().uuidString)"
        defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
        let settings = try Self.settings(suite: suite)
        let center = NotificationCenter()
        let log = UnloadLog()
        let residency = WhisperModelResidency(settings: settings, notifications: center) { await log.record() }
        settings.voiceEngine = .apple
        await settle()
        let before = await log.unloads
        installed("openai_whisper-small", on: center)
        await settle()
        #expect(await log.unloads == before + 1)
        withExtendedLifetime(residency) {}
    }

    @Test("Installed beside the model in use: unloaded")
    func installedBeside() async throws {
        let suite = "test.whisper-install.\(UUID().uuidString)"
        defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
        let settings = try Self.settings(suite: suite)
        let center = NotificationCenter()
        let log = UnloadLog()
        let residency = WhisperModelResidency(settings: settings, notifications: center) { await log.record() }
        installed("openai_whisper-base", on: center)
        await settle()
        #expect(await log.unloads == 1)
        withExtendedLifetime(residency) {}
    }

    @Test("The model in use, installed: kept warm for the first dictation")
    func installedInUse() async throws {
        let suite = "test.whisper-install.\(UUID().uuidString)"
        defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
        let settings = try Self.settings(suite: suite)
        let center = NotificationCenter()
        let log = UnloadLog()
        let residency = WhisperModelResidency(settings: settings, notifications: center) { await log.record() }
        installed("openai_whisper-small", on: center)
        await settle()
        #expect(await log.unloads == 0)
        withExtendedLifetime(residency) {}
    }

    final class Events: @unchecked Sendable {
        private let lock = NSLock()
        private var items: [String] = []
        func add(_ item: String) { lock.withLock { items.append(item) } }
        var all: [String] { lock.withLock { items } }
    }

    @Test("The download centre announces a finished install, after its onFinished")
    func centreAnnounces() async throws {
        let center = NotificationCenter()
        let downloads = WhisperDownloadCenter(install: { _, onProgress in onProgress(1) }, notifications: center)
        let events = Events()
        let observer = center.addObserver(forName: WhisperModelResidency.modelInstalled, object: nil, queue: nil) { note in
            events.add("announced:\(note.userInfo?[WhisperModelResidency.variantKey] as? String ?? "")")
        }
        defer { center.removeObserver(observer) }
        downloads.start("openai_whisper-small") { variant in events.add("finished:\(variant)") }
        let deadline = ContinuousClock.now + .seconds(2)
        while events.all.count < 2, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
        #expect(events.all == ["finished:openai_whisper-small", "announced:openai_whisper-small"])
    }
}
