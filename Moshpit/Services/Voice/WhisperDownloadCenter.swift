import Foundation
import Observation
import MoshpitKit

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
/// any screen can observe it. It does not outlive the app's time in the
/// foreground: the store downloads on an ordinary URLSession, NOT a background
/// one (`WhisperModelStore.downloadOnce` says why — the Hub downloader's
/// background session wedges after a kill), so switching apps pauses the
/// transfer. What makes that cheap is the store's resume cache: an interruption
/// costs only the time since the last byte, and Resume carries on from there.
@Observable
@MainActor
final class WhisperDownloadCenter {
    static let shared = WhisperDownloadCenter()

    /// Variant → 0…1 for downloads currently running.
    private(set) var progress: [String: Double] = [:]
    /// Variant → the message from its last failure. Cleared when it's retried.
    private(set) var failure: [String: String] = [:]
    /// The variants in ``failure``, the one that failed most recently last.
    private(set) var failureOrder: [String] = []

    /// The most recent failure still standing — what LAST ERROR shows. With
    /// two downloads failed it used to be whichever the dictionary happened
    /// to list first, an older one as likely as not (B207).
    var lastFailure: String? {
        failureOrder.reversed().lazy.compactMap { self.failure[$0] }.first { !$0.isEmpty }
    }
    /// Bumped whenever a download finishes or is paused, so views re-read
    /// the catalog (install state, and the bytes a paused download has kept,
    /// live on disk, not here).
    private(set) var completions = 0

    @ObservationIgnored private var tasks: [String: Task<Void, Never>] = [:]
    /// Which run of a variant's download is the current one. A paused run's
    /// task goes on until it notices, and what it reports meanwhile is not
    /// this variant's progress any more.
    @ObservationIgnored private var runs: [String: UUID] = [:]

    /// Fetches and loads one variant, reporting 0…1.
    typealias Install = @Sendable (_ variant: String,
                                   _ onProgress: @escaping @Sendable (Double) -> Void) async throws -> Void

    @ObservationIgnored private let install: Install

    /// `shared` is the app's; tests make their own, with an install of their own.
    init(install: @escaping Install = { variant, onProgress in
        try await WhisperModelStore.shared.install(variant: variant, onProgress: onProgress)
    }) {
        self.install = install
    }

    /// `variant` failed with `message`: the most recent failure now.
    func recordFailure(_ message: String, for variant: String) {
        failure[variant] = message
        failureOrder.removeAll { $0 == variant }
        failureOrder.append(variant)
    }

    /// `variant`'s failure no longer stands: retried, or its bytes discarded.
    func clearFailure(for variant: String) {
        failure[variant] = nil
        failureOrder.removeAll { $0 == variant }
    }

    /// Start — or carry on — fetching a variant. A second call while one is in
    /// flight is ignored rather than starting a competing transfer.
    func start(_ variant: String, onFinished: @escaping @MainActor (String) -> Void = { _ in }) {
        guard tasks[variant] == nil else { return }
        clearFailure(for: variant)
        progress[variant] = WhisperModelStore.partialBytes(for: variant) > 0 ? 0.01 : 0
        let run = UUID()
        runs[variant] = run
        let install = self.install
        tasks[variant] = Task { [weak self] in
            // The task owns its download: hold the centre strongly for its
            // duration so the nested progress closure has a stable `self`.
            guard let self else { return }
            do {
                try await install(variant) { value in
                    // Only while this run is the variant's: Pause clears the
                    // row, and a report already on its way — or the load step
                    // finishing after a Pause, whose `1` arrives once the
                    // task has been cancelled and returns before clearing
                    // anything — put "downloading" back under a row whose
                    // download had stopped, with Pause for its button, until
                    // the page was left.
                    Task { @MainActor in
                        guard self.runs[variant] == run else { return }
                        self.progress[variant] = value
                    }
                }
                // Paused: the row is Pause's to draw. Only have the pages
                // look at the disk again — a load that was still finishing
                // has just left the model installed there.
                guard !Task.isCancelled else { self.completions += 1; return }
                self.completions += 1
                onFinished(variant)
                Haptics.success()
            } catch {
                guard !Task.isCancelled else { return }
                self.recordFailure(error.localizedDescription, for: variant)
                Log.voice.error("whisper model download failed: \(error.localizedDescription)")
            }
            self.progress[variant] = nil
            self.tasks[variant] = nil
            self.runs[variant] = nil
            self.completions += 1
        }
    }

    /// Stop transferring but KEEP what's already on disk — the resume cache is
    /// what lets Resume pick up mid-file instead of starting over.
    func pause(_ variant: String) {
        tasks[variant]?.cancel()
        tasks[variant] = nil
        runs[variant] = nil
        progress[variant] = nil
        // The row shows what is on disk now (the bytes kept, Resume, the
        // discard button), not a fresh "Download": the cancelled task
        // returns before its own bump.
        completions += 1
    }

    /// Stop and throw away the partial bytes.
    func discard(_ variant: String) async {
        pause(variant)
        try? await WhisperModelStore.shared.discardPartial(variant: variant)
        clearFailure(for: variant)
        completions += 1
    }
}
