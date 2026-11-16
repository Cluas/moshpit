//! What the screens' headless tests share: a screen on an iPad-sized iOS
//! engine (the device the app's tests run on), and what the screen tells
//! its host.

#![allow(dead_code)]

pub mod a11y;
pub mod home_fixture;
pub mod settings_fixture;
pub mod text_engine;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use moshpit_ui::{
    channel::Channel,
    screen::{Params, Screen},
};
use nsh_core::{
    Points, Size,
    platform::{HostEnvironment, Idiom, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

pub const WAIT: Duration = Duration::from_secs(10);

/// A headless app on `config`, its text measured by
/// [`text_engine::WideTextEngine`]: Approx's, with East Asian wide
/// characters as wide as they are on a phone.
pub fn headless(config: EngineConfig) -> Headless {
    Headless::start_with_text_engine(config, || text_engine::WideTextEngine)
        .expect("the engine starts")
}

/// How long a step waits for something that is sure to come (the screen
/// applying a post) before it calls the screen stuck: only a broken screen
/// reaches it, so it is long enough for any machine's load.
pub const HANG: Duration = Duration::from_secs(300);

/// NSH's settle gives up after this much real time (or app time), and its
/// steps go on regardless: on a loaded machine a step can return before the
/// engine did what it set off.
const NSH_SETTLE: Duration = Duration::from_secs(2);

/// A headless app whose every step returns once the engine has done what
/// the step set off and its batches are applied ([`steady`]), however long
/// a loaded machine takes: a read straight after a step sees the step. What
/// it reads goes straight to the [`Headless`] inside.
pub struct App(pub Headless);

impl std::ops::Deref for App {
    type Target = Headless;
    fn deref(&self) -> &Headless {
        &self.0
    }
}

impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Headless {
        &mut self.0
    }
}

impl App {
    /// `step` on the app, then [`steady`].
    fn stepped<T>(&mut self, step: impl FnOnce(&mut Headless) -> T) -> T {
        let result = step(&mut self.0);
        steady(&mut self.0);
        result
    }

    pub fn press(&mut self, selector: &str) -> Result<(), String> {
        self.stepped(|app| app.press(selector))
    }

    pub fn long_press(&mut self, selector: &str) -> Result<(), String> {
        self.stepped(|app| app.long_press(selector))
    }

    pub fn press_key(&mut self, keys: &str) -> Result<(), String> {
        self.stepped(|app| app.press_key(keys))
    }

    pub fn type_text(&mut self, selector: &str, text: &str) -> Result<(), String> {
        self.stepped(|app| app.type_text(selector, text))
    }

    pub fn clear(&mut self, selector: &str) -> Result<(), String> {
        self.stepped(|app| app.clear(selector))
    }

    pub fn drag(
        &mut self,
        selector: &str,
        by: nsh_core::geometry::Point<Points>,
    ) -> Result<(), String> {
        self.stepped(|app| app.drag(selector, by))
    }

    pub fn set_environment(
        &mut self,
        what: &str,
        change: impl FnOnce(&mut nsh_core::platform::Environment),
    ) -> Result<(), String> {
        self.stepped(|app| app.set_environment(what, change))
    }

    pub fn answer_dialog(&mut self, answer: nsh_testing::DialogAnswer) -> Result<(), String> {
        self.stepped(|app| app.answer_dialog(answer))
    }

    /// NSH's settle, then [`steady`]: true unless the app keeps animating.
    pub fn settle(&mut self) -> bool {
        self.stepped(Headless::settle)
    }
}

/// Settles `app` until the engine is idle and its batches are applied, for
/// as long as that takes in real time ([`HANG`] at most: only a stuck
/// engine needs that). NSH's own settle stops after [`NSH_SETTLE`] of real
/// time; this one goes on, and stops early only when the app's own time ran
/// that far (it keeps animating: as settled as NSH would have it).
pub fn steady(app: &mut Headless) {
    let deadline = std::time::Instant::now() + HANG;
    loop {
        let before = app.clock().now();
        if app.settle() {
            return;
        }
        if app.clock().now().saturating_duration_since(before) >= NSH_SETTLE {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the engine never settled"
        );
    }
}

/// `screen` on an iPad mini in Settings' sheet (580 pt wide), iOS 18,
/// with what the host says (`params`), waiting for its root (`root_id`).
pub fn start(screen: Screen, params: Params, root_id: &str) -> (App, Arc<Channel>) {
    let (app, channel, _) = start_observing(screen, params, root_id);
    (app, channel)
}

/// Whether the view `test_id` is hidden (`visibility: hidden`, it or a
/// view around it); `None` when there is no such view.
pub fn hidden(app: &Headless, test_id: &str) -> Option<bool> {
    app.screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some(test_id))
        .map(|node| node.hidden)
}

