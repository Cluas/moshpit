import Foundation
import Testing
@testable import Ringdown

/// The arrow-key joystick's pure logic: dominant-axis direction detection and
/// the fire-on-enter / hold-to-repeat cadence driver.
@Suite("DirectionPad joystick")
struct DirectionPadTests {

    private let t: CGFloat = 11   // matches DirectionPad's threshold

    @Test("a push shorter than the threshold is the dead-zone (sends nothing)")
    func deadZone() {
        #expect(JoystickRepeater.direction(dx: 0, dy: 0, threshold: t) == nil)
        #expect(JoystickRepeater.direction(dx: 10, dy: -8, threshold: t) == nil)
        #expect(JoystickRepeater.direction(dx: -10.9, dy: 10.9, threshold: t) == nil)
    }

    @Test("dominant axis wins")
    func dominantAxis() {
        #expect(JoystickRepeater.direction(dx: 40, dy: 5, threshold: t) == "right")
        #expect(JoystickRepeater.direction(dx: -40, dy: 5, threshold: t) == "left")
        #expect(JoystickRepeater.direction(dx: 5, dy: 40, threshold: t) == "down")
        #expect(JoystickRepeater.direction(dx: 5, dy: -40, threshold: t) == "up")
    }

    @Test("diagonal resolves to the larger component; a tie favors vertical")
    func diagonal() {
        #expect(JoystickRepeater.direction(dx: 30, dy: 20, threshold: t) == "right")
        #expect(JoystickRepeater.direction(dx: 20, dy: -30, threshold: t) == "up")
        // |dx| == |dy| is not strictly greater, so the vertical branch wins.
        #expect(JoystickRepeater.direction(dx: 25, dy: 25, threshold: t) == "down")
        #expect(JoystickRepeater.direction(dx: -25, dy: -25, threshold: t) == "up")
    }

    @MainActor
    @Test("fires once when a direction is first acquired, dedupes a repeat set")
    func fireOnEnter() {
        let repeater = JoystickRepeater()
        var sent: [String] = []
        repeater.onFire = { sent.append($0) }

        repeater.set("up")          // new direction → immediate fire
        repeater.set("up")          // same direction → no extra immediate fire
        #expect(sent == ["up"])
        #expect(repeater.direction == "up")

        repeater.set("left")        // changed direction → fire again
        #expect(sent == ["up", "left"])

        repeater.stop()
        #expect(repeater.direction == nil)
    }

    @MainActor
    @Test("releasing to center stops and clears the held direction")
    func releaseToCenter() {
        let repeater = JoystickRepeater()
        var sent: [String] = []
        repeater.onFire = { sent.append($0) }

        repeater.set("down")
        repeater.set(nil)           // back to dead-zone
        #expect(repeater.direction == nil)
        #expect(sent == ["down"])
    }
}
