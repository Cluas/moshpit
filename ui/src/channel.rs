//! A surface's channel with its host: the small data path between the
//! native app and the screen an NSH surface runs.
//!
//! The UI only knows the data and events it is given (`docs/nsh/DESIGN.md`
//! §3.2). Until the core owns them, the host does: the Recent Log screen
//! shows the log the Swift app reads from `OSLogStore`, posted here, and
//! tells the host when the user goes back. Messages are JSON objects, as
//! NSH's own control path is, one per user action or state change — the
//! host posts `{"op": …}` and the screen tells the host `{"ev": …}`. Bytes
//! never come this way (the terminal's have their own channel,
//! `crates/stream`).
//!
//! - Host → screen ([`Channel::post`], any thread): checked at once against
//!   the running screen's messages, so a host learns of a message the screen
//!   would not understand when it posts it; then queued, in order, for the
//!   screen's task on NSH's engine thread ([`Channel::take_inbox`]). Posts
//!   between running the screen and its view being made wait in the queue.
//! - Screen → host ([`Channel::tell`], on the engine thread): the host's
//!   listener is called before `tell` returns; it hops to its own thread.
//!   Clearing the listener waits for a call in progress, so after
//!   [`Channel::set_listener`]`(None)` returns nothing calls the old one.
//!
//! A screen finds its surface's channel by the surface's id
//! ([`Channel::find`]), which its root's props carry.
//!
//! The channel counts the posts the running screen has applied
//! ([`Channel::all_taken`]), so a headless driver can wait for exactly the
//! post it made, not for a quiet spell that a loaded machine reaches first.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::sync::mpsc;

use crate::screen::{Params, Screen};

/// A surface's id: what a screen's props carry to find its channel.
pub type SurfaceId = u64;

/// What hears the screen: called with a JSON object (`{"ev": …}`) on NSH's
/// engine thread.
pub type Listener = Box<dyn Fn(&str) + Send + Sync>;

/// One surface's channel with its host.
pub struct Channel {
    id: SurfaceId,
    inbox: Mutex<Inbox>,
    listener: Mutex<Option<Listener>>,
}

/// What the host posts, on its way to the running screen.
struct Inbox {
    /// The screen that runs; `None` before the first.
    screen: Option<Screen>,
    /// What the host said about it.
    params: Params,
    tx: mpsc::UnboundedSender<String>,
    /// Until the screen's view takes it.
    rx: Option<mpsc::UnboundedReceiver<String>>,
    /// Posts sent to this screen, and how many of them its view has applied
    /// (or skipped as unreadable): the screen's own, so a screen started in
    /// its place counts from nothing.
    posted: u64,
    taken: Arc<AtomicU64>,
    /// The screen's view took the inbox ([`hear`]): it runs and listens.
    heard: bool,
    /// The ops of posts refused so far, each logged once (B366).
    refused: HashSet<String>,
    /// Posts that found the screen's view gone (its task ended): checked,
    /// but nobody applies them.
    lost: u64,
}

impl Inbox {
    fn new(screen: Option<Screen>, params: Params) -> Inbox {
        let (tx, rx) = mpsc::unbounded_channel();
        Inbox {
            screen,
            params,
            tx,
            rx: Some(rx),
            posted: 0,
            taken: Arc::new(AtomicU64::new(0)),
            heard: false,
            refused: HashSet::new(),
            lost: 0,
        }
    }
}

/// What became of the host's posts to the running screen
/// ([`Channel::counts`]): what a host's watchdog reads a while after the
/// screen runs, to tell a screen that never started from one never told
/// anything or one that stopped listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// The screen's view took its inbox: it runs and listens.
    pub heard: bool,
    /// Posts made to it (refused ones not counted).
    pub posted: u64,
    /// Posts its view has applied (or skipped as unreadable).
    pub applied: u64,
    /// Posts that found its view gone: it stopped listening (its task
    /// ended, as a panic in it ends it) while the surface still runs it.
    pub lost: u64,
}

