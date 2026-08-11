import SwiftUI
import UIKit

/// The dictation panel that rides directly above the shortcut bar while a
/// voice session is up. Transcription is *composed* here and only committed
/// to the remote on Insert — a terminal is the one place a mis-heard word
/// must never auto-type (same compose-then-commit shape as Blink's Prompt
/// Mode / Termius voice typing).
struct DictationOverlayView: View {
    let controller: VoiceDictationController
    let onCancel: () -> Void
    let onInsert: () -> Void
    /// Insert AND press Return. Separate from `onInsert` because the keystroke
    /// that submits a Claude Code prompt executes a shell command.
    let onSend: () -> Void

    /// Measured height of the transcript text, so the scroll area hugs a
    /// one-liner instead of reserving its full max height (which parked a
    /// short transcript at the bottom of an empty box).
    @State private var transcriptHeight: CGFloat = 20

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            content
            actions
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 10, trailing: 12))
        .background(Ink.modalBG.opacity(0.97), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Ink.cardBorder, lineWidth: 1))
        .padding(.horizontal, 8)
        .padding(.bottom, 6)
        .accessibilityIdentifier("dictation-overlay")
    }

    // MARK: Header — status word + live level meter

    @ViewBuilder
    private var header: some View {
        HStack(spacing: 8) {
            switch controller.phase {
            case .idle, .starting:
                statusLabel(String(localized: "STARTING…"), tint: Ink.meta)
            case .downloadingModel(let progress):
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "DOWNLOADING SPEECH MODEL \(Int(progress * 100))%"),
                            tint: Ink.secondary)
            case .loadingModel:
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "LOADING SPEECH MODEL…"), tint: Ink.secondary)
            case .listening:
                PulsingDot()
                statusLabel(String(localized: "LISTENING"), tint: Ink.accent)
            case .finishing:
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "FINISHING…"), tint: Ink.secondary)
            case .interrupted:
                Image(systemName: "pause.circle.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Ink.warn)
                statusLabel(String(localized: "INTERRUPTED"), tint: Ink.warn)
            case .failed:
                Image(systemName: "mic.slash.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Ink.warn)
                statusLabel(String(localized: "VOICE INPUT"), tint: Ink.warn)
            }
            Spacer(minLength: 0)
            // The engine and language actually in use, shown from the moment
            // one is picked. This is the fix for the failure that leaves no
            // trace: dictation running in the wrong language doesn't error,
            // it just returns confident nonsense, and until it was on screen
            // there was nothing to tell you that "Automatic" had resolved to
            // English while you were speaking Chinese.
            if !controller.engineLabel.isEmpty, controller.phase != .failed(.microphoneDenied) {
                Text(controller.engineLabel)
                    .font(Face.mono(9, .medium))
                    .foregroundStyle(Ink.tertiary)
                    .lineLimit(1)
                    .truncationMode(.head)
                    .accessibilityIdentifier("dictation-engine-label")
            }
            if controller.phase == .listening {
                DictationLevelMeter(level: controller.level)
            }
        }
    }

    private func statusLabel(_ text: String, tint: Color) -> some View {
        Text(text)
            .font(Face.mono(10, .semibold))
            .kerning(1.2)
            .foregroundStyle(tint)
            .lineLimit(1)
    }

    // MARK: Transcript / failure message

    @ViewBuilder
    private var content: some View {
        if case .failed(let failure) = controller.phase {
            Text(failure.message)
                .font(Face.text(13))
                .foregroundStyle(Ink.secondary)
                .fixedSize(horizontal: false, vertical: true)
        } else if controller.finalizedText.isEmpty && controller.volatileText.isEmpty {
            Text(controller.phase == .interrupted
                ? "Another app took the microphone before anything was heard."
                : "Speak — the text lands here first. Insert types it; Send types it and presses Return.")
                .font(Face.text(13))
                .foregroundStyle(Ink.placeholder)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            // Finalized text is committed; the trailing volatile hypothesis
            // still shimmers (dimmer) while the engine makes up its mind.
            ScrollView {
                (Text(controller.finalizedText).foregroundStyle(Ink.primary)
                    + Text(controller.finalizedText.isEmpty ? "" : " ")
                    + Text(controller.volatileText).foregroundStyle(Ink.tertiary))
                    .font(Face.mono(13))
                    .lineSpacing(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) {
                        transcriptHeight = $0
                    }
            }
            // Hug the text up to ~6 lines, then scroll (pinned to the tail —
            // the words being spoken right now).
            .frame(height: max(20, min(transcriptHeight, 110)))
            .defaultScrollAnchor(.bottom)
            .accessibilityIdentifier("dictation-transcript")
        }
    }

    // MARK: Actions

    @ViewBuilder
    private var actions: some View {
        if case .failed(let failure) = controller.phase {
            HStack(spacing: 8) {
                Spacer(minLength: 0)
                if failure == .microphoneDenied || failure == .speechRecognitionDenied {
                    pillButton(String(localized: "Open Settings"), prominent: true) {
                        if let url = URL(string: UIApplication.openSettingsURLString) {
                            UIApplication.shared.open(url)
                        }
                        onCancel()
                    }
                }
                pillButton(String(localized: "Dismiss"), prominent: false, action: onCancel)
            }
        } else {
            HStack(spacing: 8) {
                pillButton(String(localized: "Cancel"), prominent: false, action: onCancel)
                Spacer(minLength: 0)
                // Insert leaves the text on the prompt to read and edit; Send
                // presses Return too. Both are offered because they are
                // different amounts of trust: Send on a shell prompt runs the
                // command, and dictation is the input most likely to contain a
                // word you did not say.
                pillButton(String(localized: "Insert"), prominent: false,
                           systemImage: "text.insert",
                           disabled: !insertable,
                           action: onInsert)
                    .accessibilityIdentifier("dictation-insert")
                pillButton(String(localized: "Send"), prominent: true,
                           systemImage: "return",
                           disabled: !insertable,
                           action: onSend)
                    .accessibilityIdentifier("dictation-send")
            }
        }
    }

    /// Insert is worth offering once anything was heard — even mid-listen
    /// (it stops the mic and commits) or mid-finalization.
    private var insertable: Bool {
        !controller.transcript.isEmpty || controller.phase == .listening
    }

    private func pillButton(_ label: String, prominent: Bool,
                            systemImage: String? = nil,
                            disabled: Bool = false,
                            action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 5) {
                if let systemImage {
                    Image(systemName: systemImage).font(.system(size: 11, weight: .bold))
                }
                Text(label).font(Face.text(13, .semibold))
            }
            .foregroundStyle(prominent ? Color(hex: "090B0D") : Ink.primary)
            .padding(.horizontal, 14)
            .frame(height: 32)
            .background(
                prominent ? AnyShapeStyle(Ink.accent) : AnyShapeStyle(Ink.neutralFill),
                in: Capsule(style: .continuous))
            .overlay(
                Capsule(style: .continuous)
                    .strokeBorder(prominent ? Color.clear : Ink.groupBorder, lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.45 : 1)
    }
}

