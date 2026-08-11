import SwiftUI

/// Download, choose, and delete local Whisper models.
///
/// Sizes are on every row before anything is tapped, because these downloads
/// are two to three orders of magnitude larger than anything else the app
/// fetches, and a phone on a metered connection deserves to be asked rather
/// than told.
struct WhisperModelView: View {
    @Environment(AppSettings.self) private var settings

    @State private var options: [WhisperModelOption] = []
    /// Variant → 0…1 while a download is in flight.
    @State private var progress: [String: Double] = [:]
    @State private var tasks: [String: Task<Void, Never>] = [:]
    @State private var failure: String?
    @State private var pendingDelete: WhisperModelOption?

    var body: some View {
        @Bindable var settings = settings

        ZStack {
            MoshpitBackground()

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    FormGroup(
                        title: "WHISPER MODEL",
                        footer: "Models are downloaded from Hugging Face once, then everything runs on this device — no audio is ever uploaded. Every model here is multilingual; the bigger ones are markedly better outside English and on speech that switches language mid-sentence, but take longer per phrase. Only models this device can run are listed."
                    ) {
                        ForEach(options) { option in
                            modelRow(option, settings: settings)
                        }
                    }

                    if let failure {
                        FormGroup(title: "LAST ERROR") {
                            Text(failure)
                                .font(Face.text(12))
                                .foregroundStyle(Ink.warn)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(minHeight: Metrics.cellMinHeight, alignment: .leading)
                        }
                    }

                    if installedBytes > 0 {
                        FormGroup(title: "STORAGE") {
                            ValueRow(label: "Models on this device",
                                     value: format(installedBytes))
                        }
                    }
                }
                .padding(.horizontal, Metrics.pageHPad)
                .padding(.bottom, 28)
            }
        }
        .navigationTitle("Whisper Model")
        .navigationBarTitleDisplayMode(.inline)
        .preferredColorScheme(.dark)
        .onAppear(perform: reload)
        .onDisappear {
            // Leaving the screen must not orphan a 950 MB transfer that
            // nothing is left to report on.
            tasks.values.forEach { $0.cancel() }
            tasks.removeAll()
        }
        .alert("Remove this model?", isPresented: deleteBinding, presenting: pendingDelete) { option in
            Button("Remove", role: .destructive) { remove(option) }
            Button("Keep", role: .cancel) {}
        } message: { option in
            Text("\(option.name) frees \(format(option.installedBytes ?? 0)). You can download it again later.")
        }
    }

    // MARK: Rows

    @ViewBuilder
    private func modelRow(_ option: WhisperModelOption, settings: AppSettings) -> some View {
        let isSelected = selectedVariant == option.id
        let inFlight = progress[option.id]

        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(option.name).font(Face.text(14)).foregroundStyle(Ink.primary)
                    Text(option.summary)
                        .font(Face.text(11))
                        .foregroundStyle(Ink.meta)
                        .fixedSize(horizontal: false, vertical: true)
                        .multilineTextAlignment(.leading)
                    Text(option.isInstalled
                        ? format(option.installedBytes ?? 0)
                        : String(localized: "≈\(format(option.approximateBytes)) download"))
                        .font(Face.mono(10))
                        .foregroundStyle(Ink.tertiary)
                }
                Spacer(minLength: 8)
                trailing(option, isSelected: isSelected, inFlight: inFlight, settings: settings)
            }
            if let inFlight {
                ProgressView(value: inFlight)
                    .tint(Ink.accent)
                    .accessibilityLabel("Downloading \(option.name)")
            }
        }
        .frame(minHeight: Metrics.cellMinHeight)
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .onTapGesture {
            guard option.isInstalled, !isSelected else { return }
            settings.whisperModelId = option.id
            Haptics.select()
        }
    }

    @ViewBuilder
    private func trailing(_ option: WhisperModelOption, isSelected: Bool,
                          inFlight: Double?, settings: AppSettings) -> some View {
        if inFlight != nil {
            Button {
                tasks[option.id]?.cancel()
                tasks[option.id] = nil
                progress[option.id] = nil
            } label: {
                Text("Cancel").font(Face.text(12, .semibold)).foregroundStyle(Ink.secondary)
            }
            .buttonStyle(.plain)
        } else if option.isInstalled {
            HStack(spacing: 12) {
                if isSelected {
                    Image(systemName: "checkmark")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Ink.accent)
                }
                Button {
                    pendingDelete = option
                } label: {
                    Image(systemName: "trash")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Ink.warn)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Remove \(option.name)")
            }
        } else {
            Button { download(option, settings: settings) } label: {
                Text("Download")
                    .font(Face.text(12, .semibold))
                    .foregroundStyle(Color(hex: "090B0D"))
                    .padding(.horizontal, 12)
                    .frame(height: 28)
                    .background(Ink.accent, in: Capsule(style: .continuous))
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: Actions

    /// The model a session would actually use — the stored pick when it's
    /// installed, otherwise whatever else is. Mirrors the engine's own
    /// fallback so the checkmark never points at a model that isn't there.
    private var selectedVariant: String? {
        WhisperModelStore.resolvedVariant(preferring: settings.whisperModelId)
    }

    private var installedBytes: Int64 {
        options.compactMap(\.installedBytes).reduce(0, +)
    }

    private var deleteBinding: Binding<Bool> {
        Binding(get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } })
    }

    private func reload() {
        options = WhisperModelStore.catalog()
    }

    private func download(_ option: WhisperModelOption, settings: AppSettings) {
        guard tasks[option.id] == nil else { return }
        failure = nil
        progress[option.id] = 0
        tasks[option.id] = Task {
            do {
                try await WhisperModelStore.shared.install(variant: option.id) { value in
                    Task { @MainActor in progress[option.id] = value }
                }
                guard !Task.isCancelled else { return }
                // First model on the device becomes the active one — nobody
                // downloads a model and then expects to pick it separately.
                if settings.whisperModelId.isEmpty || selectedVariant == nil {
                    settings.whisperModelId = option.id
                }
                Haptics.success()
            } catch {
                guard !Task.isCancelled else { return }
                failure = error.localizedDescription
                Log.voice.error("whisper model download failed: \(error.localizedDescription)")
            }
            progress[option.id] = nil
            tasks[option.id] = nil
            reload()
        }
    }

    private func remove(_ option: WhisperModelOption) {
        Task {
            do {
                try await WhisperModelStore.shared.remove(variant: option.id)
                if settings.whisperModelId == option.id { settings.whisperModelId = "" }
            } catch {
                failure = error.localizedDescription
            }
            reload()
        }
    }

    private func format(_ bytes: Int64) -> String {
        bytes.formatted(.byteCount(style: .file))
    }
}
