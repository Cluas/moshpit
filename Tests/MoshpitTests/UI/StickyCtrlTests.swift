import Foundation
import MoshpitFFI
import Testing
import UIKit
@testable import Moshpit

/// The shortcut bar's sticky Ctrl (B411): armed on the coordinator typing
/// goes to, folded into the next key there once, and let go when typing
/// moves to another pane or session — never left armed on the pane left,
/// where a later letter would go out as a control code.
@Suite("sticky Ctrl")
@MainActor
struct StickyCtrlTests {

    /// A coordinator and what it sends.
    private final class Pane {
        let coordinator = SwiftTerminalView.Coordinator()
        var sent = Data()
        init() { coordinator.onInput = { [unowned self] in self.sent.append($0) } }
        /// What the person types (a key, a character, a commit).
        func type(_ s: String) { coordinator.sendTyped(ArraySlice(Array(s.utf8))) }
        /// What the terminal sends of its own (a reply, a mouse report).
        func reply(_ s: String) { coordinator.sendInput(ArraySlice(Array(s.utf8))) }
        func take() -> Data { defer { sent = Data() }; return sent }
    }

    @Test("the next key folds into its control code, once, and the chip goes out")
    func foldsOnce() {
        let a = Pane()
        var chip = false
        let ctrl = StickyCtrl()
        ctrl.onChange = { chip = $0 }
        ctrl.arm(a.coordinator)
        #expect(chip)
        a.type("c")
        #expect(a.take() == Data([0x03]))
        #expect(!chip)
        a.type("c")
        #expect(a.take() == Data("c".utf8), "one-shot")
    }

    @Test("typing moving to another pane lets the armed Ctrl go")
    func movingAwayDisarms() {
        let a = Pane(), b = Pane()
        var chip = false
        let ctrl = StickyCtrl()
        ctrl.onChange = { chip = $0 }
        ctrl.arm(a.coordinator)
        ctrl.typingMoved(to: ObjectIdentifier(b.coordinator))
        #expect(!chip, "the chip goes out with it")
        b.type("d")
        #expect(b.take() == Data("d".utf8))
        // Back on the pane left: its first letter is a letter, not ^C.
        ctrl.typingMoved(to: ObjectIdentifier(a.coordinator))
        a.type("c")
        #expect(a.take() == Data("c".utf8), "the pane left typed ^C")
    }

    @Test("off is off on the pane it was armed on, wherever typing is")
    func offWhereItWasArmed() {
        let a = Pane()
        let ctrl = StickyCtrl()
        ctrl.arm(a.coordinator)
        ctrl.disarm()
        a.type("d")
        #expect(a.take() == Data("d".utf8), "the pane armed typed ^D")
    }

    @Test("staying on the pane keeps it armed; arming another lets the first go")
    func oneAtATime() {
        let a = Pane(), b = Pane()
        let ctrl = StickyCtrl()
        ctrl.arm(a.coordinator)
        ctrl.typingMoved(to: ObjectIdentifier(a.coordinator))
        #expect(ctrl.isArmed)
        ctrl.arm(b.coordinator)
        a.type("c")
        #expect(a.take() == Data("c".utf8))
        b.type("c")
        #expect(b.take() == Data([0x03]))
    }

    @Test("B570: whatever is typed next takes it, folded or not — Return, an arrow, a commit never leave it armed")
    func anythingTypedTakesIt() {
        let a = Pane()
        var chip = false
        let ctrl = StickyCtrl()
        ctrl.onChange = { chip = $0 }
        for unfoldable in ["\r", "\u{1b}[A", "你", "\t"] {
            ctrl.arm(a.coordinator)
            a.type(unfoldable)
            #expect(a.take() == Data(unfoldable.utf8), "\(unfoldable.debugDescription) went out as typed")
            #expect(!chip && !ctrl.isArmed, "\(unfoldable.debugDescription) left the Ctrl armed")
            a.type("c")
            #expect(a.take() == Data("c".utf8), "the letter after \(unfoldable.debugDescription) went out as ^C")
        }
    }

    @Test("B570: Ctrl-Space is NUL, Ctrl-/ is ^_, Ctrl-? is DEL")
    func foldsTheOtherControlKeys() {
        #expect(SwiftTerminalView.Coordinator.ctrlFolded([0x20]) == [0x00])
        #expect(SwiftTerminalView.Coordinator.ctrlFolded([0x2f]) == [0x1f])
        #expect(SwiftTerminalView.Coordinator.ctrlFolded([0x3f]) == [0x7f])
        #expect(SwiftTerminalView.Coordinator.ctrlFolded(Array("a".utf8)) == [0x01])
        #expect(SwiftTerminalView.Coordinator.ctrlFolded(Array("[".utf8)) == [0x1b])
        #expect(SwiftTerminalView.Coordinator.ctrlFolded(Array("1".utf8)) == Array("1".utf8))
        #expect(SwiftTerminalView.Coordinator.ctrlFolded(Array("ab".utf8)) == Array("ab".utf8))
        let a = Pane()
        let ctrl = StickyCtrl()
        ctrl.arm(a.coordinator)
        a.type(" ")
        #expect(a.take() == Data([0x00]))
    }

    @Test("B570: the terminal's own replies neither fold nor take it")
    func repliesLeaveItArmed() {
        let a = Pane()
        let ctrl = StickyCtrl()
        ctrl.arm(a.coordinator)
        a.reply("A")
        #expect(a.take() == Data("A".utf8), "a reply went out as ^A")
        #expect(ctrl.isArmed)
        a.type("c")
        #expect(a.take() == Data([0x03]))
    }

    @Test("B570: on the core's view a key is typing, a reply is not")
    func coreViewSeparatesTypingFromReplies() throws {
        let surface = TerminalSurfaces.make(font: UIFont.monospacedSystemFont(ofSize: 12, weight: .regular),
                                            grid: TerminalGrid(cols: 20, rows: 3))
        let core = try #require(surface as? CoreTerminalView)
        let coordinator = SwiftTerminalView.Coordinator()
        coordinator.attach(surface)
        var sent = Data()
        coordinator.onInput = { sent.append($0) }
        let ctrl = StickyCtrl()
        ctrl.arm(coordinator)
        core.sendReply(Data("A".utf8))
        #expect(sent == Data("A".utf8))
        #expect(ctrl.isArmed, "the terminal's reply took the Ctrl")
        sent = Data()
        core.sendKey(UInt32(MP_KEY_ENTER), mods: 0)
        #expect(sent == Data([0x0d]))
        #expect(!ctrl.isArmed, "Return left the Ctrl armed")
        sent = Data()
        core.insertText("c")
        #expect(sent == Data("c".utf8), "the letter after Return went out as ^C")
    }
}
