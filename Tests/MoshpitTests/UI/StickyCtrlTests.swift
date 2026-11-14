import Foundation
import Testing
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
        func type(_ s: String) { coordinator.sendInput(ArraySlice(Array(s.utf8))) }
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
}
