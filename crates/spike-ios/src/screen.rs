//! Spike G5 (`docs/nsh/DESIGN.md`, the gate table): Moshpit's terminal
//! screen (`Moshpit/UI/Terminal/TerminalScreen.swift`) with its chrome
//! drawn by NSH over the host's terminal component `moshpit/terminal`.
//!
//! The composition follows the SwiftUI screen: a 44 pt top bar under the
//! status bar (back button, transport pill, the breadcrumb capsule on
//! Liquid Glass), the terminal, and the shortcut bar riding the keyboard.
//! Over the terminal, as its siblings (never its ancestors: an ancestor's
//! press fires together with the terminal's own tap), the banner, the
//! connecting cover and the reconnect veil.
//!
//! The keyboard is the terminal's own business. The column's bottom margin
//! is the keyboard's height ([`Styled::mb_keyboard`]) and the bar keeps
//! the home indicator's inset only while the keyboard is down, both moving
//! on the keyboard's curve in the commit that hears it. The terminal takes
//! and drops focus by its component's `focus` and `blur` commands; its
//! frame lock and snapshot cover live in the host, next to the view.
//!
//! [`G5Root`] puts the screen in an NSH navigation stack over a home page,
//! so a test can leave it and come back; [`G5Embedded`] is the screen alone,
//! for a SwiftUI navigation stack to host. The host drives both with
//! messages ([`crate::app::mp_spike_post`]) and hears them back
//! ([`crate::app::tell_host`]).

use nsh_base::{nav_page, nav_stack};
use nsh_core::prelude::*;
use serde_json::json;

use crate::{
    app::{host_messages, tell_host},
    core::core,
};

/// Signal Room's colors (`Moshpit/UI/DesignTokens.swift`, `AppTheme.swift`), the
/// spike's own: the app's are its tokens.
mod hue {
    use nsh_core::prelude::{Rgba, rgb, rgba};

    pub const TERMINAL_BG: Rgba = rgb(0x030405);
    pub const ACCENT: Rgba = rgb(0x6C6BEF);
    pub const ACCENT_WASH: Rgba = rgba(0x6C6BEF14);
    pub const WARN: Rgba = rgb(0xFFB35C);
    pub const WARN_WASH: Rgba = rgba(0xFFB35C24);
    pub const SIGNAL: Rgba = rgb(0x7AA7FF);
    pub const SIGNAL_SOFT: Rgba = rgb(0xA9C4FF);
    pub const SIGNAL_WASH: Rgba = rgba(0x7AA7FF24);
    pub const SIGNAL_BORDER: Rgba = rgba(0x7AA7FF57);
    pub const SUCCESS: Rgba = rgb(0x83E58C);
    /// The top bar: `.bar` over the terminal's background.
    pub const BAR: Rgba = rgba(0x0B0D12D1);
    pub const SHORTCUT_BAR: Rgba = rgba(0x0B0D11FA);
    pub const KEY: Rgba = rgba(0x1B2028FA);
    pub const KEY_BORDER: Rgba = rgba(0xFFFFFF14);
    pub const HAIRLINE: Rgba = rgba(0xFFFFFF13);
    pub const PRIMARY: Rgba = rgba(0xF4F1E8F5);
    pub const SECONDARY: Rgba = rgba(0xE8E3D6B8);
    pub const META: Rgba = rgba(0xC7C1B570);
    pub const VEIL: Rgba = rgba(0x00000073);
}

/// The terminal's font family; chrome text is monospaced like the app's
/// `Face.mono`.
const MONO: &str = "Menlo";

/// What a page of the stack shows.
#[derive(Debug, Clone, PartialEq)]
enum Route {
    Home,
    /// The terminal screen of pane `n` (1: the shell, 2: the TUI).
    Terminal(u32),
}

/// Where the connection is, as the cover and the veil see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// First connect: the poster covers the pane until it painted.
    Connecting,
    Live,
    /// A drop being redialled: the veil dims the last frame.
    Reconnecting,
}

