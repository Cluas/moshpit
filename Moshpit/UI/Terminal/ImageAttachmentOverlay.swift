import SwiftUI

/// The attachment panel that rides directly above the shortcut bar while an
/// image upload is in flight. Same compose-then-commit shape as the dictation
/// overlay: the upload happens eagerly, but nothing reaches the PTY until
/// Insert (paste the paths, keep typing) or Send (paste and press Return).
struct ImageAttachmentOverlayView: View {
    let controller: ImageAttachmentController
    let onCancel: () -> Void
    let onInsert: () -> Void
    /// Insert AND press Return. Separate from `onInsert` because the
    /// keystroke that submits a Claude Code prompt executes a shell command.
    let onSend: () -> Void

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
        .accessibilityIdentifier("image-attachment-overlay")
    }

    // MARK: Header — status word + progress

    @ViewBuilder
    private var header: some View {
        HStack(spacing: 8) {
            switch controller.phase {
            case .processing:
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "PREPARING…"), tint: Ink.secondary)
            case .uploading(let fraction):
                ProgressView().controlSize(.mini).tint(Ink.accent)
                statusLabel(String(localized: "UPLOADING \(Int(fraction * 100))%"),
                            tint: Ink.accent)
            case .ready:
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Ink.accent)
                statusLabel(String(localized: "READY TO INSERT"), tint: Ink.accent)
            case .failed:
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Ink.warn)
                statusLabel(String(localized: "UPLOAD FAILED"), tint: Ink.warn)
            }
            Spacer(minLength: 0)
            if totalBytes > 0 {
                Text(ByteCountFormatter.string(fromByteCount: Int64(totalBytes),
                                               countStyle: .file))
                    .font(Face.mono(9, .medium))
                    .foregroundStyle(Ink.tertiary)
            }
        }
    }

    private var totalBytes: Int {
        controller.attachments.reduce(0) { $0 + $1.byteCount }
    }

    private func statusLabel(_ text: String, tint: Color) -> some View {
        Text(text)
            .font(Face.mono(10, .semibold))
            .kerning(1.2)
            .foregroundStyle(tint)
            .lineLimit(1)
    }

    // MARK: Thumbnails / failure message

    @ViewBuilder
    private var content: some View {
        if case .failed(let message) = controller.phase {
            Text(message)
                .font(Face.text(13))
                .foregroundStyle(Ink.secondary)
                .fixedSize(horizontal: false, vertical: true)
        } else if controller.attachments.isEmpty {
            Text("Reading the selected images…")
                .font(Face.text(13))
                .foregroundStyle(Ink.placeholder)
        } else {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(controller.attachments) { attachment in
                        thumbnailCell(attachment)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func thumbnailCell(_ attachment: ImageAttachmentController.Attachment) -> some View {
        ZStack(alignment: .bottomTrailing) {
            Group {
                if let image = attachment.thumbnail {
                    Image(uiImage: image)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                } else {
                    Image(systemName: "photo")
                        .font(.system(size: 16))
                        .foregroundStyle(Ink.tertiary)
                }
            }
            .frame(width: 44, height: 44)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(Ink.groupBorder, lineWidth: 1))

            // A per-file landed tick — with several files on a slow link,
            // "which ones made it" should not be a guess.
            if attachment.remotePath != nil {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(Ink.accent)
                    .background(Circle().fill(Ink.modalBG).padding(1))
                    .offset(x: 3, y: 3)
            }
        }
        .overlay(alignment: .topLeading) {
            // The session number this image is addressable by afterwards —
            // the chip's long-press menu re-inserts #N without re-uploading.
            if let number = attachment.sessionNumber {
                Text("#\(number)")
                    .font(Face.mono(8, .bold))
                    .foregroundStyle(Ink.primary)
                    .padding(.horizontal, 3).padding(.vertical, 1)
                    .background(Ink.modalBG.opacity(0.9), in: Capsule())
                    .offset(x: -2, y: -2)
            }
        }
    }

    // MARK: Actions

    private var actions: some View {
        HStack(spacing: 8) {
            pillButton(String(localized: "Cancel"), prominent: false, action: onCancel)
            Spacer(minLength: 0)
            // Insert leaves the paths on the prompt so the user can add the
            // words around them; Send presses Return too. Two levels of trust,
            // same as dictation — a Return at a shell prompt runs the line.
            pillButton(String(localized: "Insert"), prominent: false,
                       systemImage: "text.insert",
                       disabled: !insertable,
                       action: onInsert)
                .accessibilityIdentifier("image-attach-insert")
            pillButton(String(localized: "Send"), prominent: true,
                       systemImage: "return",
                       disabled: !insertable,
                       action: onSend)
                .accessibilityIdentifier("image-attach-send")
        }
    }

    private var insertable: Bool { controller.phase == .ready }

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
