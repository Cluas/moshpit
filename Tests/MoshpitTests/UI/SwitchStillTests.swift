import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Moshpit

/// A protocol switch (mosh ⇄ SSH) moves nothing (the owner: 「mosh 切换 ssh
/// 有个动画 我们直接去掉」). It swaps in a new session: before, the new
/// session started from "never live" and came up under the first connect's
/// animated poster, faded in and out; now it carries "was live", so it comes
/// up under the reconnect's veil, never the poster, with the pane's last
/// picture under it, and nothing fades: the covers, the pill's word and the
/// terminal's swap all happen at once.
@Suite("protocol switch, no motion")
@MainActor
struct SwitchStillTests {

    private static let states: [TransportConnState] = [.connecting, .reconnecting, .offline, .live]

    @Test("A switch's new session is a reconnect: the veil through every state, never the poster")
    func aSwitchNeverShowsThePoster() {
        let reconnect = TerminalScreen.liveAfterSwap(carried: true)
        #expect(reconnect)
        for state in Self.states {
            for settling in [false, true] {
                let cover = TerminalScreen.paneCover(resolved: true, reconnect: reconnect,
                                                     state: state, settling: settling)
                #expect(cover != .poster, "\(state), settling \(settling)")
            }
        }
        let connecting = TerminalScreen.paneCover(resolved: true, reconnect: true, state: .connecting, settling: false)
        #expect(connecting == .veil)
        let painting = TerminalScreen.paneCover(resolved: true, reconnect: true, state: .live, settling: true)
        #expect(painting == .veil, "held until the new frame has painted")
        let painted = TerminalScreen.paneCover(resolved: true, reconnect: true, state: .live, settling: false)
        #expect(painted == .none)
    }

    @Test("A first connect still gets the poster; a session not resolved yet gets nothing")
    func aFirstConnectKeepsThePoster() {
        let first = TerminalScreen.liveAfterSwap(carried: false)
        #expect(!first)
        let connecting = TerminalScreen.paneCover(resolved: true, reconnect: first, state: .connecting, settling: false)
        #expect(connecting == .poster)
        let offline = TerminalScreen.paneCover(resolved: true, reconnect: first, state: .offline, settling: false)
        #expect(offline == .none, "nothing attempted: no poster")
        let unresolved = TerminalScreen.paneCover(resolved: false, reconnect: false, state: .connecting, settling: false)
        #expect(unresolved == .none)
    }

    @Test("The cover stack does not animate through a switch, and does otherwise")
    func theCoversMoveOnlyOutsideASwitch() {
        let still = TerminalScreen.coverAnimation(still: true)
        #expect(still == nil)
        let moving = TerminalScreen.coverAnimation(still: false)
        #expect(moving != nil)
    }

    @Test("The NSH chrome tells the page a switch is under way")
    func theChromeSaysStill() {
        var chrome = TerminalNSHChrome()
        chrome.still = true
        let message = chrome.message
        #expect(message["still"] as? Bool == true)
        #expect(NSHSurface.check(screen: NSHScreen.terminal.name, message: message), "the screen takes it")
        chrome.still = false
        let ordinary = chrome.message
        #expect(ordinary["still"] as? Bool == false)
    }

    // MARK: The stage (UIKit, on the device)

    @Test("A switch's swap is at once: the old terminal leaves the slot in the same turn")
    func theStageSwapsAtOnce() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let first = try #require(stage.terminal)
        stage.show(.init(source: .pane(controller, "%2"), inset: 4), instant: true)
        let second = try #require(stage.terminal)
        #expect(second !== first)
        #expect(first.window == nil, "no fade: gone at once")
        let container = try #require(stage.container)
        #expect(container.alpha == 1)
        #expect(container.superview === slot)
    }

    @Test("A switch keeps the pane's picture on top of the slot until it is released")
    func theStageHoldsThePicture() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        stage.holdStill()
        let still = try #require(stage.still)
        #expect(still.superview === slot)
        // The new session's terminal comes in under the picture.
        stage.show(.init(source: .pane(controller, "%2"), inset: 4), instant: true)
        let top = slot.subviews.last
        #expect(top === still, "the picture stays on top")
        stage.releaseStill()
        #expect(stage.still == nil)
        #expect(still.superview == nil, "gone at once")
    }
}
