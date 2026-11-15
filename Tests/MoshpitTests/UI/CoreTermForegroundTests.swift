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

    @Test("B469: let go in the background off screen, on screen after the return: a new canvas at once")
    func intoAWindowAfterTheReturnSwapsASuspectLayer() throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        let host = try #require(view.superview)
        let canvases = view.canvasesMade
        // Off screen as the app goes away, and still off screen when it is
        // active again (the screen re-ran after the return): the return's
        // new canvas passes it by.
        view.removeFromSuperview()
        CoreTermMemory.background()
        #expect(view.layerSuspect != nil, "let go in the background")
        CoreTermMemory.foreground()
        #expect(view.canvasesMade == canvases, "off screen: nothing then")
        host.addSubview(view)
        #expect(view.canvasesMade == canvases + 1, "a new layer as it comes on screen, not eight dropped frames later")
        #expect(view.layerSuspect == nil)
        #expect(view.drawablesFit)
    }

    @Test("B469: a layer with nothing against it comes on screen on its own canvas")
    func intoAWindowKeepsAHealthyLayer() throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        let window = inWindow(view)
        defer { window.isHidden = true }
        let host = try #require(view.superview)
        let canvases = view.canvasesMade
        view.removeFromSuperview()
        host.addSubview(view)
        #expect(view.layerSuspect == nil)
        #expect(view.canvasesMade == canvases)
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
        let coordinator = CoreTerminalHost.Coordinator()
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
        let coordinator = CoreTerminalHost.Coordinator()
        coordinator.attach(view)
        let orphan = UIView(frame: view.bounds)
        orphan.accessibilityIdentifier = CoreTerminalHost.Coordinator.veilIdentifier
        view.addSubview(orphan)
        let lifted = coordinator.liftStaleVeil(olderThan: 5)
        #expect(lifted)
        #expect(orphan.superview == nil)
    }

    // MARK: B445: a first open's frame, drawn whole

    @Test("B445: into a window, every row is encoded and drawn again, whatever was drawn before")
    func intoAWindowDrawsWhole() throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        view.feed(Data("first open\r\nidle pane".utf8))
        #expect(view.settle { $0.rowText(0) == "first open" })
        let before = view.fullRedraws
        let window = inWindow(view)
        defer { window.isHidden = true }
        #expect(view.fullRedraws > before, "the one full frame came before the window: drawn whole on arrival")
    }

    @Test("B445: a new size draws every row again at once, with no new bytes")
    func aNewSizeDrawsWhole() throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        view.feed(Data("idle pane".utf8))
        let window = inWindow(view)
        defer { window.isHidden = true }
        #expect(view.settle { $0.rowText(0) == "idle pane" })
        let before = view.fullRedraws
        view.frame = CGRect(x: 0, y: 0, width: 360, height: 400)
        view.layoutIfNeeded()
        #expect(view.fullRedraws == before + 1, "the keyboard, a rotation, the slot's first layout")
        view.setNeedsLayout()
        view.layoutIfNeeded()
        #expect(view.fullRedraws == before + 1, "the same size again: a render, nothing encoded anew")
    }

    @Test("B445: frames drawn in a window on screen are followed to the screen")
    func framesAreFollowedToTheScreen() async throws {
        let view = makeView()
        guard view.canvasKind == "metal" else { return }
        view.feed(Data("shown".utf8))
        let window = inWindow(view)
        defer { window.isHidden = true }
        try await settle { view.framesShown > 0 }
        #expect(view.framesShown > 0, "a drawable on screen reports its presentation")
        #expect((view.framesSinceCanvas ?? 0) > 0, "what the stage's check reads")
    }

    // MARK: The NSH stage on the app's return

    @Test("On return, a terminal another host took is hosted in the slot again")
    func stageRehostsATakenTerminal() throws {
        let controller = TerminalNSHTests.paneController()
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
        let controller = TerminalNSHTests.paneController()
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
        let controller = TerminalNSHTests.paneController()
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

/// B445: a frame the core's terminal drew can be dropped by the layer or
/// refused by the GPU, and an idle pane sends nothing that would draw
/// again; every frame is followed to the screen, and one lost with nothing
/// newer on its way is drawn again, a few times at most (headless).
@Suite("a frame followed to the screen (B445)")
struct CoreTermPresentationTests {
    @Test("A frame lost with nothing newer on its way is drawn again")
    func aLostFrameIsDrawnAgain() {
        let p = CoreTermPresentation()
        let frame = p.issue()
        #expect(p.unresolved(frame))
        #expect(p.neverReached(frame, active: true) == .drawAgain)
        #expect(!p.unresolved(frame))
        #expect(p.framesNeverShown == 1)
    }

    @Test("Nothing is lost when a newer frame got there or is on its way, or the app is not active")
    func nothingLost() {
        let p = CoreTermPresentation()
        let old = p.issue()
        let newer = p.issue()
        #expect(p.neverReached(old, active: true) == .nothingLost, "its successor decides")
        p.shown(newer)
        #expect(p.neverReached(old, active: true) == .nothingLost, "a newer one is on screen")
        let last = p.issue()
        #expect(p.neverReached(last, active: false) == .nothingLost, "the app's return draws")
        #expect(p.currentStreak == 0)
    }

    @Test("Drawn again up to the cap, then a new canvas once, then nothing until a frame is shown")
    func theCap() {
        let p = CoreTermPresentation()
        var verdicts: [CoreTermPresentation.Verdict] = []
        for _ in 0..<(CoreTermPresentation.retryCap + 3) {
            verdicts.append(p.neverReached(p.issue(), active: true))
        }
        #expect(verdicts.prefix(CoreTermPresentation.retryCap).allSatisfy { $0 == .drawAgain })
        #expect(verdicts[CoreTermPresentation.retryCap] == .giveUp)
        #expect(verdicts.dropFirst(CoreTermPresentation.retryCap + 1).allSatisfy { $0 == .nothingLost })
        p.shown(p.issue())
        #expect(p.neverReached(p.issue(), active: true) == .drawAgain, "a frame on screen starts it over")
        #expect(p.shownFrames == 1)
    }

    @Test("A frame's fate is said once, whichever of its two reports comes first")
    func aFateIsSaidOnce() {
        final class Heard: @unchecked Sendable { var said: [Bool] = [] }
        let heard = Heard()
        let fate = CoreTermFrameFate { heard.said.append($0) }
        fate.resolve(shown: false)
        fate.resolve(shown: true)
        #expect(heard.said == [false])
    }
}
