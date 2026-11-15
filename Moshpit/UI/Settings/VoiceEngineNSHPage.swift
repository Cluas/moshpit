import SwiftUI

/// Settings › Voice Input › Recognition drawn by NSH
/// (`ui/src/voice_engine.rs`): the engine in use and whether Whisper still
/// lacks a model come from the host (``VoiceEngineModel``), a pick goes back
/// to it, and Download a model pushes the SwiftUI model page as the SwiftUI
/// page's link does.
struct VoiceEngineNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @State private var model = VoiceEngineModel()
    @State private var failed = false
    @State private var showModels = false

    var body: some View {
        if failed {
            NSHPageFailed { failed = false }
        } else {
            NSHPage(
                screen: .voiceEngine,
                attach: { model.attach($0, settings: settings) },
                onMessage: { message in
                    switch message["ev"] as? String {
                    case "back":
                        dismiss()
                    case "select":
                        if let engine = (message["engine"] as? String).flatMap(VoiceEngineKind.init(rawValue:)) {
                            model.select(engine, settings: settings)
                        }
                    case "models":
                        showModels = true
                    default:
                        break
                    }
                },
                onFailure: { failed = true })
                .nshPage()
                // The title the model page's back button shows.
                .navigationTitle("Recognition")
                // Again on the way back from the model page, as the SwiftUI
                // page counts the installed models on appear.
                .onAppear { model.refresh(settings: settings) }
                .navigationDestination(isPresented: $showModels) { WhisperModelNSHPage() }
        }
    }
}

/// What the NSH Recognition page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule), and the message that
/// carries it (`{"op": "engine", …}`, `ui/src/voice_engine.rs`'s `Message`).
@MainActor
final class VoiceEngineModel {
    /// How many Whisper models are on the device, counted when the page
    /// appears (nil: not yet).
    private(set) var installedCount: Int?
    private weak var surface: NSHSurface?
    /// Counts the installed models: the store's catalog, or a test's.
    var countInstalled: () -> Int = { WhisperModelStore.catalog().filter(\.isInstalled).count }

    /// `surface` shows the engine from now on, starting with the current
    /// state (the models counted first if the page has not appeared yet).
    func attach(_ surface: NSHSurface, settings: AppSettings) {
        self.surface = surface
        if installedCount == nil {
            installedCount = countInstalled()
        }
        post(settings)
    }

    /// The page appeared: count the models again.
    func refresh(settings: AppSettings) {
        installedCount = countInstalled()
        post(settings)
    }

    /// The user picked `engine`, as the SwiftUI page applies it: the first
    /// switch to Whisper names the user's own language rather than leaving
    /// it on auto-detect.
    func select(_ engine: VoiceEngineKind, settings: AppSettings) {
        guard settings.voiceEngine != engine else { return }
        settings.voiceEngine = engine
        if engine == .whisper, settings.whisperLanguage.isEmpty {
            settings.whisperLanguage = VoiceLanguageResolver.initialWhisperLanguage()
        }
        post(settings)
    }

    private func post(_ settings: AppSettings) {
        surface?.post(Self.message(engine: settings.voiceEngine, installedModels: installedCount ?? 0))
    }

    /// The screen's message: the engine in use, and whether Whisper needs a
    /// model downloaded first (the SwiftUI page's SETUP group).
    nonisolated static func message(engine: VoiceEngineKind, installedModels: Int) -> [String: Any] {
        [
            "op": "engine",
            "engine": engine.rawValue,
            "needsModel": engine == .whisper && installedModels == 0,
        ]
    }
}
