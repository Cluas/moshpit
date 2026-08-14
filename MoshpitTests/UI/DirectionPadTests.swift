import Foundation
import Testing
@testable import Moshpit

/// The arrow-key joystick's pure logic: dominant-axis direction detection and
/// the fire-on-enter / hold-to-repeat cadence driver.
@Suite("DirectionPad joystick")
struct DirectionPadTests {

    /// The shipping value, not a copy of it. This used to be a hardcoded `11`
    /// behind a "matches DirectionPad's threshold" comment; when the threshold
    /// dropped to 7 the comment went stale and the dead-zone samples below —
    /// picked to sit just inside 11 — kept passing against a number the app no
    /// longer used, testing nothing.
    private let t: CGFloat = DirectionPad.dragThreshold

    @Test("a push shorter than the threshold is the dead-zone (sends nothing)")
    func deadZone() {
        #expect(JoystickRepeater.direction(dx: 0, dy: 0, threshold: t) == nil)
        // Expressed relative to the threshold so these stay just-inside samples
        // whatever it is tuned to.
        #expect(JoystickRepeater.direction(dx: t - 1, dy: -(t - 3), threshold: t) == nil)
        #expect(JoystickRepeater.direction(dx: -(t - 0.1), dy: t - 0.1, threshold: t) == nil)
    }

    @Test("a push just past the threshold leaves the dead-zone")
    func justOutsideDeadZone() {
        // The other half of the contract: the samples above must be inside a
        // boundary that actually exists, or a threshold of ∞ would pass too.
        #expect(JoystickRepeater.direction(dx: t, dy: 0, threshold: t) == "right")
        #expect(JoystickRepeater.direction(dx: 0, dy: -t, threshold: t) == "up")
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
