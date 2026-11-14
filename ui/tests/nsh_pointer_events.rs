//! NSH-GAPS #24, pinned down: on iOS a scroll view's content is culled (its
//! views are made as they come near the viewport), and NSH lost the
//! `pointer_events` of a view made after they were worked out when the
//! view had no props of its own: the native view kept `Auto` and took every
//! touch meant for what is under it (B302: the host folder's card border
//! covered every row on the device). Fixed upstream in b37c601b (#1812:
//! `mount_state` sends the props of a new view whose `class_pointer` is not
//! `Auto`). The headless engine leaves culling off, so this turns it on as
//! the iOS surface has it, then makes the list.

use std::sync::atomic::{AtomicUsize, Ordering};

use nsh_core::{
    Points, Size,
    color::Rgba,
    element::Root,
    platform::{Idiom, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

/// The presses each card's button heard.
static PRESSES: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];

/// What a card's overlay lets through, by the card's index.
const OVERLAYS: [(&str, PointerEvents); 2] = [
    ("none", PointerEvents::None),
    ("box-none", PointerEvents::BoxNone),
];

#[derive(Clone, PartialEq, Props)]
struct CoveredProps {
    /// Unused: a component takes props.
    seed: Prop<u64>,
}

/// A button "Show" that makes a scrolling list of two cards; each card is
/// a button with, over it, a border whose only prop is its style's
/// `pointer_events` (no test id, label or role): the B302 card.
#[component]
fn Covered(props: CoveredProps) -> impl IntoElement {
    let _ = props.seed;
    let shown = signal(false);
    view()
        .w_full()
        .h_full()
        .child(
            view()
                .w(px(120.0))
                .h(px(44.0))
                .test_id("show")
                .on_press(move || shown.set(true)),
        )
        .child(when(
            move || shown.get(),
            || {
                scroll().flex_1().test_id("list").child(
                    view().w_full().p(px(16.0)).gap(px(16.0)).children(
                        OVERLAYS.iter().enumerate().map(|(index, (name, events))| {
                            view()
                                .relative()
                                .w_full()
                                .h(px(80.0))
                                .child(
                                    view()
                                        .w_full()
                                        .h_full()
                                        .test_id(&format!("button-{name}"))
                                        .on_press(move || {
                                            PRESSES[index].fetch_add(1, Ordering::SeqCst);
                                        }),
                                )
                                .child(
                                    view()
                                        .absolute()
                                        .inset_0()
                                        .border(px(1.0))
                                        .border_color(Rgba {
                                            r: 1.0,
                                            g: 1.0,
                                            b: 1.0,
                                            a: 0.2,
                                        })
                                        .pointer_events(*events),
                                )
                        }),
                    ),
                )
            },
        ))
}

/// The iOS surface's culling: a view made after its style's
/// `pointer_events` were worked out, with no other props, keeps them, so
/// the button under it takes the press (on NSH 6a4f1de the overlay took it:
/// "is covered").
#[test]
fn a_culled_view_keeps_its_style_pointer_events_once_made() {
    let mut config = EngineConfig::new(Root::new::<Covered>(|| CoveredProps { seed: 0.into() }));
    config.viewport = Size::new(Points(402.0), Points(874.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (26, 0);
    config.environment.idiom = Idiom::Phone;
    config.environment.pixel_ratio = 3.0;
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).expect("the engine starts");
    app.wait_for("id:show").unwrap();
    app.engine().control().set_content_culling(true);
    app.press("id:show").unwrap();
    for (index, (name, _)) in OVERLAYS.iter().enumerate() {
        let button = format!("id:button-{name}");
        app.wait_for(&button).unwrap();
        app.press(&button)
            .unwrap_or_else(|e| panic!("pointer_events {name}: {e}"));
        assert_eq!(
            PRESSES[index].load(Ordering::SeqCst),
            1,
            "pointer_events {name}"
        );
    }
}
