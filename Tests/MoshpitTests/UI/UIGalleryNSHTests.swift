import Foundation
import Testing
import UIKit
@testable import Moshpit

/// R2 and R3 on the device: the redesign's `ui` layer gallery
/// (`ui/src/ui/gallery.rs`, docs/nsh/UI-DESIGN.md §4.20) runs on a real
/// surface at the owner's iPhone's size (402 × 874 pt) in both appearances,
/// its live controls mounted. Each appearance is drawn a phone screen at a
/// time, top to bottom, to the app's tmp folder as
/// `ui-gallery-<dark|light>-<n>.png` (and its top and bottom as `-top`,
/// `-bottom`), the pictures R2 and R3 are checked by. (`ui/tests/` checks
/// the components headless: touch targets, names, lanes, states.)
@Suite("NSH UI gallery on the device")
@MainActor
struct UIGalleryNSHTests {
    /// Draws the window where `surface` is to the app's tmp folder as
    /// `ui-gallery-<name>.png`.
    static func shoot(_ surface: UIView, _ name: String) {
        guard let window = surface.window else { return }
        let frame = surface.convert(surface.bounds, to: window)
        let image = UIGraphicsImageRenderer(bounds: frame).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("ui-gallery-\(name).png")
        try? image.pngData()?.write(to: url)
    }

    /// The first `T` under `root` whose accessibility identifier is `id`.
    static func control<T: UIView>(_ type: T.Type, identified id: String, in root: UIView) -> T? {
        if let hit = root as? T, hit.accessibilityIdentifier == id { return hit }
        for child in root.subviews {
            if let hit = control(type, identified: id, in: child) { return hit }
        }
        return nil
    }

    /// How much of `view` is inside its window: on an iPad in landscape a
    /// phone-sized surface reaches past the window's bottom.
    static func visibleHeight(of view: UIView) -> CGFloat {
        guard let window = view.window else { return view.bounds.height }
        let shown = view.convert(view.bounds, to: window).intersection(window.bounds).height
        return shown > 0 ? shown : view.bounds.height
    }

    /// Scrolls the gallery's list a page at a time until `find` finds what
    /// it looks for (NSH mounts what is about to show), or the list ends.
    static func scrolling<T>(_ list: UIScrollView, until find: () -> T?) async -> T? {
        let bottom = max(0, list.contentSize.height - list.bounds.height)
        let step = visibleHeight(of: list) * 0.9
        var offset: CGFloat = 0
        while true {
            list.setContentOffset(CGPoint(x: 0, y: min(offset, bottom)), animated: false)
            for _ in 0..<15 {
                if let hit = find() { return hit }
                try? await Task.sleep(for: .milliseconds(20))
            }
            if offset >= bottom { return nil }
            offset += step
        }
    }

    /// The gallery's list on `surface`.
    static func list(on surface: UIView) async throws -> UIScrollView {
        let root = try #require(await SettingsNSHTests.view(identified: "ui-gallery", in: surface))
        return try #require((root as? UIScrollView) ?? root.subviews.compactMap { $0 as? UIScrollView }.first)
    }

    /// The first secure text field under `root`.
    static func secureField(in root: UIView) -> UITextField? {
        if let field = root as? UITextField, field.isSecureTextEntry { return field }
        for child in root.subviews {
            if let hit = secureField(in: child) { return hit }
        }
        return nil
    }

