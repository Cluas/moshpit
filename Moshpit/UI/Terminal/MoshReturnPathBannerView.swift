import SwiftUI

/// Dismissible banner shown at the top of the terminal when a Mosh session's
/// UDP return path is dead: the socket connected and we kept sending, but the
/// server's reply datagrams never arrived, so the screen would otherwise stay
/// black behind a live cursor with no explanation. This almost always means a
/// VPN / proxy / firewall on the current network is passing our outbound UDP
/// but dropping the inbound datagrams — SSH rides TCP and works through the
/// same path, so "Switch to SSH" is the one-tap escape. The terminal stays
/// usable behind the banner (keystrokes still reach the server; only the
/// rendering is starved), and dismissing leaves the Mosh session as-is.
struct MoshReturnPathBannerView: View {
    let onSwitchToSSH: () -> Void
    let onDismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "wifi.exclamationmark")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Ink.warn)
                .padding(.top, 1)

            VStack(alignment: .leading, spacing: 6) {
                Text("Mosh isn't receiving data — your network may be blocking UDP (VPN, proxy, or firewall).")
                    .font(Face.text(12, .medium))
                    .foregroundStyle(Ink.primary)
                    .fixedSize(horizontal: false, vertical: true)

                Button(action: onSwitchToSSH) {
                    Text("Switch to SSH")
                        .font(Face.mono(11, .bold))
                        .kerning(0.5)
                        .foregroundStyle(Ink.accent)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("mosh-returnpath-switch-ssh")
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
            .accessibilityIdentifier("mosh-returnpath-dismiss")
        }
        .padding(EdgeInsets(top: 11, leading: 13, bottom: 11, trailing: 6))
        .background(Ink.roamBanner)
        .background(.ultraThinMaterial)
        .clipShape(RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.warn.opacity(0.34), lineWidth: 1))
        .shadow(color: .black.opacity(0.42), radius: 19, y: 9)
        .accessibilityIdentifier("mosh-returnpath-banner")
    }
}
