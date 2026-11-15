import Foundation
import Observation
import UIKit
import MoshpitKit

/// When the loaded Whisper model may be dropped.
///
/// The model is several hundred megabytes, so it goes as soon as nothing needs
/// it — but never from under a dictation. A session holds the model from
/// `prepare()` until it ends; an unload asked for while one is open waits for
/// the last one to close. Pure bookkeeping, kept apart from the store so the
/// rule can be tested without a model.
struct WhisperSessionLedger {
    private(set) var openSessions: Set<UUID> = []
    /// An unload came in while a session was open.
    private(set) var unloadWhenIdle = false
    /// Moves on every drop, so a load that was in flight when the drop landed
    /// knows not to put the model back.
    private(set) var generation = 0

    /// A dictation session starts using the model.
    mutating func open() -> UUID {
        let id = UUID()
        openSessions.insert(id)
        return id
    }

    /// A session is done with the model. True when the model should go now.
    /// Closing a session twice is harmless.
    mutating func close(_ id: UUID) -> Bool {
        guard openSessions.remove(id) != nil else { return false }
        return openSessions.isEmpty && unloadWhenIdle ? drop() : false
    }

    /// Someone wants the model gone. True when it should go now; otherwise it
    /// goes when the last open session closes.
    mutating func requestUnload() -> Bool {
        guard openSessions.isEmpty else {
            unloadWhenIdle = true
            return false
        }
        return drop()
    }

    private mutating func drop() -> Bool {
        unloadWhenIdle = false
        generation += 1
        return true
    }
}

/// Drops the Whisper model when it can't be used: voice input switched off,
/// dictation moved to Apple's engine, or the system warning that memory is
/// short. The next Whisper dictation loads it again.
///
/// Without this nothing ever unloaded it: after one Whisper dictation the
/// model (the large one is a 954 MB download) stayed resident until the app
/// exited, whatever the settings said afterwards.
///
/// A dictation in progress is safe: `WhisperModelStore.unload()` waits for its
/// session to end (``WhisperSessionLedger``).
@MainActor
final class WhisperModelResidency {
    /// Posted by ``WhisperDownloadCenter`` when a variant finished
    /// installing, its id under ``variantKey``.
    static let modelInstalled = Notification.Name("moshpit.whisperModelInstalled")
    static let variantKey = "variant"

    private let settings: AppSettings
    private let unload: @Sendable () async -> Void
    private let notifications: NotificationCenter
    private let memoryWarnings: NSObjectProtocol
    private var installs: NSObjectProtocol?

    /// `unload` defaults to `WhisperModelStore.shared.unload()`; tests count
    /// calls instead.
    init(settings: AppSettings,
         notifications: NotificationCenter = .default,
         unload: (@Sendable () async -> Void)? = nil) {
        let unload = unload ?? { await WhisperModelStore.shared.unload() }
        self.settings = settings
        self.unload = unload
        self.notifications = notifications
        memoryWarnings = notifications.addObserver(
            forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: nil
        ) { _ in
            Log.voice.info("memory warning: unloading the whisper model once it is idle")
            Task { await unload() }
        }
        installs = notifications.addObserver(forName: Self.modelInstalled, object: nil, queue: nil) { [weak self] note in
            guard let variant = note.userInfo?[Self.variantKey] as? String else { return }
            Task { @MainActor [weak self] in self?.installed(variant) }
        }
        track()
    }

    deinit {
        notifications.removeObserver(memoryWarnings)
        if let installs { notifications.removeObserver(installs) }
    }

    /// A variant finished installing — and the install loaded it, on
    /// purpose, so CoreML compiles it while the download screen still has a
    /// network (`WhisperModelStore.install`). It may stay only if it is the
    /// model a dictation would use (B675): installed while dictation had
    /// meanwhile moved to Apple's engine or voice input was switched off, or
    /// installed beside the model in use, it used to stay resident — up to a
    /// gigabyte, nothing listening for it — until the next memory warning.
    private func installed(_ variant: String) {
        let inUse = Self.wantsWhisper(settings)
            && (settings.whisperModelId.isEmpty || settings.whisperModelId == variant)
        guard !inUse else { return }
        Log.voice.info("installed \(variant, privacy: .public) isn't the model in use: unloading it")
        let unload = self.unload
        Task { await unload() }
    }

    /// Whether a dictation started now would run on Whisper.
    static func wantsWhisper(_ settings: AppSettings) -> Bool {
        settings.voiceInputEnabled && settings.voiceEngine == .whisper
    }

    private func track() {
        withObservationTracking {
            _ = Self.wantsWhisper(settings)
        } onChange: { [weak self] in
            // Called before the new value is stored; read it once it has been.
            Task { @MainActor [weak self] in self?.settingsChanged() }
        }
    }

    private func settingsChanged() {
        if !Self.wantsWhisper(settings) {
            let unload = self.unload
            Task { await unload() }
        }
        track()
    }
}