/// One pane's state: its stream and what the screen shows about it.
#[derive(Clone, Copy)]
struct Session {
    pane: u32,
    stream: u64,
    phase: Signal<Phase>,
    /// The terminal fed output since it was attached (the cover may go).
    painted: Signal<bool>,
    /// The terminal is the first responder (the keyboard is its).
    focused: Signal<bool>,
    /// Bumped to make a new terminal element (G5's rebuild test): NSH
    /// destroys the old element's native view, the host's table keeps the
    /// terminal.
    generation: Signal<u32>,
}

impl Session {
    fn new(pane: u32) -> Session {
        Session {
            pane,
            stream: core().pane(pane),
            phase: signal(Phase::Connecting),
            painted: signal(false),
            focused: signal(false),
            generation: signal(0),
        }
    }
}

/// The screen's state, shared by the root and its pages.
#[derive(Clone, Copy)]
struct App {
    routes: Signal<Vec<Route>>,
    /// The screen is alone in a SwiftUI stack: back tells the host.
    embedded: bool,
    shell: Session,
    tui: Session,
    /// The terminal on screen (one at a time): its `focus` and `blur`.
    term: ElementRef,
    /// The breadcrumb's window (⌘1–9, ⌘K).
    window: Signal<u32>,
    /// The banner under the top bar.
    banner: Signal<bool>,
}

impl App {
    fn new(routes: Vec<Route>, embedded: bool) -> App {
        App {
            routes: signal(routes),
            embedded,
            shell: Session::new(1),
            tui: Session::new(2),
            term: element_ref(),
            window: signal(1),
            banner: signal(false),
        }
    }

    fn session(&self, pane: u32) -> Session {
        if pane == 1 { self.shell } else { self.tui }
    }

    /// The pane on screen, if a terminal page is on top.
    fn top_pane(&self) -> Option<u32> {
        match self.routes.with_untracked(|r| r.last().cloned()) {
            Some(Route::Terminal(pane)) => Some(pane),
            _ => None,
        }
    }

    /// Back from the terminal: pop the page, or ask the SwiftUI stack to.
    fn back(&self) {
        if self.embedded {
            tell_host(json!({"ev": "back"}));
        } else {
            self.routes.update(|r| {
                if r.len() > 1 {
                    r.pop();
                }
            });
        }
    }

    /// A keyboard shortcut, from NSH's keymap (`via` "on_key") or from the
    /// host's `UIKeyCommand`s (`via` "host"): one handler for both.
    fn shortcut(&self, stroke: &str, via: &str) {
        log::info!(target: "moshpit", "g5 shortcut {stroke} via {via}");
        tell_host(json!({"ev": "shortcut", "stroke": stroke, "via": via}));
        match stroke {
            "mod+k" => self.window.update(|w| *w = *w % 3 + 1),
            "mod+w" => self.back(),
            _ => {
                if let Some(n) = stroke
                    .strip_prefix("mod+")
                    .and_then(|d| d.parse::<u32>().ok())
                {
                    self.window.set(n);
                }
            }
        }
    }

    /// What the host asked (`{"op": ...}`).
    fn host(&self, message: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(message) else {
            return log::warn!(target: "moshpit", "g5 host message is not JSON: {message}");
        };
        let op = value["op"].as_str().unwrap_or_default();
        let pane = value["pane"].as_u64().map(|p| p as u32);
        match op {
            "focus" | "blur" => self.term.command(op, json!(null)),
            "rekey" => {
                let session = self.session(pane.or(self.top_pane()).unwrap_or(1));
                // Tell the host the terminal is about to move to a new
                // element, before NSH takes the old one out of the window
                // (UIKit would resign the terminal, and the keyboard would
                // drop and come back). A command to the old element does not
                // arrive: its ref moves to the new element in the same turn.
                // `tell_host` runs now, on this thread; the host's hop to the
                // main queue is ahead of the batch that removes the element.
                if value["handover"].as_bool().unwrap_or(true) {
                    tell_host(json!({"ev": "handover", "stream": session.stream}));
                }
                session.generation.update(|g| *g += 1);
            }
            "pop" => self.routes.update(|r| r.truncate(1)),
            "push" => {
                let pane = pane.unwrap_or(1);
                self.routes.update(|r| r.push(Route::Terminal(pane)));
            }
            "shortcut" => self.shortcut(value["stroke"].as_str().unwrap_or_default(), "host"),
            "drop" => {
                let session = self.session(pane.or(self.top_pane()).unwrap_or(1));
                session.phase.set(Phase::Reconnecting);
                let ms = value["ms"].as_u64().unwrap_or(1600);
                set_timeout(std::time::Duration::from_millis(ms), move || {
                    session.phase.set(Phase::Live);
                });
            }
            "banner" => self.banner.update(|b| *b = !*b),
            // What the back button does.
            "back" => self.back(),
            _ => log::warn!(target: "moshpit", "g5 host op {op:?} unknown"),
        }
    }

