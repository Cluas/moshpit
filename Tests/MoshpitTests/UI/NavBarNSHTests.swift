import Foundation
import Testing
import UIKit
@testable import Moshpit

/// B316 on the device: the bar of Home, Settings and Edit Connection sets
/// itself off from the rows scrolled under it (iOS 26's scroll edge effect
/// on glass, the chrome material below iOS 26), has nothing at the top of
/// its list, and brings its look in as the list moves (bound to the list's
/// offset natively: half of it half way through `bar-edge-appear`). Each
/// page runs at an iPhone's size (402 × 874 pt, the owner's) in a window of
/// the test host; every state is also drawn to the app's tmp folder as
/// `bars-<page>-<state>-<look>.png`, the pictures the change is checked by.
/// The run with `MOSHPIT_PREVIEW_GLASS` in the environment
/// (`TEST_RUNNER_MOSHPIT_PREVIEW_GLASS=1` for xcodebuild) draws iOS 26's
/// bar on this iOS 18 device; without it, iOS 18's.
@Suite("NSH bars on the device")
@MainActor
struct NavBarNSHTests {
    /// The owner's iPhone, in points.
    static let phone = CGRect(x: 0, y: 0, width: 402, height: 874)

    /// Whether this run draws iOS 26's bar.
    static var glass: Bool {
        ProcessInfo.processInfo.environment["MOSHPIT_PREVIEW_GLASS"] != nil
            || ProcessInfo.processInfo.operatingSystemVersion.majorVersion >= 26
    }

    /// The view that sets the bar off: the scroll edge effect on glass,
    /// the classic background below.
    static var look: String { glass ? "nav-bar-edge" : "nav-bar-background" }