/// The bar's look behind its items, as `scroll` (the page's list scrolled
/// that far, or back) leaves it: on iOS 26 the scroll edge effect, below
/// the classic background; neither shows at the scroll edge, the one the
/// OS draws shows once content is under the bar, the other is never made.
pub fn assert_bar_follows_scrolling(app: &mut Headless, list: &str, glass: bool) {
    let (drawn, never) = if glass {
        ("nav-bar-edge", "nav-bar-background")
    } else {
        ("nav-bar-background", "nav-bar-edge")
    };
    assert_eq!(hidden(app, never), None, "{never} on this OS");
    assert_eq!(hidden(app, drawn), Some(true), "{drawn} at the scroll edge");
    app.scroll(&format!("id:{list}"), Points(120.0)).unwrap();
    steady(app);
    assert_eq!(
        hidden(app, drawn),
        Some(false),
        "{drawn} with content under the bar"
    );
    app.scroll(&format!("id:{list}"), Points(-120.0)).unwrap();
    steady(app);
    assert_eq!(
        hidden(app, drawn),
        Some(true),
        "{drawn} back at the scroll edge"
    );
}

/// A native component as the engine made it: its name and its props as
/// last set.
type Native = (String, HashMap<String, serde_json::Value>);

/// The host's native components on a screen, by their container's node.
#[derive(Clone, Default)]
pub struct Natives(Arc<Mutex<HashMap<nsh_core::NodeId, Native>>>);

impl Natives {
    /// The node of the `name` component whose `identifier` prop is `id`.
    pub fn find(&self, name: &str, id: &str) -> Option<nsh_core::NodeId> {
        let natives = self.0.lock().unwrap();
        let mut found = natives.iter().filter(|(_, (n, props))| {
            n == name && props.get("identifier").and_then(|v| v.as_str()) == Some(id)
        });
        let hit = found.next().map(|(node, _)| *node);
        assert!(found.next().is_none(), "{name} {id}: more than one");
        hit
    }

    /// The nodes of every `name` component.
    pub fn all(&self, name: &str) -> Vec<nsh_core::NodeId> {
        let natives = self.0.lock().unwrap();
        let mut nodes: Vec<_> = natives
            .iter()
            .filter(|(_, (n, _))| n == name)
            .map(|(node, _)| *node)
            .collect();
        nodes.sort();
        nodes
    }

    /// The node of the one `name` component.
    pub fn only(&self, name: &str) -> Option<nsh_core::NodeId> {
        let natives = self.0.lock().unwrap();
        let mut found = natives.iter().filter(|(_, (n, _))| n == name);
        let hit = found.next().map(|(node, _)| *node);
        assert!(found.next().is_none(), "{name}: more than one");
        hit
    }

    /// Prop `prop` of node `node`.
    pub fn prop(&self, node: nsh_core::NodeId, prop: &str) -> Option<serde_json::Value> {
        self.0.lock().unwrap().get(&node)?.1.get(prop).cloned()
    }