// MARK: - Bits

/// The red-dot equivalent in Moshpit's palette: an accent dot breathing at
/// recording cadence.
private struct PulsingDot: View {
    @State private var dim = false

    var body: some View {
        Circle()
            .fill(Ink.accent)
            .frame(width: 7, height: 7)
            .shadow(color: Ink.accent.opacity(0.8), radius: 3)
            .opacity(dim ? 0.35 : 1)
            .onAppear {
                withAnimation(.easeInOut(duration: 0.7).repeatForever(autoreverses: true)) {
                    dim = true
                }
            }
    }
}

/// Five capsules dancing with mic loudness — center-weighted so it reads as
/// a voice waveform, not a VU bar.
struct DictationLevelMeter: View {
    /// 0…1 loudness from the controller.
    let level: Float

    private static let weights: [CGFloat] = [0.45, 0.75, 1.0, 0.75, 0.5]

    var body: some View {
        HStack(spacing: 2.5) {
            ForEach(Self.weights.indices, id: \.self) { index in
                Capsule(style: .continuous)
                    .fill(Ink.accent)
                    .frame(width: 3, height: 3 + 13 * Self.weights[index] * CGFloat(level))
            }
        }
        .frame(height: 18)
        .animation(.easeOut(duration: 0.12), value: level)
        .accessibilityHidden(true)
    }
}
