import MoshpitKit
import SwiftUI

/// Settings › Voice Input › Model drawn by NSH (`ui/src/whisper_model.rs`):
/// the models, their downloads and the one in use come from the host
/// (``WhisperModelPageModel``, over `WhisperModelStore` and
/// `WhisperDownloadCenter`), again whenever any of it changes; what the user
/// does goes back to the host. Removal asks first, with an alert, and goes
/// to the store: a model
/// removed while it is still loading is the store's to drop (B124), not
/// something this page tracks.
struct WhisperModelNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @State private var model = WhisperModelPageModel()
    @State private var failed = false
    @State private var pendingDelete: WhisperModelOption?

    /// The downloads, app-wide (``WhisperDownloadCenter``).
    private var downloads: WhisperDownloadCenter { .shared }

    var body: some View {
        if failed {
            NSHPageFailed { failed = false }
        } else {
            NSHPage(
                screen: .whisperModel,
                attach: { model.attach($0, settings: settings) },
                onEvent: { if case .whisperModel(let event) = $0 { handle(event) } },
                onFailure: { failed = true })
                .nshPage()
                .navigationTitle("Whisper Model")
                .onAppear { model.reload(settings: settings) }
                // Install state lives on disk: read it again when a download
                // ends; the rest is the download centre's and the setting's.
                .onChange(of: downloads.completions) { _, _ in model.reload(settings: settings) }
                .onChange(of: downloads.progress) { _, _ in model.post(settings) }
                .onChange(of: downloads.lastFailure) { _, _ in model.post(settings) }
                .onChange(of: settings.whisperModelId) { _, _ in model.post(settings) }
                .alert("Remove this model?", isPresented: deleteBinding, presenting: pendingDelete) { option in
                    Button("Remove", role: .destructive) { model.remove(option, settings: settings) }
                    Button("Keep", role: .cancel) {}
                } message: { option in
                    Text("\(option.name) frees \(WhisperModelPageModel.format(option.installedBytes ?? 0)). You can download it again later.")
                }
        }
    }

    private var deleteBinding: Binding<Bool> {
        Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } })
    }

    /// What the screen tells the host.
    private func handle(_ event: WhisperModelEvent) {
        switch event {
        case .back:
            dismiss()
        case .select(let id):
            model.select(id, settings: settings)
        case .download(let id):
            model.download(id, settings: settings)
        case .pause(let id):
            downloads.pause(id)
        case .discard(let id):
            Task { await downloads.discard(id) }
        case .remove(let id):
            pendingDelete = model.options.first { $0.id == id && $0.isInstalled }
        }
    }
}

/// What the NSH Model page shows, owned by the host so a surface made again
/// starts from it (``NSHPage``'s third rule), and the state that carries it
/// (``WhisperModelsState``, the screen's own through the page bridge).
@MainActor
final class WhisperModelPageModel {
    /// The models this device can run, as last read from disk.
    private(set) var options: [WhisperModelOption] = []
    private weak var surface: NSHSurface?
    /// The catalog: the store's, or a capture run's seed.
    var catalog: () -> [WhisperModelOption] = { WhisperModelSeed.requested?.options ?? WhisperModelStore.catalog() }

    /// `surface` shows the models from now on, starting with the current
    /// state.
    func attach(_ surface: NSHSurface, settings: AppSettings) {
        self.surface = surface
        if options.isEmpty { options = catalog() }
        post(settings)
    }

    /// Reads the catalog again (a download ended, a model was removed).
    func reload(settings: AppSettings) {
        options = catalog()
        post(settings)
    }

    /// The model a session would use: the stored pick when it is installed,
    /// otherwise whatever else is.
    static func selected(_ settings: AppSettings) -> String? {
        WhisperModelSeed.requested?.selected ?? WhisperModelStore.resolvedVariant(preferring: settings.whisperModelId)
    }

    /// The user picked downloaded model `id`.
    func select(_ id: String, settings: AppSettings) {
        guard options.contains(where: { $0.id == id && $0.isInstalled }), id != Self.selected(settings) else { return }
        settings.whisperModelId = id
    }

