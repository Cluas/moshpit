import CoreGraphics
import Foundation

/// Finger travel in points → whole rows, carrying the sub-row remainder to the
/// next tick instead of throwing it away. Routes that can only move by rows (a
/// forwarded wheel, tmux copy-mode over mosh, herdr) use it so a slow drag
/// adds up to the same rows a fast one does — the old `Int(dy / cellHeight)`
/// dropped up to a row per tick and made a slow drag barely move.
struct ScrollRowCarry {
    private(set) var remainder: CGFloat = 0

    /// Positive = toward older output. Returns the whole rows now due.
    mutating func take(pixels: CGFloat, cellHeight: CGFloat) -> Int {
        guard cellHeight > 0, pixels.isFinite else { return 0 }
        remainder += pixels
        let rows = Int((remainder / cellHeight).rounded(.towardZero))
        remainder -= CGFloat(rows) * cellHeight
        return rows
    }

    mutating func reset() { remainder = 0 }
}

/// The coast after a drag lifts: UIScrollView's deceleration curve (velocity
/// decays by `decelerationRate` per millisecond) reproduced for a scroll the
/// host drives itself, so a flick over the scrollback keeps moving and eases
/// out the way every other list on the phone does. Pure: the gesture feeds it
/// frame times and moves the content by what it returns.
struct ScrollFling {
    /// `UIScrollView.DecelerationRate.normal`.
    static let decelerationRate: CGFloat = 0.998
    /// Below this the motion is imperceptible: stop and settle.
    static let restVelocity: CGFloat = 20
    /// A lift below this is a drag that stopped, not a flick — no coast.
    static let minimumVelocity: CGFloat = 120
    /// Cap so a wild flick doesn't keep the content moving for many seconds.
    static let maximumVelocity: CGFloat = 5000

    /// Points per second; positive = toward older output.
    private(set) var velocity: CGFloat

    init?(velocity: CGFloat) {
        guard velocity.isFinite, abs(velocity) >= Self.minimumVelocity else { return nil }
        self.velocity = min(Self.maximumVelocity, max(-Self.maximumVelocity, velocity))
    }

    var isAtRest: Bool { abs(velocity) < Self.restVelocity }

    /// Advance `dt` seconds; returns the distance to travel this frame (the
    /// exact integral of the exponential over the frame, so frame-rate
    /// changes don't change where the coast ends).
    mutating func step(dt: TimeInterval) -> CGFloat {
        guard dt > 0, !isAtRest else { return 0 }
        let decay = pow(Self.decelerationRate, CGFloat(dt * 1000))
        let next = velocity * decay
        let distance = (velocity - next) / (-log(Self.decelerationRate) * 1000)
        velocity = next
        return distance
    }
}
