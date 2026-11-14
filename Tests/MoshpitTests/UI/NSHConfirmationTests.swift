import Foundation
import Testing
import UIKit
@testable import Moshpit

/// B498: a confirmation an NSH page's long-press menu asks hangs from the
/// row the menu came from — never from the page, which from iOS 26 put an
/// iPhone's popover near the top of the screen, over the toolbar, pointing
/// at nothing ("Kill pane 2?" on Canary 130).
@Suite("NSH confirmations hang from their row")
@MainActor
struct NSHConfirmationTests {
    /// A surface in a window with one row, as the NSH Home lays a pane row
    /// out: identified, lower down the page.
    private func page() -> (UIWindow, UIView, UIView) {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        let surface = UIView(frame: window.bounds)
        let row = UIView(frame: CGRect(x: 16, y: 620, width: 358, height: 56))
        row.accessibilityIdentifier = "home-pane-C-w1:p2"
        surface.addSubview(row)
        window.addSubview(surface)
        window.isHidden = false
        return (window, surface, row)
    }

    @Test("the menu's row is the one a confirmation points at")
    func menuRemembersItsRow() {
        let (window, surface, row) = page()
        defer { window.isHidden = true }
        let menus = NSHContextMenus()
        menus.menus = [row.accessibilityIdentifier!: { [] }]
        menus.install(on: surface)
        #expect(menus.openedView == nil, "no menu opened yet")
        let config = menus.contextMenuInteraction(UIContextMenuInteraction(delegate: menus),
                                                  configurationForMenuAtLocation: CGPoint(x: 100, y: 640))
        #expect(config != nil)
        #expect(menus.openedView === row)
        #expect(menus.surfaceView === surface)
        row.removeFromSuperview()
        #expect(menus.openedView == nil, "a row no longer on screen is no anchor")
    }

    @Test("with its row on screen it hangs from the row, on a phone and an iPad alike")
    func hangsFromTheRow() {
        let (window, surface, row) = page()
        defer { window.isHidden = true }
        for idiom in [UIUserInterfaceIdiom.phone, .pad] {
            #expect(NSHConfirmation.placement(source: row, page: surface, idiom: idiom) == .row(row.bounds))
        }
        let alert = NSHConfirmation.sheet(title: "Kill pane 2?", message: nil,
                                          actions: [.init(title: "Kill Pane", destructive: true) {}],
                                          cancelTitle: "Cancel", cancel: {})
        NSHConfirmation.hang(alert, .row(row.bounds), source: row, page: surface)
        let popover = alert.popoverPresentationController
        #expect(popover == nil || popover?.sourceView === row)
        #expect(popover == nil || popover?.sourceRect == row.bounds)
    }

    @Test("with no row a phone's sheet comes from the bottom, an iPad's from the page's bottom middle")
    func noRowNoRandomSpot() {
        let (window, surface, row) = page()
        defer { window.isHidden = true }
        row.removeFromSuperview()
        #expect(NSHConfirmation.placement(source: row, page: surface, idiom: .phone) == .bottom)
        #expect(NSHConfirmation.placement(source: nil, page: surface, idiom: .phone) == .bottom)
        #expect(NSHConfirmation.placement(source: nil, page: surface, idiom: .pad)
                == .pageBottom(CGRect(x: 195, y: 844, width: 0, height: 0)))
    }

    @Test("the sheet: the destructive action, then Cancel, which also runs for a tap outside")
    func sheetShape() {
        var killed = 0, cancelled = 0
        let alert = NSHConfirmation.sheet(
            title: "Kill pane 2?", message: nil,
            actions: [.init(title: "Kill Pane", destructive: true) { killed += 1 }],
            cancelTitle: "Cancel", cancel: { cancelled += 1 })
        #expect(alert.preferredStyle == .actionSheet)
        #expect(alert.title == "Kill pane 2?")
        #expect(alert.actions.map(\.title) == ["Kill Pane", "Cancel"])
        #expect(alert.actions.map(\.style) == [.destructive, .cancel])
        _ = killed
        _ = cancelled
    }
}