    /// Hears the host for as long as the root lives.
    fn listen(self) {
        let mut messages = host_messages();
        spawn(async move {
            loop {
                match messages.recv().await {
                    Ok(message) => self.host(&message),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!(target: "moshpit", "g5 missed {n} host messages");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }
}

/// G5 in an NSH navigation stack: a home page with the panes, and the
/// terminal screen pushed over it (pane 1 open at launch).
#[component]
pub fn G5Root() -> impl IntoElement {
    let app = App::new(vec![Route::Home, Route::Terminal(1)], false);
    app.listen();
    nav_stack().flex_1().bg(hue::TERMINAL_BG).child(each(
        move || app.routes.get(),
        |route: &Route| format!("{route:?}"),
        move |route, index| {
            let page = match route.get_untracked() {
                Route::Home => home_page(app),
                Route::Terminal(pane) => nav_page()
                    .title("Terminal")
                    .header_visible(false)
                    .child(terminal_screen(app, app.session(pane))),
            };
            // Back natively (the swipe): drop the route and those above it.
            page.on_dismiss(move || app.routes.update(|r| r.truncate(index.get_untracked())))
        },
    ))
}

/// G5's terminal screen alone, for a SwiftUI navigation stack to host: the
/// host owns the navigation, the back button asks it to pop.
#[component]
pub fn G5Embedded() -> impl IntoElement {
    let app = App::new(vec![Route::Terminal(1)], true);
    app.listen();
    view()
        .flex_1()
        .bg(hue::TERMINAL_BG)
        .child(terminal_screen(app, app.shell))
}

/// The stack's first page: the panes, a row each.
fn home_page(app: App) -> Screen {
    let row = move |pane: u32, title: &'static str, detail: &'static str| {
        view()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .px(px(16.0))
            .py(px(14.0))
            .border_b_1()
            .border_color(hue::HAIRLINE)
            .active(|s| s.bg(hue::KEY))
            .role(AccessibilityRole::Button)
            .label(title)
            .test_id(&format!("g5-home-pane-{pane}"))
            .on_press(move || app.routes.update(|r| r.push(Route::Terminal(pane))))
            .child(view().w(px(8.0)).h(px(8.0)).rounded_full().bg(hue::SUCCESS))
            .child(
                view()
                    .flex_1()
                    .gap(px(2.0))
                    .child(text(title).text_size(px(16.0)).text_color(hue::PRIMARY))
                    .child(
                        text(detail)
                            .text_size(px(12.0))
                            .font_family(MONO)
                            .text_color(hue::META),
                    ),
            )
            .child(text("›").text_size(px(20.0)).text_color(hue::META))
    };
    nav_page().title("Moshpit").header_large_title(true).child(
        scroll()
            .flex_1()
            .bg(hue::TERMINAL_BG)
            .child(row(1, "dev · shell", "pane 1 · 3000 lines of history"))
            .child(row(2, "dev · claude", "pane 2 · full-screen app")),
    )
}

/// The terminal screen of `session`.
fn terminal_screen(app: App, session: Session) -> impl IntoElement {
    // The hardware keyboard's shortcuts, while this screen lives (NSH's
    // keymap gets the presses SwiftTerm leaves to its next responder). On a
    // Mac (Catalyst) ⌘W is the menu bar's File > Close, which AppKit runs
    // whether or not the press was taken: bound here too, it would go back
    // and close the window at once (G4).
    for stroke in [
        "mod+k", "mod+w", "mod+1", "mod+2", "mod+3", "mod+4", "mod+5", "mod+6", "mod+7", "mod+8",
        "mod+9",
    ] {
        if stroke == "mod+w" && is_catalyst() {
            continue;
        }
        on_key(stroke, move |s: &str| app.shortcut(s, "on_key"));
    }
    // What NSH sees of the surface, for the host's checks.
    effect(move || {
        let (keyboard, area, size) = (keyboard_height(), safe_area(), window());
        tell_host(json!({
            "ev": "env",
            "keyboard": keyboard.0,
            "keyboard_ms": keyboard_duration(),
            "safe": [area.top.0, area.right.0, area.bottom.0, area.left.0],
            "window": [size.width.0, size.height.0],
        }));
    });
    // The first connect's cover goes a beat after the pane painted.
    effect(move || {
        if session.painted.get() && session.phase.get_untracked() == Phase::Connecting {
            set_timeout(std::time::Duration::from_millis(500), move || {
                if session.phase.get_untracked() == Phase::Connecting {
                    session.phase.set(Phase::Live);
                }
            });
        }
    });
    view()
        .flex_1()
        .flex_col()
        .bg(hue::TERMINAL_BG)
        // The keyboard's height: the column ends where the keyboard starts,
        // moving with it.
        .mb_keyboard()
        .child(top_bar(app, session))
        .child(terminal_area(app, session))
        .child(shortcut_bar(app, session))
}

/// The top bar: back, transport pill, breadcrumb; a system bar's 44 pt.
fn top_bar(app: App, session: Session) -> impl IntoElement {
    view()
        .pt_safe_area()
        .bg(hue::BAR)
        .border_b_1()
        .border_color(hue::HAIRLINE)
        .test_id("g5-top-bar")
        .child(
            view()
                .h(px(44.0))
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .child(back_button(app))
                .child(transport_pill(app, session))
                .child(breadcrumb(app, session)),
        )
}

/// A 32 pt disc on NSH's own Liquid Glass (`glass_effect`, a thin
/// material's blur before iOS 26) in a 44 pt hit target: the breadcrumb's
/// glass is the host's component, this one is NSH's, to compare the two on
/// the device.
fn back_button(app: App) -> impl IntoElement {
    view()
        .w(px(44.0))
        .h(px(44.0))
        .items_center()
        .justify_center()
        .role(AccessibilityRole::Button)
        .label("Back")
        .test_id("terminal-back")
        .on_press(move || app.back())
        .child(
            view()
                .w(px(32.0))
                .h(px(32.0))
                .rounded_full()
                .items_center()
                .justify_center()
                // Glass where UIKit has it (iOS 26), a thin material's
                // blur below: NSH falls back from one to the other itself.
                .glass_effect(nsh_core::GlassEffect::Regular)
                .backdrop_blur(px(20.0))
                .backdrop_material(BackdropMaterial::Thin)
                .active(|s| s.opacity(0.6))
                .child(
                    text("‹")
                        .text_size(px(24.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hue::ACCENT)
                        .mt(px(-3.0)),
                ),
        )
}

/// The transport pill: mosh's blue dot and word; a press toggles the
/// banner (the spike's stand-in for the protocol switch).
fn transport_pill(app: App, session: Session) -> impl IntoElement {
    let dot = move || match session.phase.get() {
        Phase::Live => hue::SIGNAL,
        _ => hue::WARN,
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(9.0))
        .h(px(24.0))
        .rounded_full()
        .bg(hue::SIGNAL_WASH)
        .border_1()
        .border_color(hue::SIGNAL_BORDER)
        .role(AccessibilityRole::Button)
        .label("Transport")
        .test_id("terminal-transport-switch")
        .on_press(move || app.banner.update(|b| *b = !*b))
        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(dot))
        .child(
            text("mosh")
                .text_size(px(11.0))
                .font_family(MONO)
                .text_color(hue::SIGNAL_SOFT),
        )
}

/// The breadcrumb: session › window › pane on the host's Liquid Glass
/// component (`moshpit/glass`), the crumbs NSH's own views over it.
fn breadcrumb(app: App, session: Session) -> impl IntoElement {
    let sep = || text("›").text_size(px(9.0)).text_color(hue::META);
    let window = app.window;
    let pane_title = if session.pane == 1 { "zsh" } else { "claude" };
    view()
        .flex_1()
        .min_w(px(0.0))
        .h(px(32.0))
        .relative()
        .test_id("g5-breadcrumb")
        // The glass behind, filling the capsule: a native component is a
        // leaf, so the crumbs are its siblings, not its children.
        .child(
            native("moshpit/glass")
                .absolute()
                .inset_0()
                .prop("shape", json!("capsule"))
                .prop("interactive", json!(true)),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .h_full()
                .child(crumb(|| "dev".to_owned()))
                .child(sep())
                .child(crumb(move || format!("{}:moshpit", window.get())))
                .child(sep())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(6.0))
                        .py(px(4.0))
                        .rounded_full()
                        .bg(hue::WARN_WASH)
                        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(hue::WARN))
                        .child(
                            text(pane_title)
                                .text_size(px(11.0))
                                .font_family(MONO)
                                .text_color(hue::WARN),
                        ),
                ),
        )
}

/// A breadcrumb segment: mono text on a wash of the accent.
fn crumb(label: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .px(px(6.0))
        .py(px(4.0))
        .rounded_full()
        .bg(hue::ACCENT_WASH)
        .min_w(px(0.0))
        .child(
            text(label)
                .text_size(px(11.0))
                .font_family(MONO)
                .truncate()
                .text_color(hue::ACCENT),
        )
}

/// The terminal and what lies over it: the banner, the connecting cover,
/// the reconnect veil. The area itself takes no press.
fn terminal_area(app: App, session: Session) -> impl IntoElement {
    let cover = computed(move || session.phase.get() == Phase::Connecting);
    let veil = computed(move || session.phase.get() == Phase::Reconnecting);
    view()
        .flex_1()
        .relative()
        .overflow_hidden()
        .bg(hue::TERMINAL_BG)
        // A new element (and so a new native view) per generation; the
        // host re-parents the same terminal into it.
        .child(dynamic(move || {
            let generation = session.generation.get();
            untracked(|| terminal(app, session, generation))
        }))
        // The connecting cover: over the pane until it painted, taking the
        // touches meanwhile; fades out.
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(hue::TERMINAL_BG)
                .items_center()
                .justify_center()
                .gap(px(14.0))
                .opacity(move || if cover.get() { 1.0_f32 } else { 0.0 })
                .transition_opacity()
                .duration(280.0)
                .pointer_events(move || {
                    if cover.get() {
                        PointerEvents::Auto
                    } else {
                        PointerEvents::None
                    }
                })
                .test_id("g5-connecting-cover")
                .child(
                    text("Opening the pit")
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hue::PRIMARY),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded_full()
                        .border_1()
                        .border_color(hue::SIGNAL_BORDER)
                        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(hue::SIGNAL))
                        .child(
                            text("mosh · dev.example.com")
                                .text_size(px(11.0))
                                .font_family(MONO)
                                .text_color(hue::SECONDARY),
                        ),
                ),
        )
        // The reconnect veil: dims the kept frame, never takes a touch.
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(hue::VEIL)
                .items_center()
                .pt(px(18.0))
                .opacity(move || if veil.get() { 1.0_f32 } else { 0.0 })
                .transition_opacity()
                .duration(300.0)
                .pointer_events(PointerEvents::None)
                .test_id("g5-reconnect-veil")
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded_full()
                        .glass_effect(nsh_core::GlassEffect::Regular)
                        .backdrop_blur(px(20.0))
                        .backdrop_material(BackdropMaterial::UltraThin)
                        .border_1()
                        .border_color(hue::SIGNAL_BORDER)
                        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(hue::SIGNAL))
                        .child(
                            text("Reconnecting…")
                                .text_size(px(11.0))
                                .font_family(MONO)
                                .text_color(hue::PRIMARY),
                        ),
                ),
        )
        // The banner: a sibling at the top, over the terminal.
        .child(when(app.banner, move || banner(app)))
}

