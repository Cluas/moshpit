import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// B843: a page NSH draws, in the iPad split view's detail column, sits
/// beside the sidebar (iOS 26 floats it over the column as a leading
/// safe-area inset), and takes the whole width anywhere else. The window
/// is the iPad Pro 13"'s in landscape, the sidebar the shots' 380 pt.
@MainActor
@Suite("NSH page safe area (B843)")
struct NSHPageSafeAreaTests {
    final class Probe { var frame: CGRect = .zero }

    struct Page: View {
        let probe: Probe
        var body: some View {
            Color.black
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { probe.frame = $0 }
                .modifier(NSHPageSafeArea())
        }
    }

    private func frame(keepsSides: Bool?) -> CGRect {
        let probe = Probe()
        let page = Page(probe: probe)
        let root = keepsSides.map { AnyView(page.environment(\.nshKeepsSideInsets, $0)) } ?? AnyView(page)
        let host = UIHostingController(rootView: root)
        host.additionalSafeAreaInsets = UIEdgeInsets(top: 0, left: 380, bottom: 0, right: 0)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 1376, height: 1032))
        window.rootViewController = host
        window.makeKeyAndVisible()
        host.view.frame = window.bounds
        host.view.setNeedsLayout()
        host.view.layoutIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
        host.view.layoutIfNeeded()
        defer { window.isHidden = true }
        return probe.frame
    }

    @Test("In the split view's detail column the page sits beside the sidebar")
    func besideTheSidebar() {
        let f = frame(keepsSides: true)
        #expect(f.minX == 380 && f.width == 996, "\(f)")
    }

    @Test("Anywhere else the page takes the whole width, as it always has")
    func elsewhere() {
        #expect(frame(keepsSides: nil).width == 1376)
        #expect(frame(keepsSides: false).width == 1376)
    }

    @Test("Top and bottom (the status bar, the home indicator, the keyboard) are ignored either way")
    func vertical() {
        #expect(NSHPageSafeArea.ignored(keepsSides: true) == .vertical)
        #expect(NSHPageSafeArea.ignored(keepsSides: false) == .all)
    }
}
