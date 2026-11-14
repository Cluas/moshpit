//! The motion tokens read the surface's reduced motion: a screen built on
//! them jumps when the user asks for less motion (UI-DESIGN §5.2).

use moshpit_ui::ui::motion::{Duration, structural_spring};
use nsh_core::element::Root;
use nsh_core::prelude::*;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[derive(Clone, PartialEq, Props)]
pub struct ProbeProps {
    /// Unused: a component's props need a field.
    pub surface: Prop<u64>,
}

/// Says what every duration token and the spring are on its surface.
#[component]
fn Probe(_props: ProbeProps) -> impl IntoElement {
    let words = Duration::ALL
        .iter()
        .map(|d| format!("{}={}", d.name(), d.ms()))
        .chain([format!("spring={}", structural_spring(None).is_some())])
        .collect::<Vec<_>>()
        .join(" ");
    view().test_id("probe").child(text(words))
}

fn start(reduce_motion: bool) -> Headless {
    let mut config = EngineConfig::new(Root::new::<Probe>(|| ProbeProps {
        surface: 0u64.into(),
    }));
    config.environment.reduce_motion = reduce_motion;
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:probe").unwrap();
    app
}

#[test]
fn reduced_motion_zeroes_every_duration() {
    let mut app = start(true);
    app.wait_for("instant=0 fast=0 base=0 slow=0 slower=0 spring=false")
        .unwrap();
}

#[test]
fn full_motion_keeps_the_design() {
    let mut app = start(false);
    app.wait_for("instant=120 fast=180 base=240 slow=360 slower=500 spring=true")
        .unwrap();
}