/// The terminal element of `session`'s pane, keyed by `generation`.
fn terminal(app: App, session: Session, generation: u32) -> NativeView {
    native("moshpit/terminal")
        .key(format!("terminal-{}-{generation}", session.pane))
        .element_ref(&app.term)
        .absolute()
        .inset_0()
        .prop("stream", json!(session.stream))
        .prop("generation", json!(generation))
        .test_id("g5-terminal-slot")
        .on_native_event(move |event: &NativeEvent| match &*event.name {
            "focus" => session.focused.set(event.data.as_bool().unwrap_or(false)),
            "painted" => session.painted.set(true),
            _ => {}
        })
}

/// The mosh return path banner (`MoshReturnPathBannerView`): a capsule card
/// under the top bar; a press dismisses it.
fn banner(app: App) -> impl IntoElement {
    view()
        .absolute()
        .top(px(10.0))
        .left(px(12.0))
        .right(px(12.0))
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(14.0))
        .py(px(10.0))
        .rounded(px(14.0))
        .bg(rgba(0x1A1410F2))
        .border_1()
        .border_color(rgba(0xFFB35C5C))
        .role(AccessibilityRole::Button)
        .label("Dismiss banner")
        .test_id("g5-banner")
        .on_press(move || app.banner.set(false))
        .child(view().w(px(8.0)).h(px(8.0)).rounded_full().bg(hue::WARN))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    text("No reply from the server for 6 s")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hue::PRIMARY),
                )
                .child(
                    text("mosh's UDP return path looks closed. Switch to SSH?")
                        .text_size(px(11.0))
                        .text_color(hue::SECONDARY),
                ),
        )
}

