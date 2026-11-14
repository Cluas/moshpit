import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// B293: the app's root carried `.id(appThemeId + accentHex)`, so saving
/// the accent in use made the whole app again — every navigation stack
/// popped, every sheet (Settings, where the accent is edited) closed. The
/// accent reaches SwiftUI's views through Observation (`Ink.accent` reads
/// `AppSettings.shared.appThemeId` and `AppThemeStore.shared`), and NSH
/// pages, which are told their colors when they run, run again on their
/// own (``NSHPage``): the root stays.
@Suite("An accent change keeps the app as it is", .serialized)
@MainActor
struct AccentRebuildTests {

    /// Counts how often its content was made (a new identity runs
    /// `onAppear` again and gets a new `@State`).
    final class Made {
        var tokens: [UUID] = []
    }

    struct Probe: View {
        let made: Made
        @State private var token = UUID()
        var body: some View {
            Text("probe").foregroundStyle(Ink.accent)
                .onAppear { made.tokens.append(token) }
        }
    }

    /// `content` in a window of the test host's scene, until `done`.
    private static func host(_ content: some View) -> (UIWindow, () -> Void) {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) }
            ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 744, height: 1133))
        window.rootViewController = UIHostingController(rootView: content)
        window.isHidden = false
        return (window, { window.isHidden = true; window.rootViewController = nil })
    }

    /// Runs `body` with the accent switched away from what it was, and back.
    private static func switchingAccent(_ body: (_ switchAccent: () -> Void) async throws -> Void) async rethrows {
        let settings = AppSettings.shared
        let before = settings.appThemeId
        defer { settings.appThemeId = before }
        let other = before == AppThemeCatalog.amberConsole.id
            ? AppThemeCatalog.terminalGreen.id : AppThemeCatalog.amberConsole.id
        try await body { settings.appThemeId = other }
    }

    private static func settle(_ condition: () -> Bool) async {
        let deadline = Date().addingTimeInterval(NSHTestWait.ceiling)
        while !condition(), Date() < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test("the root's theming keeps what is under it; the old `.id` made it again")
    func theRootStays() async throws {
        try await Self.switchingAccent { switchAccent in
            let made = Made()
            let (_, done) = Self.host(Probe(made: made).appThemeRoot())
            defer { done() }
            await Self.settle { made.tokens.count == 1 }
            #expect(made.tokens.count == 1)
            switchAccent()
            try await Task.sleep(for: .milliseconds(500))
            #expect(made.tokens.count == 1, "the accent made the root again: Settings would close")

            // The control: the root as it was, keyed on the accent, is made again.
            let old = Made()
            let (_, oldDone) = Self.host(OldRoot(made: old))
            defer { oldDone() }
            await Self.settle { old.tokens.count == 1 }
            AppSettings.shared.appThemeId = AppThemeCatalog.signalRoom.id
            await Self.settle { old.tokens.count == 2 }
            #expect(old.tokens.count == 2, "the probe can't tell a root made again")
        }
    }

    /// B293's root: `.id(appThemeId + accentHex)`.
    struct OldRoot: View {
        let made: Made
        var body: some View {
            Probe(made: made)
                .tint(Ink.accent)
                .id(AppSettings.shared.appThemeId + AppThemeCatalog.current.accentHex)
        }
    }

    @Test("an NSH page runs again in the new accent, from its host's model")
    func anNSHPageRunsAgain() async throws {
        try await Self.switchingAccent { switchAccent in
            final class Attached { var runs: [(NSHSurface, String)] = [] }
            let box = Attached()
            let page = NSHPage(screen: .recentLog, attach: { surface in
                box.runs.append((surface, NSHScreenParams.themeKey))
            })
            let (_, done) = Self.host(page)
            defer { done() }
            await Self.settle { box.runs.count == 1 }
            try #require(box.runs.count == 1, "the page never ran")
            switchAccent()
            await Self.settle { box.runs.count == 2 }
            try #require(box.runs.count == 2, "the page kept the old accent")
            #expect(box.runs[1].0 !== box.runs[0].0, "a new surface, told the new colors")
            #expect(box.runs[1].1 != box.runs[0].1)
            #expect(box.runs[1].1 == NSHScreenParams.themeKey)
        }
    }
}