    /// A surface the owner's iPhone's size, in a window of the test host's
    /// scene, running `screen` (on its `page`: a page of Settings).
    static func surface(running screen: NSHScreen, page: String? = nil,
                        register: (NSHSurface) -> Void = { _ in }) throws -> (NSHSurface, () -> Void) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        // Dark, as the app's hosts force it until R18's host part: Home and
        // Edit Connection follow the window's appearance now (R18 part A),
        // and the bars' pictures are of the dark app whatever the device's.
        window.overrideUserInterfaceStyle = .dark
        let surface = try #require(NSHSurface.make(frame: phone))
        register(surface)
        window.addSubview(surface)
        window.isHidden = false
        var params = NSHScreenParams.current()
        if let page { params["page"] = page }
        #expect(surface.run(screen: screen.name, params: params))
        return (surface, {
            surface.removeFromSuperview()
            window.isHidden = true
        })
    }

    /// The page's list: the scroll view `id`.
    static func list(_ id: String, in surface: UIView) async throws -> UIScrollView {
        let view = try #require(await SettingsNSHTests.view(identified: id, in: surface))
        return try #require((view as? UIScrollView) ?? view.subviews.compactMap { $0 as? UIScrollView }.first)
    }

    /// Scrolls `list` to `y` (as a finger would leave it) and gives the
    /// page a moment to follow.
    static func scroll(_ list: UIScrollView, to y: CGFloat) async {
        list.setContentOffset(CGPoint(x: 0, y: y), animated: false)
        try? await Task.sleep(for: .milliseconds(400))
    }

    /// How much of the bar's look shows: 0 when it is not made or hidden.
    static func shown(_ surface: UIView) -> CGFloat {
        guard let view = AccentRowMenus.view(identified: look, in: surface), !view.isHidden else { return 0 }
        return view.alpha
    }

    /// Whether the other OS's look was made (never).
    static func otherMade(_ surface: UIView) -> Bool {
        AccentRowMenus.view(identified: glass ? "nav-bar-background" : "nav-bar-edge", in: surface) != nil
    }

    /// Draws the window where `surface` is (the blur is drawn only with the
    /// window's whole hierarchy) to the app's tmp folder as
    /// `bars-<name>-<look>.png`.
    static func shoot(_ surface: UIView, _ name: String) {
        guard let window = surface.window else { return }
        let frame = surface.convert(surface.bounds, to: window)
        let image = UIGraphicsImageRenderer(bounds: frame).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("bars-\(name)-\(glass ? "glass" : "classic").png")
        try? image.pngData()?.write(to: url)
    }

    /// Settings' hub (R8) wears Home's large title: its bar's look comes in
    /// only once the title has gone, as Home's does.
    @Test("Settings' hub: nothing under the large title, the look once it has gone")
    func settingsHub() async throws {
        let (settings, _, _, _) = SettingsRootNSHTests.scratch()
        let (surface, done) = try Self.surface(running: .settings)
        defer { done() }
        #expect(surface.post(.settings(state: SettingsRootNSHTests.snapshot(settings))))
        let list = try await Self.list("settings-list", in: surface)
        try? await Task.sleep(for: .milliseconds(400))
        Self.shoot(surface, "settings-hub-top")
        #expect(Self.shown(surface) < 0.01, "nothing under the large title")
        #expect(!Self.otherMade(surface))
        await Self.scroll(list, to: 160)
        Self.shoot(surface, "settings-hub-scrolled")
        #expect(Self.shown(surface) > 0.99, "the look once the large title has gone")
        await Self.scroll(list, to: 0)
        #expect(Self.shown(surface) < 0.01, "nothing back under the large title")
    }

    /// A page of Settings (R8: Terminal, which has the cursor's group;
    /// Connections, which has Mosh's) wears the bar every page does.
    @Test("Settings' pages: nothing at the list's top, the look coming in with the list, whole over the rows")
    func settings() async throws {
        let (settings, _, _, _) = SettingsRootNSHTests.scratch()
        for (page, anchor) in [("terminal", "cursor"), ("connections", "mosh")] {
            let (surface, done) = try Self.surface(running: .settings, page: page)
            defer { done() }
            #expect(surface.post(.settings(state: SettingsRootNSHTests.snapshot(settings))))
            let list = try await Self.list("settings-list", in: surface)
            try? await Task.sleep(for: .milliseconds(400))
            Self.shoot(surface, "settings-\(page)-top")
            #expect(Self.shown(surface) == 0, "\(page): nothing at the top")
            #expect(!Self.otherMade(surface))
            // Half way through the theme's 8 pt: half of the look, natively.
            await Self.scroll(list, to: 4)
            let half = Self.shown(surface)
            #expect(half > 0.2 && half < 0.8, "\(page): half way in: \(half)")
            await Self.scroll(list, to: 160)
            Self.shoot(surface, "settings-\(page)-scrolled")
            #expect(Self.shown(surface) > 0.99, "\(page): the look over the rows")
            await Self.scroll(list, to: 0)
            #expect(Self.shown(surface) == 0, "\(page): nothing back at the top")
            // The anchor's group under the bar. Each page fits an iPhone's
            // screen, so the list stays where it is: the look follows where
            // the reveal leaves it.
            #expect(surface.post(["op": "reveal", "anchor": anchor]))
            try? await Task.sleep(for: .milliseconds(500))
            Self.shoot(surface, "settings-\(anchor)")
            let shown = Self.shown(surface)
            let at = list.contentOffset.y
            #expect(at >= 8 ? shown > 0.99 : shown < 0.01, "\(anchor) at \(at): \(shown)")
        }
    }

    @Test("Edit Connection: the look over the groups scrolled under the bar")
    func connectionForm() async throws {
        let (surface, done) = try Self.surface(running: .connectionForm) { HomeNSHComponents.register(on: $0) }
        defer { done() }
        let model = ConnectionFormModel(draft: ConnectionDraft(ConnectionFormNSHTests.connection()))
        model.attach(surface, settings: AppSettings.shared, keys: [])
        let list = try await Self.list("connection-form-list", in: surface)
        try? await Task.sleep(for: .milliseconds(400))
        Self.shoot(surface, "form-top")
        #expect(Self.shown(surface) == 0, "nothing at the top")
        await Self.scroll(list, to: 330)
        Self.shoot(surface, "form-scrolled")
        #expect(Self.shown(surface) > 0.99, "the look over the groups")
    }

    @Test("Home: nothing under the large title, the look once it has gone")
    func home() async throws {
        let (surface, done) = try Self.surface(running: .home) { HomeNSHComponents.register(on: $0) }
        defer { done() }
        #expect(surface.post(HomeNSHTests.message()))
        let add = try #require(await SettingsNSHTests.view(identified: "home-add", in: surface))
        try? await Task.sleep(for: .milliseconds(400))
        // The ＋ is R2's 44 pt icon button (R4): it takes a touch anywhere
        // in its 44 pt square, as UIKit's bar button does, up to the gear
        // beside it (NSH e78cce60 changed how a child's slop meets its
        // parent's bounds; the square needs no slop).
        let window = try #require(surface.window)
        // Measured once the bar has settled: run 13 measured ＋ at
        // (52, 0) — the leading side, mid-layout — and every point inside
        // it reached the view laid over it then.
        var frame = add.convert(add.bounds, to: window)
        for _ in 0..<20 {
            try? await Task.sleep(for: .milliseconds(100))
            let now = add.convert(add.bounds, to: window)
            if now == frame { break }
            frame = now
        }
        #expect(frame.width >= 43.5 && frame.height >= 43.5, "＋: \(frame)")
        #expect(frame.midX > surface.frame.midX, "＋ is on the bar's leading side: \(frame) in \(surface.frame)")
        for (side, point) in [("left", CGPoint(x: frame.minX + 3, y: frame.midY)),
                              ("right", CGPoint(x: frame.maxX - 3, y: frame.midY)),
                              ("top", CGPoint(x: frame.midX, y: frame.minY + 3)),
                              ("bottom", CGPoint(x: frame.midX, y: frame.maxY - 3))] {
            let hit = window.hitTest(point, with: nil)
            #expect(hit === add || hit?.isDescendant(of: add) == true,
                    "3 pt inside ＋'s \(side) edge (\(frame)) reached \(hit.map { "\(type(of: $0))" } ?? "nothing")")
        }
        Self.shoot(surface, "home-top")
        #expect(Self.shown(surface) < 0.01, "nothing under the large title")
        let list = try #require(Self.scrollView(in: surface))
        await Self.scroll(list, to: 240)
        Self.shoot(surface, "home-scrolled")
        #expect(Self.shown(surface) > 0.99, "the look once the large title has gone")
    }

    /// `id`'s frame in its window (through the transforms around it).
    static func frame(_ id: String, in surface: UIView) -> CGRect? {
        guard let view = AccentRowMenus.view(identified: id, in: surface), let window = surface.window else { return nil }
        return view.convert(view.bounds, to: window)
    }

    /// How opaque `id` is drawn: its own alpha (what a binding sets).
    static func alpha(_ id: String, in surface: UIView) -> CGFloat {
        AccentRowMenus.view(identified: id, in: surface)?.alpha ?? -1
    }

    /// The list let go where it is, as a finger lifting with no fling:
    /// UIKit's end of a drag, which NSH reads as the list coming to rest.
    static func letGo(_ list: UIScrollView) async {
        list.delegate?.scrollViewDidEndDragging?(list, willDecelerate: false)
        try? await Task.sleep(for: .milliseconds(900))
    }

    @Test("Home: the large title stretches, collapses and settles with the list (R4)")
    func homeLargeTitle() async throws {
        let (surface, done) = try Self.surface(running: .home) { HomeNSHComponents.register(on: $0) }
        defer { done() }
        // A dozen more saved hosts: a list that fits the screen cannot stay
        // collapsed (NSH, like UIKit, scrolls no further than its end).
        // Home's record is the core's JSON since the move to the core: add the
        // saved cards to its connections.
        var message = HomeNSHTests.message()
        var connections = message["connections"] as? [Any] ?? []
        let saved = connections.count > 1 ? connections[1] as? [String: Any] ?? [:] : [:]
        for n in 0..<12 {
            var card = saved
            card["id"] = UUID().uuidString
            card["name"] = "saved-\(n)"
            card["initial"] = "S"
            connections.append(card)
        }
        message["connections"] = connections
        #expect(surface.post(message))
        _ = try #require(await SettingsNSHTests.view(identified: "home-add", in: surface))
        try? await Task.sleep(for: .milliseconds(400))
        let list = try #require(Self.scrollView(in: surface))
        #expect(list.contentSize.height - list.bounds.height > 200, "the list scrolls: \(list.contentSize)")
        let stretch = try #require(AccentRowMenus.view(identified: "nav-bar-large-stretch", in: surface))
        // The collapse is the large title's row (52 pt at the default text
        // size), the view the title's bindings sit in.
        let collapse = try #require(stretch.superview?.bounds.height)
        #expect(collapse > 40 && collapse < 80, "the large title's row: \(collapse)")
        let rest = try #require(Self.frame("nav-bar-title", in: surface))
        let inlineRest = try #require(Self.frame("nav-bar-inline-title", in: surface))
        #expect(abs(rest.minX - 16) < 0.5, "the large title at space-page: \(rest)")
        #expect(Self.alpha("nav-bar-large-shrink", in: surface) > 0.99)
        #expect(Self.alpha("nav-bar-inline-title", in: surface) < 0.01)

        // (a) Pulled 32 pt past the top: half of the 64 pt stretch, from
        // the leading edge.
        await Self.scroll(list, to: -32)
        Self.shoot(surface, "home-pulled")
        let pulled = try #require(Self.frame("nav-bar-title", in: surface))
        #expect(abs(pulled.width / rest.width - 1.04) < 0.01, "pulled: \(pulled) from \(rest)")
        #expect(abs(pulled.minX - rest.minX) < 0.5, "the leading edge stays: \(pulled)")

        // (b) The large title fades from 40% of the collapse until its
        // baseline, 17 pt above its row's bottom, reaches the bar. At 60%
        // most of the way out; the inline title not in yet, 4 pt low.
        let gone = collapse - 17
        await Self.scroll(list, to: collapse * 0.6)
        let large60 = Self.alpha("nav-bar-large-shrink", in: surface)
        let fade60 = 1 - (collapse * 0.2) / (gone - collapse * 0.4)
        #expect(abs(large60 - fade60) < 0.05, "60%: the large title at \(large60), not \(fade60)")
        #expect(Self.alpha("nav-bar-inline-title", in: surface) < 0.01)
        let low = try #require(Self.frame("nav-bar-inline-title", in: surface))
        // 80%: its baseline under the bar, the large title gone (no
        // descenders' tips left under the bar) and at 0.968; the inline
        // title half in, half way up.
        await Self.scroll(list, to: collapse * 0.8)
        Self.shoot(surface, "home-collapsing")
        let large80 = Self.alpha("nav-bar-large-shrink", in: surface)
        let inline80 = Self.alpha("nav-bar-inline-title", in: surface)
        #expect(large80 < 0.01, "80%: the large title at \(large80)")
        #expect(abs(inline80 - 0.5) < 0.05, "80%: the inline title at \(inline80)")
        let shrunk = try #require(Self.frame("nav-bar-title", in: surface))
        #expect(abs(shrunk.width / rest.width - 0.968) < 0.01, "80%: \(shrunk) from \(rest)")
        #expect(abs(shrunk.minX - rest.minX) < 0.5, "the leading edge stays: \(shrunk)")
        #expect(Self.shown(surface) < 0.01, "no look before the last bar-edge-appear points")
        // 100%: the large title gone, the inline title in, risen 4 pt.
        await Self.scroll(list, to: collapse)
        #expect(Self.alpha("nav-bar-large-shrink", in: surface) < 0.01)
        #expect(Self.alpha("nav-bar-inline-title", in: surface) > 0.99)
        let up = try #require(Self.frame("nav-bar-inline-title", in: surface))
        #expect(abs(low.minY - up.minY - 4) < 0.5, "the inline title rises 4 pt: \(low) to \(up)")
        #expect(abs(inlineRest.minY - up.minY - 4) < 0.5, "4 pt above where it waited: \(up), \(inlineRest)")
        #expect(Self.shown(surface) > 0.99, "the look once the large title has gone")

        // (c) Let go part way: to the nearer end. Past the collapse: left.
        await Self.scroll(list, to: collapse * 0.4)
        await Self.letGo(list)
        #expect(abs(list.contentOffset.y) < 0.5, "under half: back to the top, at \(list.contentOffset.y)")
        await Self.scroll(list, to: collapse * 0.6)
        await Self.letGo(list)
        #expect(abs(list.contentOffset.y - collapse) < 0.5, "past half: collapsed, at \(list.contentOffset.y)")
        await Self.scroll(list, to: collapse + 40)
        await Self.letGo(list)
        #expect(abs(list.contentOffset.y - collapse - 40) < 0.5, "past the collapse: left, at \(list.contentOffset.y)")
    }

    /// The largest scroll view in `root` (Home's list).
    static func scrollView(in root: UIView) -> UIScrollView? {
        func all(_ v: UIView) -> [UIView] { v.subviews.flatMap { [$0] + all($0) } }
        return all(root).compactMap { $0 as? UIScrollView }
            .max { $0.bounds.width * $0.bounds.height < $1.bounds.width * $1.bounds.height }
    }
}