static CHANNELS: LazyLock<Mutex<HashMap<SurfaceId, Arc<Channel>>>> = LazyLock::new(Mutex::default);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Channel {
    /// A new channel with an id of its own, findable until [`Self::close`].
    pub fn open() -> Arc<Channel> {
        let channel = Arc::new(Channel {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            inbox: Mutex::new(Inbox::new(None, Params::default())),
            listener: Mutex::new(None),
        });
        lock(&CHANNELS).insert(channel.id, channel.clone());
        channel
    }

    /// The channel of surface `id`, while it is open.
    pub fn find(id: SurfaceId) -> Option<Arc<Channel>> {
        lock(&CHANNELS).get(&id).cloned()
    }

    /// Its surface's id.
    pub fn id(&self) -> SurfaceId {
        self.id
    }

    /// The surface went away: nobody finds the channel any more, its screen
    /// hears no more posts (its task ends) and the host's listener is let go.
    pub fn close(&self) {
        lock(&CHANNELS).remove(&self.id);
        let mut closed = Inbox::new(None, Params::default());
        closed.rx = None;
        *lock(&self.inbox) = closed;
        self.set_listener(None);
    }

    /// `screen` runs from now on with `params` (in place of the one before,
    /// whose messages are dropped): posts are its messages and wait for its
    /// view.
    pub(crate) fn start(&self, screen: Screen, params: Params) {
        *lock(&self.inbox) = Inbox::new(Some(screen), params);
    }

    /// What the host said about the running screen.
    pub fn params(&self) -> Params {
        lock(&self.inbox).params.clone()
    }

    /// The screen that runs, if one does.
    pub fn screen(&self) -> Option<Screen> {
        lock(&self.inbox).screen
    }

    /// Posts `json` to the running screen, after what was posted before.
    /// Any thread. `Err` (nothing posted) when no screen runs or `json` is
    /// not one of its messages: the error says why.
    pub fn post(&self, json: &str) -> Result<(), String> {
        let refused = self.try_post(json);
        if let Err(error) = &refused {
            // Once per op and screen: a host posting the same refused state
            // on every change would bury everything else (B366).
            let op = split_op("post", json).map_or_else(|_| "?".to_owned(), |(op, _)| op);
            if lock(&self.inbox).refused.insert(op.clone()) {
                log::error!(target: "moshpit", "post {op:?} refused: {error}");
            }
        }
        refused
    }

    fn try_post(&self, json: &str) -> Result<(), String> {
        let mut inbox = lock(&self.inbox);
        let Some(screen) = inbox.screen else {
            return Err("no screen runs on this surface".to_owned());
        };
        screen.check(json)?;
        // The receiver is gone only once the screen's view went away: what
        // it would have shown has nowhere to go, which is no error (and no
        // post to wait for).
        if inbox.tx.send(json.to_owned()).is_ok() {
            inbox.posted += 1;
        } else {
            inbox.lost += 1;
        }
        Ok(())
    }

    /// What the host posts to the running screen, in order: the screen's
    /// view takes it once (`None` after that, or before a screen runs).
    pub fn take_inbox(&self) -> Option<mpsc::UnboundedReceiver<String>> {
        self.take_counted_inbox().map(|(rx, _)| rx)
    }

    /// [`Self::take_inbox`], with what its reader counts each post on once
    /// it has applied it ([`Self::all_taken`]).
    fn take_counted_inbox(&self) -> Option<(mpsc::UnboundedReceiver<String>, Arc<AtomicU64>)> {
        let mut inbox = lock(&self.inbox);
        let taken = inbox.taken.clone();
        let rx = inbox.rx.take()?;
        inbox.heard = true;
        Some((rx, taken))
    }

    /// What became of the host's posts to the running screen so far.
    pub fn counts(&self) -> Counts {
        let inbox = lock(&self.inbox);
        Counts {
            heard: inbox.heard,
            posted: inbox.posted,
            applied: inbox.taken.load(Ordering::Acquire),
            lost: inbox.lost,
        }
    }

    /// Whether the running screen's view has applied every post made to it
    /// so far ([`hear`] counts a post once its `apply` returned). A headless
    /// driver's step waits for it, then for the engine to submit what the
    /// posts made, instead of for the batches to stop for a while.
    pub fn all_taken(&self) -> bool {
        let inbox = lock(&self.inbox);
        inbox.taken.load(Ordering::Acquire) >= inbox.posted
    }

    /// Sets (`None`: clears) what hears the screen. Clearing waits for a
    /// call in progress; the listener must not call this itself.
    pub fn set_listener(&self, listener: Option<Listener>) {
        *lock(&self.listener) = listener;
    }

    /// Tells the host `event` (a JSON object), if it listens.
    pub fn tell(&self, event: &serde_json::Value) {
        let listener = lock(&self.listener);
        if let Some(listener) = listener.as_ref() {
            listener(&event.to_string());
        }
    }
}

