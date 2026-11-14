import SwiftUI
import UIKit

/// Settings › App Icon: the SwiftUI page (``AppIconGalleryView``), or the NSH
/// one while its switch is on (``NSHScreen/appIcon``). With the switch off
/// the SwiftUI page is all there is: no NSH code runs.
struct AppIconPage: View {
    var body: some View {
        if NSHScreen.appIcon.isOn {
            AppIconNSHPage()
        } else {
            AppIconGalleryView()
                .darkOnlyPage()
        }
    }
}

/// Settings › App Icon drawn by NSH (`ui/src/app_icon.rs`). The icons and the
/// one in use come from the host (``AppIconModel``). Picking one stays the
/// host's to do: iOS switches the icon (`setAlternateIconName`), the setting
/// follows only once iOS has, and the screen shows the outcome — the new
/// pick, or why iOS refused it. The tick is the icon iOS shows, not the
/// stored setting the SwiftUI page ticks (``AppIconModel/selectedID(_:)``).
/// If NSH cannot run here, the SwiftUI page shows instead.
struct AppIconNSHPage: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(AppSettings.self) private var settings
    @State private var model = AppIconModel()
    @State private var failed = false

    var body: some View {
        if failed {
            AppIconGalleryView()
                .darkOnlyPage()
        } else {
            NSHPage(
                screen: .appIcon,
                attach: { model.attach($0, settings: settings) },
                onMessage: { message in
                    switch message["ev"] as? String {
                    case "back":
                        dismiss()
                    case "select":
                        if let id = message["id"] as? String { model.select(id, settings: settings) }
                    default:
                        break
                    }
                },
                onFailure: { failed = true })
                .nshPage()
                // The title the next page's back button would show.
                .navigationTitle("App Icon")
        }
    }
}

/// What the NSH App Icon page shows, owned by the host so a surface made
/// again starts from it (``NSHPage``'s third rule), and the message that
/// carries it (`{"op": "icons", …}`, `ui/src/app_icon.rs`'s `Message`).
@MainActor
final class AppIconModel {
    /// Why iOS refused the last change (its description), until one works.
    private(set) var failure: String?
    private weak var surface: NSHSurface?

    /// How the icon is switched: iOS's `setAlternateIconName` (`nil` the
    /// primary icon), or a test's stand-in. `done` gets iOS's error.
    var setIcon: (_ alternateName: String?, _ done: @escaping @Sendable (Error?) -> Void) -> Void = { name, done in
        UIApplication.shared.setAlternateIconName(name, completionHandler: done)
    }

    /// The icon iOS shows on the home screen: its `alternateIconName` (`nil`
    /// the primary icon), or a test's stand-in.
    var shownIcon: () -> String? = { UIApplication.shared.alternateIconName }

    /// The icon to tick: the one iOS shows, which is what the home screen
    /// has; the stored setting only when iOS names an icon the catalog does
    /// not have. The SwiftUI page ticks the setting, which can name no icon
    /// of the catalog (then it ticks none) or one iOS does not show.
    func selectedID(_ settings: AppSettings) -> String {
        let shown = shownIcon()
        return AppIconCatalog.all.first { $0.alternateName == shown }?.id ?? settings.appIconId
    }

    /// `surface` shows the icons from now on, starting with the current
    /// state.
    func attach(_ surface: NSHSurface, settings: AppSettings) {
        self.surface = surface
        post(settings)
    }

    /// The user picked icon `id`: iOS switches to it, then the setting
    /// follows; a refusal is shown, and the setting kept, so the checkmark
    /// never disagrees with the home screen.
    func select(_ id: String, settings: AppSettings) {
        guard id != selectedID(settings),
              let option = AppIconCatalog.all.first(where: { $0.id == id })
        else { return }
        setIcon(option.alternateName) { [weak self] error in
            let detail = error?.localizedDescription
            Task { @MainActor in
                guard let self else { return }
                self.failure = detail
                if detail == nil { settings.appIconId = option.id }
                self.post(settings)
            }
        }
    }

    private func post(_ settings: AppSettings) {
        surface?.post(Self.message(selected: selectedID(settings), failure: failure))
    }

    /// The screen's message: every icon, the one in use, and why the last
    /// change failed, if it did.
    nonisolated static func message(selected: String, failure: String?) -> [String: Any] {
        var message: [String: Any] = [
            "op": "icons",
            "icons": AppIconCatalog.all.map(icon),
            "selected": selected,
        ]
        if let failure {
            message["failure"] = failure
        }
        return message
    }

    /// One icon as the screen takes it: its preview, when the bundle has it,
    /// as the file the SwiftUI page loads with `UIImage(named:)`.
    nonisolated static func icon(_ option: AppIconOption) -> [String: Any] {
        var icon: [String: Any] = ["id": option.id, "name": option.name]
        if let image = previewURI(option) {
            icon["image"] = image
        }
        return icon
    }

    /// The preview of `option` as an NSH `asset:/` URI: a path in `bundle`'s
    /// resources, where a surface finds its assets (`ui/src/ios.rs`). nil
    /// when the bundle has no preview (an asset renamed without
    /// regenerating): the screen draws its placeholder, as the SwiftUI page
    /// does.
    nonisolated static func previewURI(_ option: AppIconOption, in bundle: Bundle = .main) -> String? {
        guard let url = bundle.url(forResource: "\(option.previewAsset)@3x", withExtension: "png"),
              let resources = bundle.resourceURL
        else { return nil }
        let path = url.standardizedFileURL.path
        let root = resources.standardizedFileURL.path
        guard path.hasPrefix(root + "/") else { return nil }
        return "asset:/" + path.dropFirst(root.count + 1)
    }
}