    /// The gallery on a phone-sized surface in a window of its own, in
    /// `light` or dark.
    static func mount(light: Bool) throws -> (UIWindow, NSHSurface) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        // A phone's width, and its height or the window's if the window is
        // shorter (an iPad in landscape): every shot then holds all of it.
        var frame = NavBarNSHTests.phone
        frame.size.height = min(frame.height, window.bounds.height)
        let surface = try #require(NSHSurface.make(frame: frame))
        UIGallerySurface.prepare(surface)
        window.addSubview(surface)
        window.isHidden = false
        #expect(surface.run(screen: "ui-gallery", params: UIGallerySurface.params(light: light)))
        return (window, surface)
    }

    @Test("the gallery runs dark and light, its controls mounted, drawn a screen at a time")
    func gallery() async throws {
        for light in [false, true] {
            let look = light ? "light" : "dark"
            let (window, surface) = try Self.mount(light: light)
            defer {
                surface.removeFromSuperview()
                window.isHidden = true
            }
            let root = try #require(await SettingsNSHTests.view(identified: "ui-gallery", in: surface))
            for id in ["ui-gallery-presses", "ui-gallery-press-primary", "ui-gallery-press-icon-glass",
                       "ui-gallery-loading"] {
                #expect(await SettingsNSHTests.view(identified: id, in: surface) != nil, "\(look): \(id)")
            }
            // Further down, mounted as the list comes near them (NSH mounts
            // what is about to show): looked for at each page.
            var lower: Set<String> = ["ui-gallery-section-rows", "ui-gallery-row-nav", "ui-gallery-field-secret-empty",
                                      "ui-gallery-empty-page", "ui-gallery-banner-info", "ui-gallery-toast-show",
                                      "ui-gallery-swatch"]
            try? await Task.sleep(for: .milliseconds(600))
            Self.shoot(surface, "\(look)-top")
            let list = try #require((root as? UIScrollView) ?? root.subviews.compactMap { $0 as? UIScrollView }.first)
            let bottom = max(0, list.contentSize.height - list.bounds.height)
            #expect(bottom > 0, "\(look): the gallery is taller than a phone")
            var page = 0
            var offset: CGFloat = 0
            let step = Self.visibleHeight(of: list) * 0.9
            while true {
                list.setContentOffset(CGPoint(x: 0, y: min(offset, bottom)), animated: false)
                try? await Task.sleep(for: .milliseconds(400))
                Self.shoot(surface, "\(look)-\(page)")
                lower = lower.filter { AccentRowMenus.view(identified: $0, in: surface) == nil }
                if offset >= bottom { break }
                page += 1
                offset += step
            }
            #expect(lower.isEmpty, "\(look): never mounted while the gallery scrolled: \(lower.sorted())")
            Self.shoot(surface, "\(look)-bottom")
        }
    }

    /// R3's system controls are the host's: the switch on `primary` with
    /// its off track `muted` (the host's `offTint`: its background, rounded
    /// as the track), the segmented control and the select's menu.
    @Test("R3's switch, segmented control and select are mounted, the switch's off track colored")
    func systemControls() async throws {
        let (window, surface) = try Self.mount(light: false)
        defer {
            surface.removeFromSuperview()
            window.isHidden = true
        }
        let list = try await Self.list(on: surface)
        let toggle = await Self.scrolling(list) {
            Self.control(UISwitch.self, identified: "ui-gallery-switch", in: surface)
        }
        let control = try #require(toggle, "the switch is mounted")
        #expect(control.onTintColor != nil, "on is primary")
        #expect(control.backgroundColor != nil, "the off track has a color")
        #expect(control.layer.cornerRadius > 0, "rounded as the track")
        let segmented = await Self.scrolling(list) {
            Self.control(UISegmentedControl.self, identified: "ui-gallery-segmented", in: surface)
        }
        #expect(segmented != nil, "the segmented control is mounted")
        let menu = await Self.scrolling(list) {
            Self.control(UIButton.self, identified: "ui-gallery-select", in: surface)
        }
        #expect(menu != nil, "the select's menu is mounted")
    }

    /// NSH cannot see focus in a host's view: the secure field says it
    /// (`focus`, `blur`), and the `ui` layer's secret field rings itself
    /// while the field has the keyboard.
    @Test("the secret field rings itself while the host's field has the keyboard")
    func secretFieldFocus() async throws {
        let (window, surface) = try Self.mount(light: false)
        defer {
            surface.endEditing(true)
            surface.removeFromSuperview()
            window.isHidden = true
        }
        let list = try await Self.list(on: surface)
        let secret = try #require(await Self.scrolling(list) {
            AccentRowMenus.view(identified: "ui-gallery-field-secret-empty", in: surface)
        }, "the secret field is mounted")
        // Where the list stopped: the host's field mounts inside it.
        var found: UITextField?
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while found == nil, Date() < deadline {
            found = Self.secureField(in: secret)
            if found == nil { try? await Task.sleep(for: .milliseconds(20)) }
        }
        let field = try #require(found, "the host's secure field")
        let ring = "ui-gallery-field-secret-empty-ring"
        #expect(AccentRowMenus.view(identified: ring, in: surface) == nil, "no ring before the keyboard")
        #expect(field.becomeFirstResponder())
        #expect(await SettingsNSHTests.view(identified: ring, in: surface) != nil, "ringed while focused")
        field.resignFirstResponder()
        let gone = Date().addingTimeInterval(NSHTestWait.ceiling)
        while AccentRowMenus.view(identified: ring, in: surface) != nil, Date() < gone {
            try? await Task.sleep(for: .milliseconds(20))
        }
        #expect(AccentRowMenus.view(identified: ring, in: surface) == nil, "the ring goes with the keyboard")
    }
}
