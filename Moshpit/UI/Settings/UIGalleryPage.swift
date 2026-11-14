import MoshpitKit
import SwiftUI

/// Core Switches › UI gallery: the redesign's `ui` layer (`ui/src/ui/`),
/// every component in its variants, sizes and states, on the NSH screen
/// `ui-gallery` (docs/nsh/UI-DESIGN.md §4.20), dark or light. Tooling for
/// the developer, like the page it hangs off: English only, `verbatim:`.
/// Its entry moves to Settings › About › Version (seven taps) with the
/// redesign's Settings (R8).
struct UIGalleryPage: View {
    @State private var light = false

    var body: some View {
        UIGallerySurface(light: light)
            // A new appearance is a new root: make the surface again.
            .id(light)
            .ignoresSafeArea(edges: .bottom)
            .navigationTitle(Text(verbatim: "UI gallery"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Picker(selection: $light) {
                        Text(verbatim: "Dark").tag(false)
                        Text(verbatim: "Light").tag(true)
                    } label: {
                        Text(verbatim: "Appearance")
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("ui-gallery-appearance")
                }
            }
    }
}

/// A surface running `ui-gallery` in the appearance asked for.
struct UIGallerySurface: UIViewRepresentable {
    let light: Bool

    /// The params the gallery runs with: the app's (its language, the theme
    /// the user picked) and the appearance (`ui/src/screen.rs`'s
    /// `Params.light`, which only the gallery reads).
    @MainActor
    static func params(light: Bool) -> [String: Any] {
        var params = NSHScreenParams.current()
        params["light"] = light
        return params
    }

    /// The host components the gallery's components need beyond every
    /// surface's own (`NSHHostComponents`): the app's mark (an empty page's,
    /// Home's components) and a secure field for the secret fields, which
    /// keeps what is typed nowhere (a demonstration, not a form).
    @MainActor
    static func prepare(_ surface: NSHSurface) {
        HomeNSHComponents.register(on: surface)
        surface.register(component: "moshpit/connection-password") { context in
            NSHSecretField(text: "", context: context, filled: { !$0.isEmpty }) { _ in }
        }
    }

    func makeUIView(context: Context) -> UIView {
        guard let surface = NSHSurface.make() else { return UIView() }
        Self.prepare(surface)
        let light = light
        surface.onWindow = { surface in
            if !surface.run(screen: "ui-gallery", params: Self.params(light: light)) {
                Log.ui.error("NSH screen ui-gallery did not run")
            }
        }
        return surface
    }

    func updateUIView(_ uiView: UIView, context: Context) {}
}
