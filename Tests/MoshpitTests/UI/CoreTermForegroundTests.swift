import Foundation
import Testing
import UIKit
@testable import Moshpit

/// B386: after the app had been in the background a while, the terminal
/// came back blank under its chrome. Typing went out and echoed into it
/// unseen, an input-method switch flashed the content for an instant (the
/// keyboard cover: a CoreGraphics picture of the rows, not the Metal layer)
/// and only a redial or a new session brought it back. What a terminal now
/// does about it: drawables let go are made again before anything is drawn
/// into them, nothing is drawn while the app is in the background, every
/// terminal on screen draws again when the app is active again (on a new
/// layer when its own had a reason to fail), a render that can't present
/// says why, and the
/// NSH stage checks on the app's return that the terminal is in its slot
/// with no stuck veil over it. Metal: on a device.
@Suite("core term after the background (B386)", .serialized)
@MainActor
struct CoreTermForegroundTests {

    private static let font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)

    private func makeView(width: CGFloat = 360, height: CGFloat = 480) -> CoreTerminalView {
        CoreTerminalView(frame: CGRect(x: 0, y: 0, width: width, height: height), font: Self.font)
    }

    private func inWindow(_ view: CoreTerminalView) -> UIWindow {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow(frame: CGRect(x: 0, y: 0, width: 600, height: 800))
        window.frame = CGRect(x: 0, y: 0, width: 600, height: 800)
        let host = UIViewController()
        window.rootViewController = host
        host.view.addSubview(view)
        window.isHidden = false
        view.layoutIfNeeded()
        return window
    }

    private func settle(_ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(2)
        while !condition(), Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test("Drawables let go, then a render at the same size: drawn into drawables the view's size")
    func dropThenRenderAtTheSameSize() throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        #expect(view.drawablesFit)
        let fitted = try #require(view.drawablePixelSize)
        // What the background does to a terminal off screen, here without
        // moving it, so nothing but the render can make them again.
        view.releaseDrawablesForTesting()
        #expect(!view.drawablesFit)
        let before = view.renders
        view.renderNowForTesting()
        let after = try #require(view.drawablePixelSize)
        #expect(after == fitted, "a one-pixel drawable shows as the background colour all over")
        #expect(view.drawablesFit)
        #expect(view.renders == before + 1, "the frame was presented")
        #expect(view.failedRenders == 0)
    }

    @Test("The app active again: a layer that never failed draws on itself, no new canvas")
    func foregroundRedrawsAHealthyLayerInPlace() async throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        // Away and back with the terminal on screen the whole time: nothing
        // let go, nothing refused (no await between: no other test draws
        // meanwhile).
        CoreTermMemory.background()
        CoreTermMemory.foreground()
        let before = view.renders
        let canvases = view.canvasesMade
        view.requestRender()
        try await settle { view.renders > before }
        #expect(view.renders > before)
        #expect(view.canvasesMade == canvases, "a healthy layer is not swapped")
        #expect(view.layerSuspect == nil)
    }

    @Test("The app active again: drawables let go in the background get a new canvas")
    func foregroundSwapsALayerLetGoInTheBackground() async throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        let host = try #require(view.superview)
        let canvases = view.canvasesMade
        // Off screen as the app goes away: its drawables go; back on screen
        // before the app is active again.
        view.removeFromSuperview()
        CoreTermMemory.background()
        let suspect = view.layerSuspect
        host.addSubview(view)
        CoreTermMemory.foreground()
        #expect(suspect != nil, "let go in the background")
        #expect(view.canvasesMade == canvases + 1, "on a new layer, as a redial's new terminal had")
        #expect(view.layerSuspect == nil)
        #expect(view.drawablesFit)
        // The old layer goes once the new one has drawn.
        try await settle { view.subviews.filter { $0 is CoreTermMetalCanvas }.count == 1 }
        #expect(view.subviews.filter { $0 is CoreTermMetalCanvas }.count == 1)
    }

    @Test("The app active again: a layer that could not present gets a new canvas")
    func foregroundSwapsALayerThatCannotPresent() throws {
        let view = makeView(width: 0, height: 0)
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        view.renderNowForTesting()
        #expect(view.failedRenders > 0)
        let canvases = view.canvasesMade
        CoreTermMemory.foreground()
        #expect(view.canvasesMade == canvases + 1)
    }

    @Test("The app active again: a terminal off screen is left alone")
    func foregroundLeavesATerminalOffScreen() async throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        let before = view.renders
        let canvases = view.canvasesMade
        CoreTermMemory.foreground()
        try await Task.sleep(for: .milliseconds(50))
        #expect(view.renders == before)
        #expect(view.canvasesMade == canvases, "it draws when it is back in a window")
    }

    @Test("A render that can't present counts the streak, and the next one that can ends it")
    func cannotPresentIsCounted() throws {
        let view = makeView(width: 0, height: 0)
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        // Moving into the window already asks for a frame, which can't present
        // either; count from there.
        let before = view.failedRenders
        view.renderNowForTesting()
        view.renderNowForTesting()
        let after = view.failedRenders
        #expect(after == before + 2, "no size, no picture")
        view.frame = CGRect(x: 0, y: 0, width: 360, height: 480)
        view.layoutIfNeeded()
        view.renderNowForTesting()
        #expect(view.failedRenders == 0)
    }

    // MARK: Veils

    @Test("A pane-switch veil long past its cap comes off; a fresh one stays")
    func staleVeilIsLifted() {
        let view = makeView()
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(view)
        coordinator.veilForSwitch()
        #expect(coordinator.isCoverPresented)
        let fresh = coordinator.liftStaleVeil(olderThan: 5)
        #expect(!fresh)
        #expect(coordinator.isCoverPresented, "a veil inside its cap is the switch's")
        let stale = coordinator.liftStaleVeil(olderThan: 5, now: CACurrentMediaTime() + 60)
        #expect(stale)
        #expect(!coordinator.isCoverPresented)
    }

    @Test("A veil no coordinator holds is taken out of the terminal")
    func orphanVeilIsRemoved() {
        let view = makeView()
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(view)
        let orphan = UIView(frame: view.bounds)
        orphan.accessibilityIdentifier = SwiftTerminalView.Coordinator.veilIdentifier
        view.addSubview(orphan)
        let lifted = coordinator.liftStaleVeil(olderThan: 5)
        #expect(lifted)
        #expect(orphan.superview == nil)
    }

    // MARK: The NSH stage on the app's return

    @Test("On return, a terminal another host took is hosted in the slot again")
    func stageRehostsATakenTerminal() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        #expect(stage.reassert() == .nothing, "no target, nothing to check")
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let terminal = try #require(stage.terminal)
        #expect(stage.reassert() == .fine)
        // Another host takes the terminal.
        let thief = TerminalHostContainer(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
        thief.host(controller.surface(for: "%1"))
        #expect(stage.terminal == nil, "not the stage's any more")
        #expect(stage.reassert() == .rehosted)
        #expect(stage.terminal === terminal, "the very same terminal, not a new one")
        #expect(terminal.superview?.superview === slot)
    }

    @Test("On return, a host out of the slot on screen is moved in")
    func stageAdoptsIntoTheSlot() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (slot, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let container = try #require(stage.container)
        container.removeFromSuperview()
        #expect(stage.reassert() == .adopted)
        #expect(container.superview === slot)
    }

    @Test("On return, a stuck veil over the terminal is lifted")
    func stageLiftsAStuckVeil() throws {
        let controller = TmuxSessionController(sshSession: MockTmuxTransport())
        let stage = TerminalNSHStage()
        let (_, window) = TerminalNSHTests.slot(for: stage)
        defer { window.isHidden = true }
        stage.show(.init(source: .pane(controller, "%1"), inset: 4))
        let coordinator = try #require(controller.coordinator(for: "%1"))
        coordinator.veilForSwitch()
        #expect(stage.reassert() == .fine, "a veil inside its cap stays")
        #expect(stage.reassert(now: CACurrentMediaTime() + 60) == .unveiled)
        #expect(!coordinator.isCoverPresented)
    }
}
