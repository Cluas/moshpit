//! A screen's body behind an error boundary (B366).
//!
//! Every redesigned screen draws its bar beside its body, and the body only
//! once the host's first state has come (`when(ready, body)`). A panic in
//! the body's code (a binding, an effect, a component's view) is caught by
//! NSH, and without a boundary the part that failed shows nothing: the
//! screen is its bar and nothing under it, the panic said only on stderr,
//! which a phone keeps nowhere. With [`ready_body`] (or [`guarded`]) the
//! failed body is replaced by a small "Couldn't show this page" with Retry,
//! and the host hears both:
//!
//! - `{"ev": "failed", "component": …, "message": …}` when the body fails,
//!   which the host logs (the app's diagnostics log);
//! - `{"ev": "retry"}` when Retry is pressed: the host runs the screen
//!   again, on the state it keeps (`NSHPage`'s third rule).

use std::rc::Rc;

use nsh_core::prelude::*;
use serde_json::json;

use crate::ui::{
    button::{Variant, button},
    empty::empty,
};

/// What tells the host (`{"ev": …}`), as the screens hold it.
pub type Tell = Rc<dyn Fn(serde_json::Value)>;

/// `body` once `ready` holds, behind the boundary ([`guarded`]): what a
/// screen's `when(ready, body)` becomes.
pub fn ready_body<E: IntoElements + 'static>(
    ready: impl Fn() -> bool + 'static,
    body: impl Fn() -> E + 'static,
    tell: Tell,
) -> AnyElement {
    let body = Rc::new(body);
    when(ready, move || {
        let body = body.clone();
        guarded(move || body(), tell.clone())
    })
    .into_element()
}

/// `body`, or once something in it failed, the failure: the host told
/// (`failed`) and a Retry that asks it to run the screen again (`retry`).
pub fn guarded<E: IntoElements + 'static>(
    body: impl Fn() -> E + 'static,
    tell: Tell,
) -> AnyElement {
    catch_error(body, move |error, _reset| failed(&error, tell.clone()))
}

/// The host's `failed` event for `error`.
pub fn failed_event(error: &ComponentError) -> serde_json::Value {
    json!({
        "ev": "failed",
        "component": error.component,
        "message": error.message,
    })
}

/// The host's `retry` event.
pub fn retry_event() -> serde_json::Value {
    json!({"ev": "retry"})
}

/// What shows in place of a body that failed, the host told why.
fn failed(error: &ComponentError, tell: Tell) -> AnyElement {
    // On stderr too (a headless run, a debugger); the host's log has it from
    // the event, so not twice through the log handler.
    log::info!(target: "moshpit", "a screen's body failed: {error}");
    tell(failed_event(error));
    view()
        .w_full()
        .flex_1()
        .justify_center()
        .items_center()
        .test_id("page-failed")
        .child(
            empty(t!("Couldn't show this page"))
                .icon("exclamationmark.triangle")
                .action(
                    button(t!("Retry"))
                        .variant(Variant::Secondary)
                        .test_id("page-failed-retry")
                        .on_press(move || tell(retry_event())),
                ),
        )
        .into_element()
}
