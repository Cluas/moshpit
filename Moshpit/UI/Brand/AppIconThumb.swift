import SwiftUI

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