/// Hears the host's posts to `screen` on `channel` from the screen's own
/// task (on NSH's engine thread, owned by the component that calls this, so
/// it ends with it): each one read by `parse` and handed to `apply`, in
/// order. A post the screen cannot read is logged and skipped (the channel
/// checked it when it was posted, so this is a bug).
pub fn hear<M: 'static>(
    channel: Option<&Arc<Channel>>,
    screen: Screen,
    parse: fn(&str) -> Result<M, String>,
    apply: impl Fn(M) + 'static,
) {
    let Some((mut inbox, taken)) = channel.and_then(|channel| channel.take_counted_inbox()) else {
        log::warn!(target: "moshpit", "{}: no channel to hear the host on", screen.name());
        return;
    };
    nsh_core::prelude::spawn(async move {
        while let Some(json) = inbox.recv().await {
            match parse(&json) {
                Ok(message) => apply(message),
                Err(error) => log::warn!(target: "moshpit", "{error}"),
            }
            taken.fetch_add(1, Ordering::Release);
        }
    });
}

/// The JSON object of a host's message: its `"op"` taken out, the rest
/// left for the op's fields. `screen` names the screen in errors.
pub fn split_op(
    screen: &str,
    json: &str,
) -> Result<(String, serde_json::Map<String, serde_json::Value>), String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("{screen}: not JSON: {e}"))?;
    let serde_json::Value::Object(mut object) = value else {
        return Err(format!("{screen}: a message is a JSON object"));
    };
    match object.remove("op") {
        Some(serde_json::Value::String(op)) => Ok((op, object)),
        _ => Err(format!("{screen}: a message has a string \"op\"")),
    }
}