/// The keys the bar sends, and the bytes.
const CHIPS: &[(&str, &[u8])] = &[
    ("esc", b"\x1b"),
    ("tab", b"\t"),
    ("ctrl-c", b"\x03"),
    ("↑", b"\x1b[A"),
    ("↓", b"\x1b[B"),
    ("←", b"\x1b[D"),
    ("→", b"\x1b[C"),
    ("|", b"|"),
    ("~", b"~"),
    ("/", b"/"),
    ("-", b"-"),
];

/// The shortcut bar: chips that write to the pane's input (the core's
/// queue, as the keyboard does), and the keyboard toggle. Above the home
/// indicator while the keyboard is down, on the keyboard while it is up.
fn shortcut_bar(app: App, session: Session) -> impl IntoElement {
    let chip = move |label: &'static str, bytes: &'static [u8]| {
        view()
            .h(px(30.0))
            .min_w(px(36.0))
            .px(px(10.0))
            .items_center()
            .justify_center()
            .rounded(px(7.0))
            .bg(hue::KEY)
            .border_1()
            .border_color(hue::KEY_BORDER)
            .active(|s| s.bg(hue::ACCENT))
            .role(AccessibilityRole::Button)
            .label(label)
            .on_press(move || {
                moshpit_stream::view::write(session.stream, bytes);
            })
            .child(
                text(label)
                    .text_size(px(13.0))
                    .font_family(MONO)
                    .text_color(hue::PRIMARY),
            )
    };
    let toggle = view()
        .w(px(40.0))
        .h(px(30.0))
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .bg(move || {
            if session.focused.get() {
                hue::ACCENT
            } else {
                hue::KEY
            }
        })
        .border_1()
        .border_color(hue::KEY_BORDER)
        .role(AccessibilityRole::Button)
        .label("Keyboard")
        .test_id("g5-keyboard-toggle")
        .on_press(move || {
            // Focus is the component's own command: NSH has no focus for
            // native views.
            let name = if session.focused.get_untracked() {
                "blur"
            } else {
                "focus"
            };
            log::info!(target: "moshpit", "g5 keyboard toggle: {name}");
            tell_host(json!({"ev": "toggle", "command": name}));
            app.term.command(name, json!(null));
        })
        .child(
            text(move || if session.focused.get() { "⌄" } else { "⌨" })
                .text_size(px(15.0))
                .text_color(hue::PRIMARY),
        );
    view()
        .bg(hue::SHORTCUT_BAR)
        .border_t_1()
        .border_color(hue::HAIRLINE)
        // The home indicator's inset only while the keyboard is down: up,
        // the keyboard covers it (and the column's margin is its height).
        .pb(move || {
            px(if keyboard_height() > px(0.0) {
                0.0
            } else {
                safe_area().bottom.0
            })
        })
        .test_id("g5-shortcut-bar")
        .child(
            view()
                .h(px(44.0))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(8.0))
                .child(
                    scroll().horizontal().flex_1().scrollbar(false).child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .children(CHIPS.iter().map(|&(label, bytes)| chip(label, bytes))),
                    ),
                )
                .child(toggle),
        )
}
