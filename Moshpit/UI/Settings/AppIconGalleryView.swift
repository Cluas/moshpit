import SwiftUI
import UIKit

/// Home-screen icon picker.
///
/// Independent of the accent color on purpose: iOS can only switch to icons
/// that were bundled and declared at build time, so an icon could never track a
/// custom accent. Rather than hide that behind an approximate auto-match, the
/// icon is its own choice — and since the set is fixed anyway, it may as well
/// offer genuinely different figures in the primary's glass language rather
/// than eight recolors.
struct AppIconGalleryView: View {
    @Environment(AppSettings.self) private var settings

    /// Surfaced when iOS refuses the change. `setAlternateIconName` fails if the
    /// name isn't in `CFBundleAlternateIcons`, which would otherwise look like
    /// a dead tap.
    @State private var failure: String?

    private let columns = [GridItem(.adaptive(minimum: 96), spacing: 14)]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                LazyVGrid(columns: columns, spacing: 16) {
                    ForEach(AppIconCatalog.all) { option in
                        IconCell(option: option,
                                 isSelected: option.id == settings.appIconId,
                                 onSelect: { apply(option) })
                    }
                }

                Text("The icon is separate from the accent color, because iOS only allows switching between icons bundled with the app — a custom accent can't have matching artwork generated for it.")
                    .font(.footnote)
                    .foregroundStyle(Ink.meta)
                    .fixedSize(horizontal: false, vertical: true)

                if let failure {
                    Text(failure)
                        .font(.footnote)
                        .foregroundStyle(Ink.danger)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(16)
        }
        .background(Ink.screenBG)
        .navigationTitle("App Icon")
        // Fires once iOS has actually accepted the icon — not on the tap it
        // may still reject.
        .sensoryFeedback(.selection, trigger: settings.appIconId)
        .navigationBarTitleDisplayMode(.inline)
    }

    private func apply(_ option: AppIconOption) {
        guard option.id != settings.appIconId else { return }
        UIApplication.shared.setAlternateIconName(option.alternateName) { error in
            Task { @MainActor in
                if let error {
                    // Don't record a selection iOS rejected — the home screen
                    // would disagree with the checkmark.
                    failure = String(localized: "Couldn't change the icon: \(error.localizedDescription)")
                } else {
                    failure = nil
                    settings.appIconId = option.id
                }
            }
        }
    }
}

private struct IconCell: View {
    let option: AppIconOption
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            VStack(spacing: 7) {
                AppIconThumb(option: option, side: 68)
                    .overlay(alignment: .bottomTrailing) {
                        if isSelected {
                            Image(systemName: "checkmark.circle.fill")
                                .font(.system(size: 17))
                                .foregroundStyle(Ink.accent, Ink.groupRaised)
                                .offset(x: 5, y: 5)
                        }
                    }
                Text(option.name)
                    .font(Face.text(12))
                    .foregroundStyle(isSelected ? Ink.primary : Ink.secondary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("app-icon-\(option.id)")
        .accessibilityLabel(option.name)
    }
}

/// The icon as the home screen draws it: every icon is an Icon Composer
/// document, and its preview is that document rendered by Icon Composer's own
/// tool (scripts/gen/generate-alternate-icons.mjs), so the picker shows the glass —
/// not a flat re-drawing that could drift from what lands on the home screen.
struct AppIconThumb: View {
    let option: AppIconOption
    var side: CGFloat = 44

    var body: some View {
        Group {
            if let image = UIImage(named: option.previewAsset) {
                Image(uiImage: image).resizable()
            } else if option.isPrimary {
                // The vector mark in the icon's fixed violet: same geometry
                // (`MoshpitGlyph.Metrics`), flat — what iOS 18 shows anyway.
                MoshpitMark(size: side, accent: MoshpitMark.iconViolet)
            } else {
                // Only reachable if an asset is renamed without regenerating —
                // show something inert rather than an empty hole.
                RoundedRectangle(cornerRadius: side * 0.22, style: .continuous)
                    .fill(Ink.groupRaised)
                    .overlay(Image(systemName: "questionmark").foregroundStyle(Ink.meta))
            }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: side * 0.22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: side * 0.22, style: .continuous)
            .strokeBorder(Color.white.opacity(0.12), lineWidth: 1))
        .shadow(color: .black.opacity(0.3), radius: side * 0.08, y: side * 0.03)
    }
}
