//! The motion tokens (`docs/nsh/UI-DESIGN.md` §5): how long, on what curve,
//! how far and how much things move. The `ui` layer's components and the
//! screens on them take every duration, easing, spring, distance and scale
//! from here, never a number of their own.
//!
//! Reduced motion is part of each token: a duration read through
//! [`Duration::ms`] is zero while the user asks for less motion, so what
//! uses it jumps; [`structural_spring`] gives no spring then. What follows
//! a finger (a drag, a scroll-driven fade) is not motion and is not
//! touched.

use nsh_core::prelude::*;

/// A duration token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Duration {
    /// 120 ms: a hover wash, a color changing.
    Instant,
    /// 180 ms: a press, a control's state, leaving.
    Fast,
    /// 240 ms: moving into place: a selection sliding, a glyph swapping, a
    /// disclosure turning, a menu leaving.
    Base,
    /// 360 ms: appearing: a panel, a toast, a row that newly needs you.
    Slow,
    /// 500 ms: an unhurried entrance (a list's first wave).
    Slower,
}

impl Duration {
    pub const ALL: [Duration; 5] = [
        Duration::Instant,
        Duration::Fast,
        Duration::Base,
        Duration::Slow,
        Duration::Slower,
    ];

    /// Its name in the design (`base`).
    pub fn name(self) -> &'static str {
        match self {
            Duration::Instant => "instant",
            Duration::Fast => "fast",
            Duration::Base => "base",
            Duration::Slow => "slow",
            Duration::Slower => "slower",
        }
    }

    /// Its length in milliseconds with full motion.
    pub const fn nominal_ms(self) -> f32 {
        match self {
            Duration::Instant => 120.0,
            Duration::Fast => 180.0,
            Duration::Base => 240.0,
            Duration::Slow => 360.0,
            Duration::Slower => 500.0,
        }
    }

    /// Its length when motion is `reduced` or not: zero when it is.
    pub const fn ms_when(self, reduced: bool) -> f32 {
        if reduced { 0.0 } else { self.nominal_ms() }
    }

    /// Its length on this surface now: zero while the user asks for less
    /// motion.
    pub fn ms(self) -> f32 {
        self.ms_when(reduce_motion())
    }

    /// A timing of this length on `easing` ([`Duration::ms`]).
    pub fn timing(self, easing: Easing) -> Timing {
        Timing::new(self.ms(), easing)
    }
}

/// Things appearing: fast at first, settling into place.
pub const ENTER: Easing = Easing::EaseOut;
/// Something moving between two places.
pub const MOVE: Easing = Easing::EaseInOut;
/// Things leaving: slow at first, then gone.
pub const LEAVE: Easing = Easing::EaseIn;
/// A press settling, as a cubic Bézier's points: quick and soft, no
/// overshoot (a transition takes them as `ease_bezier`).
pub const SETTLE_CURVE: [f32; 4] = [0.22, 1.0, 0.36, 1.0];
/// [`SETTLE_CURVE`] as an easing (an animation's timing).
pub const SETTLE: Easing = Easing::Bezier(
    SETTLE_CURVE[0],
    SETTLE_CURVE[1],
    SETTLE_CURVE[2],
    SETTLE_CURVE[3],
);

/// How quickly the structural spring settles, in milliseconds (NSH's
/// response): what the user throws or drags lands on it.
pub const SPRING_RESPONSE_MS: f32 = 350.0;

/// The spring for what the user throws or drags and lets go (a swipe
/// tray, a panel), critically damped (no overshoot), continuing at
/// `velocity` (per second) when the release has one; `None` while the user
/// asks for less motion, so what uses it lands at once.
pub fn structural_spring(velocity: Option<f32>) -> Option<Spring> {
    structural_spring_when(velocity, reduce_motion())
}

/// [`structural_spring`] when motion is `reduced` or not.
pub fn structural_spring_when(velocity: Option<f32>, reduced: bool) -> Option<Spring> {
    if reduced {
        return None;
    }
    let mut spring = Spring::new(SPRING_RESPONSE_MS).damping_ratio(1.0);
    spring.velocity = velocity;
    Some(spring)
}

/// How far things travel, in points.
pub mod distance {
    /// A label rising as it changes.
    pub const RISE: f32 = 4.0;
    /// A popup or a banner sliding in from its edge.
    pub const SLIDE: f32 = 8.0;
    /// A panel or a toast entering.
    pub const ENTER: f32 = 16.0;
}

/// How much things scale.
pub mod scale {
    /// A button pressed.
    pub const PRESS: f32 = 0.97;
    /// An icon button or a key pressed.
    pub const PRESS_ICON: f32 = 0.95;
    /// A panel or a dialog as it enters.
    pub const ENTER: f32 = 0.96;
}

/// A working agent's dot breathing (one period, ms); it rests while motion
/// is reduced, the app is in the background or the dot is off screen.
pub const BREATHE_MS: f32 = 1600.0;
/// A skeleton's pulse (one period, ms); it rests while motion is reduced.
pub const SKELETON_MS: f32 = 1200.0;

/// The shortcut bar riding the keyboard: the keyboard's own curve (the host
/// lines it up with the keyboard's start, NSH-GAPS #4).
pub const KEYBOARD: nsh_core::LayoutCurve = nsh_core::LayoutCurve::Keyboard;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_the_design() {
        let ms: Vec<f32> = Duration::ALL.iter().map(|d| d.nominal_ms()).collect();
        assert_eq!(ms, [120.0, 180.0, 240.0, 360.0, 500.0]);
        // Leaving (fast) is quicker than entering (slow).
        assert!(Duration::Fast.nominal_ms() < Duration::Slow.nominal_ms());
    }

    #[test]
    fn reduced_motion_makes_every_duration_zero_and_drops_the_spring() {
        for d in Duration::ALL {
            assert_eq!(d.ms_when(true), 0.0, "{}", d.name());
            assert_eq!(d.ms_when(false), d.nominal_ms(), "{}", d.name());
        }
        assert!(structural_spring_when(Some(800.0), true).is_none());
    }

    #[test]
    fn the_structural_spring_does_not_overshoot_and_keeps_the_throw() {
        let spring = structural_spring_when(Some(800.0), false).unwrap();
        let critical = 2.0 * (spring.stiffness * spring.mass).sqrt();
        assert!(
            (spring.damping / critical - 1.0).abs() < 1e-3,
            "damping ratio 1"
        );
        assert_eq!(spring.velocity, Some(800.0));
    }

    #[test]
    fn presses_shrink_a_little_and_icons_a_little_more() {
        const { assert!(scale::PRESS_ICON < scale::PRESS && scale::PRESS < 1.0) };
        const { assert!(distance::RISE < distance::SLIDE && distance::SLIDE < distance::ENTER) };
    }
}
