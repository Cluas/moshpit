//! The spike's NSH root: a status line over the terminal. The terminal is
//! the host's native component `moshpit/terminal`; all it gets from here is
//! its stream's id. Status text comes back as a component event (the
//! control path); bytes never pass through NSH.

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::core::core;

#[component]
pub fn SpikeRoot() -> impl IntoElement {
    let stream = core().terminal();
    let status = signal(format!("stream {stream} · echo on the core's runtime"));
    view()
        .flex_1()
        .w_full()
        .flex_col()
        .pt_safe_area()
        .bg(move || theme().background)
        .child(
            view().px_4().py_1().child(
                text(move || status.get())
                    .text_size(px(12.0))
                    .truncate()
                    .text_color(move || theme().muted_foreground),
            ),
        )
        .child(
            native("moshpit/terminal")
                .flex_1()
                .w_full()
                .overflow_hidden()
                .prop("stream", serde_json::json!(stream))
                .on_native_event(move |event: &NativeEvent| {
                    if &*event.name == "status"
                        && let Some(text) = event.data["text"].as_str()
                    {
                        status.set(text.to_owned());
                    }
                }),
        )
}