    /// The names of the props node `node` was given, sorted.
    pub fn props(&self, node: nsh_core::NodeId) -> Vec<String> {
        let natives = self.0.lock().unwrap();
        let mut names: Vec<String> = natives
            .get(&node)
            .map(|(_, props)| props.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// How many `name` components there are.
    pub fn count(&self, name: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .values()
            .filter(|(n, _)| n == name)
            .count()
    }

    fn apply(&self, batch: &nsh_core::MountBatch) {
        let mut natives = self.0.lock().unwrap();
        for item in &batch.items {
            match item {
                nsh_core::MountItem::Component { id, name } => {
                    natives.insert(*id, (name.to_string(), HashMap::new()));
                }
                nsh_core::MountItem::ComponentProps { id, props } => {
                    if let Some((_, current)) = natives.get_mut(id) {
                        for (name, value) in props {
                            match value {
                                Some(value) => current.insert(name.to_string(), value.clone()),
                                None => current.remove(&**name),
                            };
                        }
                    }
                }
                nsh_core::MountItem::Delete { id } => {
                    natives.remove(id);
                }
                _ => {}
            }
        }
    }
}

/// A native component tells its screen `name` with `data`, as the host's
/// component would (`mp_ui_component_emit`).
pub fn emit(app: &mut Headless, node: nsh_core::NodeId, name: &str, data: serde_json::Value) {
    app.engine().control().dispatch_event(
        node,
        nsh_core::EventKind::NativeEvent,
        nsh_core::EventPayload::Native {
            name: name.into(),
            data,
        },
    );
    // The event went to the engine ahead of the flush: once it is done,
    // the event's batches are in.
    flushed(app);
    steady(app);
}

/// Has the engine run everything sent to it so far (and what that queued
/// in turn), then applies the batches it made ([`nsh_engine::EngineControl::flush`]:
/// its `done` runs once they were submitted). What a step waits for,
/// rather than for no batch to come for 100 ms, which a loaded machine
/// reaches before the engine even took the step.
pub fn flushed(app: &mut Headless) {
    let (tx, rx) = mpsc::channel();
    app.engine().control().flush(move |_| {
        let _ = tx.send(());
    });
    // A gone engine answers nothing: nothing more comes from it either.
    let _ = rx.recv_timeout(HANG);
    while app.next_batch(Duration::ZERO).is_some() {}
}

/// [`start`], keeping track of the host's native components it makes.
pub fn start_observing(
    screen: Screen,
    params: Params,
    root_id: &str,
) -> (App, Arc<Channel>, Natives) {
    start_observing_with(screen, params, root_id, |_| {})
}

/// [`start`] on iOS `version` (the iPad mini's iOS 18.2 otherwise): iOS 26
/// draws the bars with Liquid Glass.
pub fn start_on(
    screen: Screen,
    params: Params,
    root_id: &str,
    version: (u16, u16),
) -> (App, Arc<Channel>) {
    let (app, channel, _) = start_observing_with(screen, params, root_id, move |config| {
        config.environment.os_version = version;
    });
    (app, channel)
}

/// [`start_observing`], the engine's config changed by `change` before it
/// starts.
pub fn start_observing_with(
    screen: Screen,
    params: Params,
    root_id: &str,
    change: impl FnOnce(&mut EngineConfig),
) -> (App, Arc<Channel>, Natives) {
    let channel = Channel::open();
    let root = screen.root(&channel, &params);
    let mut config = EngineConfig::new(root);
    config.viewport = Size::new(Points(580.0), Points(1000.0));
    config.environment.os = Os::Ios;
    config.environment.os_version = (18, 2);
    config.environment.idiom = Idiom::Pad;
    config.environment.pixel_ratio = 2.0;
    // A manual clock (the app's time moves only as the driver moves it): a
    // step settles when the engine says it is idle, not after 100 ms with
    // no batch, and a wait moves the clock a frame at a time — so nothing a
    // test reads depends on how loaded the machine is.
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    // What the iOS host does with the params' language and text size
    // (`AppView::set_locale`, `set_font_scale`).
    HostEnvironment {
        locale: params.nsh_locale(),
        font_scale: params.font_scale,
    }
    .apply(&mut config.environment);
    change(&mut config);
    let mut app = headless(config);
    let natives = Natives::default();
    let observed = natives.clone();
    app.observe(move |_, batch| observed.apply(batch));
    app.wait_for(&format!("id:{root_id}")).unwrap();
    (App(app), channel, natives)
}

/// Posts `json` (one of the screen's messages) and returns once the screen
/// has applied it and the batches that made are applied: a read straight
/// after it sees the post, however loaded the machine. The screen takes a
/// post on its own task ([`moshpit_ui::channel::hear`]); once the channel
/// says every post was applied ([`Channel::all_taken`]), an engine flush
/// after that brings the batches in. Then the usual settle (native
/// animations, what the batches set off).
pub fn post(app: &mut Headless, channel: &Channel, json: &str) {
    channel.post(json).unwrap();
    take(app, channel, json);
    steady(app);
}

/// Waits until the screen has applied every post so far, and applies the
/// batches they made; `what` names the post in the failure.
fn take(app: &mut Headless, channel: &Channel, what: &str) {
    let deadline = std::time::Instant::now() + HANG;
    loop {
        // Read before the flush: then everything counted ran before it.
        let taken = channel.all_taken();
        flushed(app);
        if taken {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the screen never applied {what}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Waits for the screen to show a dialog, by [`WAIT`] at most: true once
/// one is pending. For a dialog that comes after something asynchronous
/// (a task the step started, a timer): a manual clock moves a frame at a
/// time while it waits. A dialog a post or a press shows at once is there
/// when the step returns.
pub fn wait_dialog(app: &mut Headless) -> bool {
    let deadline = std::time::Instant::now() + WAIT;
    while app.pending_dialog().is_none() && std::time::Instant::now() < deadline {
        let _ = app.next_change(Duration::from_millis(50));
    }
    app.pending_dialog().is_some()
}

/// The labels of `dialog`'s buttons (an action sheet's options), in order.
pub fn dialog_buttons(dialog: &nsh_core::Dialog) -> Vec<&str> {
    dialog.buttons.iter().map(|b| b.text.as_str()).collect()
}

/// What the screen tells the host, from now on.
pub fn hear(channel: &Channel) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    channel.set_listener(Some(Box::new(move |json| {
        let _ = tx.send(json.to_owned());
    })));
    rx
}

/// The next thing the screen told the host, as JSON.
pub fn next(rx: &mpsc::Receiver<String>) -> serde_json::Value {
    let json = rx.recv_timeout(WAIT).expect("the screen tells the host");
    serde_json::from_str(&json).unwrap()
}

/// Nothing more was told.
pub fn quiet(rx: &mpsc::Receiver<String>) {
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "the screen told the host something more"
    );
}

/// The app's language: Simplified Chinese.
pub fn zh_hans() -> Params {
    Params {
        locale: Some("zh-Hans".into()),
        ..Params::default()
    }
}
