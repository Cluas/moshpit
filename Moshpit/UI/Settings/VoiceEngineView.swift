import SwiftUI

/// Picks which speech engine dictation runs on.
///
/// Two entries rather than an "Automatic" third: the choice has a real cost on
/// one side (a several-hundred-megabyte download) and a real capability on the
/// other (one sentence, two languages), and silently deciding that for someone
/// is exactly what leaves a multilingual user staring at a transcript of
/// nonsense with no idea what to change.
struct VoiceEngineView: View {
    @Environment(AppSettings.self) private var settings

    @State private var installedCount = 0

    var body: some View {
        @Bindable var settings = settings

        ZStack {
            MoshpitBackground()

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    FormGroup(
                        title: "RECOGNITION",
                        footer: "Both engines run entirely on this device — your voice is never uploaded. Whisper additionally needs its model downloaded once over the network before it can be used."
                    ) {
                        ForEach(VoiceEngineKind.allCases) { kind in
                            engineRow(kind: kind, settings: settings)
                        }
                    }

                    if settings.voiceEngine == .whisper, installedCount == 0 {
                        FormGroup(title: "SETUP") {
                            NavigationLink {
                                WhisperModelView()
                            } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: "arrow.down.circle")
                                        .font(.system(size: 14, weight: .semibold))
                                        .foregroundStyle(Ink.accent)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text("Download a model")
                                            .font(Face.text(14)).foregroundStyle(Ink.primary)
                                        Text("Whisper can't transcribe until one is on the device")
                                            .font(Face.text(11)).foregroundStyle(Ink.meta)
                                    }
                                    Spacer()
                                    MiniChevron()
                                }
                                .frame(minHeight: Metrics.cellMinHeight)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .padding(.horizontal, Metrics.pageHPad)
                .padding(.bottom, 28)
            }
        }
        .navigationTitle("Recognition")
        .navigationBarTitleDisplayMode(.inline)
        .preferredColorScheme(.dark)
        .onAppear { installedCount = WhisperModelStore.catalog().filter(\.isInstalled).count }
    }

    private func engineRow(kind: VoiceEngineKind, settings: AppSettings) -> some View {
        Button {
            guard settings.voiceEngine != kind else { return }
            settings.voiceEngine = kind
            // First switch to Whisper: preselect a language rather than
            // leaving it on Auto-detect when the user's own configuration
            // already says they speak Chinese. Detection wavers on
            // code-switched speech; an explicit language plus the mixed-word
            // primer doesn't.
            if kind == .whisper, settings.whisperLanguage.isEmpty {
                settings.whisperLanguage = VoiceLanguageResolver.initialWhisperLanguage()
            }
            Haptics.select()
        } label: {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(kind.displayName).font(Face.text(14)).foregroundStyle(Ink.primary)
                    Text(kind.summary)
                        .font(Face.text(11))
                        .foregroundStyle(Ink.meta)
                        .fixedSize(horizontal: false, vertical: true)
                        .multilineTextAlignment(.leading)
                }
                Spacer()
                if settings.voiceEngine == kind {
                    Image(systemName: "checkmark")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Ink.accent)
                }
            }
            .frame(minHeight: Metrics.cellMinHeight)
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(settings.voiceEngine == kind ? .isSelected : [])
    }
}
