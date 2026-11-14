//! The companion (moshpit-design `docs/nsh/PET-DESIGN.md` §7, §10): the
//! person's pet, or its egg, drawn from its sprites (`moshpit-pet`) and
//! playing its frames.
//!
//! A sprite is drawn as a PNG scaled by whole pixels a cell for a 3x
//! screen, so the pixel art stays crisp (an image the platform scales up
//! is smoothed). Frames are made once per figure, size and appearance and
//! kept. With reduced motion the first frame stays.

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use moshpit_pet::{Appearance, EggPose, Look, Pose, Sprite, egg, egg_durations};
use nsh_core::{element::ImageSrc, prelude::*};

/// What a companion shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Figure {
    /// The egg; cracked when it's ready to hatch. `wobble`: its wobble
    /// alone, at once (a tap), not its loop's still first.
    Egg {
        ready: bool,
        wobble: bool,
    },
    Pet {
        look: Look,
        pose: Pose,
    },
}

impl Figure {
    fn frames(self) -> Vec<(Sprite, u32)> {
        match self {
            Figure::Egg { ready, wobble } => {
                let pose = egg_pose(ready);
                egg_durations(pose)
                    .into_iter()
                    .enumerate()
                    .skip(usize::from(wobble))
                    .filter_map(|(i, ms)| egg(pose, i).map(|s| (s, ms)))
                    .collect()
            }
            Figure::Pet { look, pose } => look
                .durations(pose)
                .into_iter()
                .enumerate()
                .filter_map(|(i, ms)| look.compose(pose, i).map(|s| (s, ms)))
                .collect(),
        }
    }

    /// How long one pass of its frames takes.
    pub fn length(self) -> Duration {
        let ms: u32 = match self {
            Figure::Egg { ready, wobble } => egg_durations(egg_pose(ready))
                .iter()
                .skip(usize::from(wobble))
                .sum(),
            Figure::Pet { look, pose } => look.durations(pose).iter().sum(),
        };
        Duration::from_millis(u64::from(ms))
    }
}

fn egg_pose(ready: bool) -> EggPose {
    if ready {
        EggPose::Ready
    } else {
        EggPose::Still
    }
}

/// A figure's frames as images, and how long each shows.
struct Frames {
    uris: Vec<SharedString>,
    ms: Vec<u32>,
}

impl Frames {
    /// The frame `elapsed` milliseconds into a loop of them.
    fn at(&self, elapsed: u64) -> SharedString {
        let total: u64 = self.ms.iter().map(|m| u64::from(*m)).sum::<u64>().max(1);
        let mut t = elapsed % total;
        for (uri, ms) in self.uris.iter().zip(&self.ms) {
            if t < u64::from(*ms) {
                return uri.clone();
            }
            t -= u64::from(*ms);
        }
        self.uris.first().cloned().unwrap_or_default()
    }
}

/// Whole pixels a cell for a 3x screen at `size` points.
fn scale(size: f32) -> u32 {
    ((size * 3.0) / 24.0).round().max(1.0) as u32
}

thread_local! {
    static FRAMES: RefCell<HashMap<(Figure, bool, u32), Rc<Frames>>> = RefCell::new(HashMap::new());
}

fn frames(figure: Figure, dark: bool, size: f32) -> Rc<Frames> {
    let key = (figure, dark, scale(size));
    if let Some(f) = FRAMES.with(|c| c.borrow().get(&key).cloned()) {
        return f;
    }
    let appearance = if dark {
        Appearance::Dark
    } else {
        Appearance::Light
    };
    let (uris, ms) = figure
        .frames()
        .into_iter()
        .map(|(s, ms)| (SharedString::from(s.data_uri(appearance, key.2)), ms))
        .unzip();
    let made = Rc::new(Frames { uris, ms });
    FRAMES.with(|c| {
        let mut c = c.borrow_mut();
        // A handful of figures at a time; never grows past a few dozen.
        if c.len() > 48 {
            c.clear();
        }
        c.insert(key, made.clone());
    });
    made
}

/// How often the frames are looked at: a frame lasts 110 ms at the least.
const TICK: Duration = Duration::from_millis(40);

/// A companion `size` points square, showing `figure` (it can change: a
/// new pose, an egg that cracked, a moment), its frames looping from
/// `since` (a figure's own clock, ms; a moment starts its figure at 0).
/// Hidden from screen readers: whatever holds it says what it is.
pub fn companion(figure: impl Fn() -> Figure + 'static, size: f32) -> AnyElement {
    let figure = computed(figure);
    let elapsed = signal(0u64);
    // The loop's clock starts over when the figure changes.
    effect(move || {
        let _ = figure.get();
        elapsed.set(0);
    });
    let _ticker = (!reduce_motion()).then(|| {
        set_interval(TICK, move || {
            elapsed.update(|t| *t += TICK.as_millis() as u64)
        })
    });
    let src = move || {
        let frames = frames(figure.get(), dark(), size);
        let t = if reduce_motion() { 0 } else { elapsed.get() };
        ImageSrc::from(frames.at(t))
    };
    image(src)
        .w(px(size))
        .h(px(size))
        .accessibility_elements_hidden(true)
        .into_element()
}
