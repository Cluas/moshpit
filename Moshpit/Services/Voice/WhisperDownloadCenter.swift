import Foundation
import Observation

/// Owns in-flight Whisper model downloads, for the whole app's lifetime.
///
/// Exists because the first version put the download `Task` in the model
/// screen's `@State` and cancelled it in `onDisappear`. That reasoning was
/// backwards: it treated leaving the screen as abandoning the download, when a
/// several-hundred-megabyte transfer is exactly the thing a user expects to
/// keep running while they go and do something else. Navigating away, or
/// switching to another app, killed it — and with the progress living in the
/// view, there was nothing left to show that it had ever started.
///
/// A singleton on the main actor instead: the download outlives any screen, and
/// any screen can observe it. Pair this with the store's background URLSession
/// (which keeps bytes moving while the app is suspended) and its resume cache
/// (which makes an interruption cost only the time already spent).
@Observable
@MainActor
final class WhisperDownloadCenter {
    static let shared = WhisperDownloadCenter()

    /// Variant → 0…1 for downloads currently running.
    private(set) var progress: [String: Double] = [:]
    /// Variant → the message from its last failure. Cleared when it's retried.
    private(set) var failure: [String: String] = [:]
    /// Bumped whenever a download finishes, so views re-read the catalog
    /// (install state lives on disk, not here).
    private(set) var completions = 0

    @ObservationIgnored private var tasks: [String: Task<Void, Never>] = [:]

    private init() {}

    func isDownloading(_ variant: String) -> Bool { tasks[variant] != nil }

    /// Start — or carry on — fetching a variant. A second call while one is in
    /// flight is ignored rather than starting a competing transfer.
    func start(_ variant: String, onFinished: @escaping @MainActor (String) -> Void = { _ in }) {
        guard tasks[variant] == nil else { return }
        failure[variant] = nil
        progress[variant] = WhisperModelStore.partialBytes(for: variant) > 0 ? 0.01 : 0
        tasks[variant] = Task { [weak self] in
            do {
                try await WhisperModelStore.shared.install(variant: variant) { value in
                    Task { @MainActor [weak self] in self?.progress[variant] = value }
                }
                guard !Task.isCancelled else { return }
                self?.completions += 1
                onFinished(variant)
                Haptics.success()
            } catch {
                guard !Task.isCancelled else { return }
                self?.failure[variant] = error.localizedDescription
                Log.voice.error("whisper model download failed: \(error.localizedDescription)")
            }
            self?.progress[variant] = nil
            self?.tasks[variant] = nil
            self?.completions += 1
        }
    }

    /// Stop transferring but KEEP what's already on disk — the resume cache is
    /// what lets Resume pick up mid-file instead of starting over.
    func pause(_ variant: String) {
        tasks[variant]?.cancel()
        tasks[variant] = nil
        progress[variant] = nil
    }

    /// Stop and throw away the partial bytes.
    func discard(_ variant: String) async {
        pause(variant)
        try? await WhisperModelStore.shared.discardPartial(variant: variant)
        failure[variant] = nil
        completions += 1
    }
}