    /// Download or Resume: the first model on the device becomes the one in
    /// use.
    func download(_ id: String, settings: AppSettings) {
        guard options.contains(where: { $0.id == id }) else { return }
        WhisperDownloadCenter.shared.start(id) { variant in
            if settings.whisperModelId.isEmpty || Self.selected(settings) == nil {
                settings.whisperModelId = variant
            }
        }
    }

    /// The alert's Remove: the store deletes the weights (and drops the model,
    /// even one still loading); a removed pick falls back to whatever else is
    /// installed.
    func remove(_ option: WhisperModelOption, settings: AppSettings) {
        Task {
            do {
                try await WhisperModelStore.shared.remove(variant: option.id)
                if settings.whisperModelId == option.id { settings.whisperModelId = "" }
            } catch {
                Log.voice.error("whisper model removal failed: \(error.localizedDescription)")
            }
            reload(settings: settings)
        }
    }

    func post(_ settings: AppSettings) {
        let downloads = WhisperDownloadCenter.shared
        let seed = WhisperModelSeed.requested
        surface?.post(.whisperModel(state: Self.state(
            options: options,
            progress: seed?.progress ?? downloads.progress,
            failure: seed?.failure ?? downloads.lastFailure,
            selected: Self.selected(settings))))
    }

    /// What the page shows: every model with its sizes formatted, its
    /// download's progress and whether it is the one in use; the last
    /// failure; what the downloaded ones take.
    nonisolated static func state(options: [WhisperModelOption], progress: [String: Double],
                                  failure: String?, selected: String?) -> WhisperModelsState {
        let installed = options.compactMap(\.installedBytes).reduce(0, +)
        return WhisperModelsState(
            models: options.map { option in
                WhisperModelRow(
                    id: option.id, name: option.name, summary: option.summary,
                    approximate: format(option.approximateBytes),
                    installed: option.installedBytes.map(format),
                    partial: option.isPartial ? format(option.partialBytes) : nil,
                    progress: progress[option.id], selected: option.id == selected)
            },
            failure: failure,
            storage: installed > 0 ? format(installed) : nil)
    }

    /// Bytes as a file size (`ByteCountFormatter`'s `.file` style).
    nonisolated static func format(_ bytes: Int64) -> String {
        bytes.formatted(.byteCount(style: .file))
    }
}

/// What a capture run has the model pages show (`-MOSHPIT_SEED_WHISPER
/// mixed`): one downloaded and in use, one downloading,
/// one paused, one not downloaded, and a failure. Nothing is written: the
/// seed stands in for the catalog, the downloads and the pick. DEBUG only.
struct WhisperModelSeed {
    let options: [WhisperModelOption]
    let progress: [String: Double]
    let failure: String?
    let selected: String?

    static var requested: WhisperModelSeed? {
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-MOSHPIT_SEED_WHISPER"), i + 1 < args.count,
              args[i + 1] == "mixed"
        else { return nil }
        return mixed
        #else
        return nil
        #endif
    }

    #if DEBUG
    private static let mixed: WhisperModelSeed = {
        let names: [(String, String, String, Int64)] = [
            ("openai_whisper-large-v3_turbo_954MB", String(localized: "Large v3 Turbo"),
             String(localized: "Most accurate, especially on mixed-language speech. Needs a recent chip."), 954),
            ("openai_whisper-small", String(localized: "Small"),
             String(localized: "Good balance of accuracy and speed."), 470),
            ("openai_whisper-base", String(localized: "Base"),
             String(localized: "Fast and light. Noticeably weaker outside English."), 145),
            ("openai_whisper-tiny", String(localized: "Tiny"),
             String(localized: "Fastest, lowest accuracy. For older devices."), 75),
        ]
        let megabyte: Int64 = 1_000_000
        let options = names.enumerated().map { index, entry in
            WhisperModelOption(
                id: entry.0, name: entry.1, summary: entry.2, approximateBytes: entry.3 * megabyte,
                installedBytes: index == 0 ? 1_621 * megabyte : nil,
                partialBytes: index == 2 ? 48 * megabyte : (index == 1 ? 120 * megabyte : 0))
        }
        return WhisperModelSeed(
            options: options, progress: [names[1].0: 0.42],
            failure: "The network connection was lost.", selected: names[0].0)
    }()
    #endif
}
