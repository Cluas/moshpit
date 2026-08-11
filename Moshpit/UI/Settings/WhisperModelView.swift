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
    @State private var pendingDelete: WhisperModelOption?

    /// Downloads are owned app-wide, not by this screen — see
    /// ``WhisperDownloadCenter``. Leaving the screen (or the app) must not
    /// interrupt a several-hundred-megabyte transfer.
    private var downloads: WhisperDownloadCenter { .shared }

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

                    if let failure = downloads.failure.values.first(where: { !$0.isEmpty }) {
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
        // Install state lives on disk, so re-read it whenever a download ends.
        .onChange(of: downloads.completions) { _, _ in reload() }
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
        let inFlight = downloads.progress[option.id]

        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(option.name).font(Face.text(14)).foregroundStyle(Ink.primary)
                    Text(option.summary)
                        .font(Face.text(11))
                        .foregroundStyle(Ink.meta)
                        .fixedSize(horizontal: false, vertical: true)
                        .multilineTextAlignment(.leading)
                    // While a download is interrupted, say how far it got —
                    // that number is the difference between "start over" and
                    // "carry on", and the transfer resumes mid-file.
                    Text(sizeLine(option))
                        .font(Face.mono(10))
                        .foregroundStyle(option.isPartial ? Ink.accent : Ink.tertiary)
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
            // Pause, not cancel: the bytes already on disk stay, so tapping
            // Resume continues from where this stopped.
            Button { downloads.pause(option.id) } label: {
                Text("Pause").font(Face.text(12, .semibold)).foregroundStyle(Ink.secondary)
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
            HStack(spacing: 12) {
                if option.isPartial {
                    Button { Task { await downloads.discard(option.id) } } label: {
                        Image(systemName: "trash")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(Ink.warn)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Discard the partial download of \(option.name)")
                }
                Button { download(option, settings: settings) } label: {
                    Text(option.isPartial ? "Resume" : "Download")
                        .font(Face.text(12, .semibold))
                        .foregroundStyle(Color(hex: "090B0D"))
                        .padding(.horizontal, 12)
                        .frame(height: 28)
                        .background(Ink.accent, in: Capsule(style: .continuous))
                }
                .buttonStyle(.plain)
            }
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

    /// What the size line says, which depends on how far along we are.
    private func sizeLine(_ option: WhisperModelOption) -> String {
        if option.isInstalled { return format(option.installedBytes ?? 0) }
        if option.isPartial {
            return String(localized: "\(format(option.partialBytes)) of ≈\(format(option.approximateBytes)) — paused")
        }
        return String(localized: "≈\(format(option.approximateBytes)) download")
    }

    private func download(_ option: WhisperModelOption, settings: AppSettings) {
        downloads.start(option.id) { variant in
            // First model on the device becomes the active one — nobody
            // downloads a model and then expects to pick it separately.
            if settings.whisperModelId.isEmpty || selectedVariant == nil {
                settings.whisperModelId = variant
            }
        }
    }

    private func remove(_ option: WhisperModelOption) {
        Task {
            do {
                try await WhisperModelStore.shared.remove(variant: option.id)
                if settings.whisperModelId == option.id { settings.whisperModelId = "" }
            } catch {
                Log.voice.error("whisper model removal failed: \(error.localizedDescription)")
            }
            reload()
        }
    }

    private func format(_ bytes: Int64) -> String {
        bytes.formatted(.byteCount(style: .file))
    }
}
