//! The NSH UI of the spike: a status line drawn by NSH over the terminal
//! component (`moshpit/terminal`, Termux's view registered by the host),
//! which gets its pane's stream id as a prop and nothing else.

use nsh_core::prelude::*;

use crate::core;

/// The root `NshView.runRustApp` mounts.
pub fn root() -> nsh_core::element::Root {
    nsh_core::element::Root::of::<App>().appearance(nsh_core::element::Appearance::Dark)
}

#[component]
pub fn App() -> impl IntoElement {
    let pane = core::pane();
    let mut status_rx = pane.status();
    let status = signal(SharedString::from(status_rx.borrow().line()));
    let size = signal(SharedString::from(""));
    // The core's status, on the engine thread: woken by the watch channel,
    // no timer.
    spawn(async move {
        while status_rx.changed().await.is_ok() {
            let line = status_rx.borrow_and_update().line();
            status.set(SharedString::from(line));
        }
    });
    view()
        .flex_1()
        .bg(rgb(0x0b0e14))
        .child(
            view()
                .px(px(10.0))
                .py(px(6.0))
                .bg(rgb(0x151a23))
                .child(
                    text(move || status.get())
                        .text_color(rgb(0x8fd3c1))
                        .text_size(px(11.0))
                        .font_family("monospace"),
                )
                .child(
                    text(move || size.get())
                        .text_color(rgb(0x5c6773))
                        .text_size(px(10.0))
                        .font_family("monospace"),
                ),
        )
        .child(
            native("moshpit/terminal")
                .prop("stream", serde_json::json!(pane.id))
                .on_native_event(move |e: &NativeEvent| {
                    if &*e.name == "size" {
                        let cols = e.data["cols"].as_u64().unwrap_or(0);
                        let rows = e.data["rows"].as_u64().unwrap_or(0);
                        size.set(SharedString::from(format!(
                            "view event: size {cols}×{rows} (NSH native event)"
                        )));
                    }
                })
                .flex_1(),
        )
}
