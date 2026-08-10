import SwiftUI

/// Dismissible banner shown at the top of the terminal when the session ran at
/// less than the user's chosen transport because the host lacked a dependency.
/// "Install …" opens the Install Assist sheet; the × dismisses for this
/// session. Never blocks — the terminal is fully usable behind it.
struct HostBannerView: View {
    let notice: DegradeNotice
    let onInstall: () -> Void
    let onDismiss: () -> Void

    private var message: String {
        switch notice.missing {
        case .tmux:
            // "tmux not found on this host — plain SSH session."
            return String(localized: "tmux not found on this host — plain SSH session.")
        case .herdr:
            // We do NOT silently attach tmux instead, even when it's present:
            // the two hold unrelated sessions.
            return String(localized: "herdr not found on this host — plain shell session.")
        case .moshServer:
            // "mosh-server not found — connected over SSH instead."
            return String(localized: "mosh-server not found — connected over SSH instead.")
        }
    }

    private var installLabel: String {
        switch notice.missing {
        case .tmux:       return String(localized: "Install tmux")
        case .herdr:      return String(localized: "Install herdr")
        case .moshServer: return String(localized: "Install mosh")
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Ink.warn)
                .padding(.top, 1)

            VStack(alignment: .leading, spacing: 6) {
                Text(message)
                    .font(Face.text(12, .medium))
                    .foregroundStyle(Ink.primary)
                    .fixedSize(horizontal: false, vertical: true)

                Button(action: onInstall) {
                    Text(installLabel)
                        .font(Face.mono(11, .bold))
                        .kerning(0.5)
                        .foregroundStyle(Ink.accent)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("host-banner-install")
            }

            Spacer(minLength: 4)

            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(Ink.secondary)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("host-banner-dismiss")
        }
        .padding(EdgeInsets(top: 11, leading: 13, bottom: 11, trailing: 6))
        .background(Ink.roamBanner)
        .background(.ultraThinMaterial)
        .clipShape(RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.warn.opacity(0.34), lineWidth: 1))
        .shadow(color: .black.opacity(0.42), radius: 19, y: 9)
        .accessibilityIdentifier("host-banner")
    }
}
