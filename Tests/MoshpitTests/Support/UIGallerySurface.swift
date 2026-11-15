@testable import Moshpit

/// How the tests run the `ui-gallery` screen (`ui/src/screen.rs`, every
/// component of the `ui` layer in its variants, sizes and states): the
/// developer page that hosted it in the app had no way in any more (E6a),
/// so only its setup lives on, here.
enum UIGallerySurface {
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
}