impl std::fmt::Debug for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channel")
            .field("id", &self.id)
            .field("screen", &self.screen())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc as std_mpsc;

    use serde_json::json;

    use super::*;

    const LOADING: &str = r#"{"op":"log","state":"loading"}"#;

    fn drained(rx: &mut mpsc::UnboundedReceiver<String>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(message) = rx.try_recv() {
            out.push(message);
        }
        out
    }

    #[test]
    fn a_channel_is_found_by_its_id_until_it_closes() {
        let a = Channel::open();
        let b = Channel::open();
        assert_ne!(a.id(), b.id());
        assert!(Arc::ptr_eq(&Channel::find(a.id()).unwrap(), &a));
        a.close();
        assert!(Channel::find(a.id()).is_none());
        assert!(Channel::find(b.id()).is_some());
        b.close();
    }

    #[test]
    fn posts_need_a_running_screen_and_one_of_its_messages() {
        let channel = Channel::open();
        assert_eq!(
            channel.post(LOADING),
            Err("no screen runs on this surface".to_owned())
        );
        channel.start(Screen::RecentLog, Params::default());
        assert_eq!(channel.screen(), Some(Screen::RecentLog));
        assert!(channel.post(LOADING).is_ok());
        let error = channel.post(r#"{"op":"focus"}"#).unwrap_err();
        assert!(error.contains("focus"), "{error}");
        assert!(channel.post("not json").is_err());
        let mut inbox = channel.take_inbox().unwrap();
        assert_eq!(drained(&mut inbox), [LOADING], "only what was accepted");
        channel.close();
    }

    #[test]
    fn all_taken_counts_the_running_screens_posts() {
        let channel = Channel::open();
        channel.start(Screen::RecentLog, Params::default());
        assert!(channel.all_taken(), "nothing posted yet");
        channel.post(LOADING).unwrap();
        channel.post(LOADING).unwrap();
        assert!(!channel.all_taken(), "posted, not yet applied");
        let (rx, taken) = channel.take_counted_inbox().unwrap();
        taken.fetch_add(1, Ordering::Release);
        assert!(!channel.all_taken(), "one of two applied");
        taken.fetch_add(1, Ordering::Release);
        assert!(channel.all_taken());
        assert!(channel.post("not json").is_err());
        assert!(channel.all_taken(), "a refused post is nothing to wait for");
        drop(rx);
        channel.post(LOADING).unwrap();
        assert!(
            channel.all_taken(),
            "a post to a view that is gone is nothing to wait for"
        );
        // A screen started in its place counts from nothing: the old
        // reader's counts are not its.
        channel.start(Screen::RecentLog, Params::default());
        channel.post(LOADING).unwrap();
        taken.fetch_add(5, Ordering::Release);
        assert!(!channel.all_taken());
        channel.close();
    }

    #[test]
    fn the_screen_gets_every_post_in_order_from_any_thread() {
        let channel = Channel::open();
        channel.start(Screen::RecentLog, Params::default());
        let messages: Vec<String> = (0..200)
            .map(|i| {
                json!({"op": "log", "state": "failed", "message": format!("m{i}")}).to_string()
            })
            .collect();
        let poster = {
            let channel = channel.clone();
            let messages = messages.clone();
            std::thread::spawn(move || {
                for message in &messages {
                    channel.post(message).unwrap();
                }
            })
        };
        poster.join().unwrap();
        let mut inbox = channel.take_inbox().unwrap();
        assert_eq!(drained(&mut inbox), messages);
        assert!(channel.take_inbox().is_none(), "taken once");
        channel.close();
    }

    #[test]
    fn a_new_screen_starts_with_an_empty_queue_and_the_old_one_hears_no_more() {
        let channel = Channel::open();
        channel.start(Screen::RecentLog, Params::default());
        channel.post(LOADING).unwrap();
        let mut old = channel.take_inbox().unwrap();
        channel.start(Screen::RecentLog, Params::default());
        channel
            .post(r#"{"op":"log","state":"loaded","entries":[]}"#)
            .unwrap();
        assert_eq!(drained(&mut old), [LOADING]);
        assert!(matches!(
            old.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
        let mut new = channel.take_inbox().unwrap();
        assert_eq!(drained(&mut new).len(), 1);
        channel.close();
        assert!(channel.post(LOADING).is_err(), "closed: no screen runs");
    }

    #[test]
    fn the_host_hears_what_the_screen_tells_it_until_it_stops_listening() {
        let channel = Channel::open();
        let (tx, rx) = std_mpsc::channel();
        channel.set_listener(Some(Box::new(move |json| {
            tx.send(json.to_owned()).unwrap();
        })));
        channel.tell(&json!({"ev": "back"}));
        assert_eq!(rx.try_recv().unwrap(), r#"{"ev":"back"}"#);
        channel.set_listener(None);
        channel.tell(&json!({"ev": "back"}));
        assert!(rx.try_recv().is_err());
        channel.close();
    }

    #[test]
    fn clearing_the_listener_waits_for_a_call_in_progress() {
        let channel = Channel::open();
        let (entered_tx, entered) = std_mpsc::channel();
        let (release, release_rx) = std_mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let done = Arc::new(Mutex::new(false));
        let done_in_call = done.clone();
        channel.set_listener(Some(Box::new(move |_| {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            *done_in_call.lock().unwrap() = true;
        })));
        let teller = {
            let channel = channel.clone();
            std::thread::spawn(move || channel.tell(&json!({"ev": "back"})))
        };
        entered.recv().unwrap();
        let clearer = {
            let channel = channel.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                channel.set_listener(None);
                // Cleared only after the call returned.
                assert!(*done.lock().unwrap());
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!clearer.is_finished(), "clearing waits for the call");
        release.send(()).unwrap();
        teller.join().unwrap();
        clearer.join().unwrap();
        channel.close();
    }

    #[test]
    fn the_counts_tell_a_screen_that_never_listened_from_one_that_stopped() {
        let channel = Channel::open();
        channel.start(Screen::RecentLog, Params::default());
        assert_eq!(channel.counts(), Counts::default(), "nothing yet");
        channel.post(LOADING).unwrap();
        let counts = channel.counts();
        assert!(!counts.heard, "its view never took the inbox");
        assert_eq!((counts.posted, counts.applied, counts.lost), (1, 0, 0));
        let (rx, taken) = channel.take_counted_inbox().unwrap();
        taken.fetch_add(1, Ordering::Release);
        let counts = channel.counts();
        assert!(counts.heard);
        assert_eq!((counts.posted, counts.applied, counts.lost), (1, 1, 0));
        // Its task ended (a panic ends it): what comes next has nowhere to go.
        drop(rx);
        channel.post(LOADING).unwrap();
        let counts = channel.counts();
        assert_eq!((counts.posted, counts.applied, counts.lost), (1, 1, 1));
        // A refused post is not counted at all.
        assert!(channel.post(r#"{"op":"nope"}"#).is_err());
        assert_eq!(channel.counts().posted, 1);
        // A screen run again counts from nothing.
        channel.start(Screen::RecentLog, Params::default());
        assert_eq!(channel.counts(), Counts::default());
        channel.close();
    }
}
