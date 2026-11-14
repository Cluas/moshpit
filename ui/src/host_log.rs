//! Where the screens' log goes (B366): stderr, as before, and the host's
//! handler for warnings and errors, so they reach the app's own log.
//!
//! On a phone stderr goes nowhere anyone can read: a screen whose post was
//! refused, whose body panicked or whose engine never started said so only
//! there. The iOS host sets a handler (`mp_ui_set_log_handler`) that hands
//! each warning and error to its `Log.ui`, which the app's diagnostics log
//! shows. The logger itself is the process's one `log` logger, installed
//! once ([`install`]); it is plain Rust, tested here without a host.

use std::{
    cell::Cell,
    ffi::CString,
    sync::{Arc, RwLock},
};

/// What hears a forwarded record: its level, target and message. Called on
/// whatever thread logged it, so it must be thread-safe and must not log
/// through `log` itself (that call would not be forwarded).
pub type Handler = Arc<dyn Fn(log::Level, &str, &str) + Send + Sync>;

static HANDLER: RwLock<Option<Handler>> = RwLock::new(None);

thread_local! {
    /// A forward in progress on this thread: a record its handler logs is
    /// not forwarded again (no loop through the host).
    static FORWARDING: Cell<bool> = const { Cell::new(false) };
}

/// Sets (`None`: clears) the host's handler, for every record from now on.
pub fn set_handler(handler: Option<Handler>) {
    *HANDLER.write().unwrap_or_else(|e| e.into_inner()) = handler;
}

/// Whether a record at `level` goes to the host: warnings and errors.
pub fn forwards(level: log::Level) -> bool {
    level <= log::Level::Warn
}

/// Hands a record to `handler` if it [`forwards`]: what the logger does
/// with the host's handler.
pub fn forward_to(
    handler: &dyn Fn(log::Level, &str, &str),
    level: log::Level,
    target: &str,
    message: &str,
) {
    if !forwards(level) || FORWARDING.with(Cell::get) {
        return;
    }
    FORWARDING.with(|f| f.set(true));
    handler(level, target, message);
    FORWARDING.with(|f| f.set(false));
}

/// Hands a record to the handler set, if one is.
pub fn forward(level: log::Level, target: &str, message: &str) {
    let handler = HANDLER.read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(handler) = handler {
        forward_to(&*handler, level, target, message);
    }
}

/// `text` as a C string: a NUL in it (a message quoting what a host sent)
/// is dropped instead of losing the whole line.
pub fn c_text(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

/// The process's logger: stderr for warnings and everything of ours, the
/// host's handler for warnings and errors.
struct HostLog;

impl log::Log for HostLog {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Warn || m.target().starts_with("moshpit")
    }

    fn log(&self, r: &log::Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let message = r.args().to_string();
        eprintln!("[{}] {}: {}", r.target(), r.level(), message);
        forward(r.level(), r.target(), &message);
    }

    fn flush(&self) {}
}

static LOGGER: HostLog = HostLog;

/// Installs the logger, once per process (another one installed first
/// stays: then nothing of this module runs).
pub fn install() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    type Heard = Arc<Mutex<Vec<(log::Level, String, String)>>>;

    fn hearing() -> (Heard, impl Fn(log::Level, &str, &str)) {
        let heard: Heard = Arc::default();
        let into = heard.clone();
        (
            heard,
            move |level: log::Level, target: &str, message: &str| {
                into.lock()
                    .unwrap()
                    .push((level, target.to_owned(), message.to_owned()))
            },
        )
    }

    #[test]
    fn warnings_and_errors_reach_the_host_and_nothing_quieter() {
        let (heard, handler) = hearing();
        forward_to(&handler, log::Level::Error, "js", "HomeScreen failed: boom");
        forward_to(
            &handler,
            log::Level::Warn,
            "moshpit",
            "home: no channel to hear the host on",
        );
        forward_to(&handler, log::Level::Info, "moshpit", "ui surface 3 made");
        forward_to(&handler, log::Level::Debug, "moshpit", "detail");
        assert_eq!(
            *heard.lock().unwrap(),
            vec![
                (
                    log::Level::Error,
                    "js".to_owned(),
                    "HomeScreen failed: boom".to_owned()
                ),
                (
                    log::Level::Warn,
                    "moshpit".to_owned(),
                    "home: no channel to hear the host on".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn what_the_handler_logs_is_not_forwarded_again() {
        let heard: Heard = Arc::default();
        let into = heard.clone();
        let handler = move |level: log::Level, target: &str, message: &str| {
            into.lock()
                .unwrap()
                .push((level, target.to_owned(), message.to_owned()));
            // The host logging back through Rust, say: no second forward.
            forward_to(
                &|_: log::Level, _: &str, _: &str| panic!("forwarded twice"),
                level,
                target,
                "again",
            );
        };
        forward_to(&handler, log::Level::Error, "moshpit", "once");
        assert_eq!(heard.lock().unwrap().len(), 1);
        // And the next record is forwarded as usual.
        forward_to(&handler, log::Level::Error, "moshpit", "twice");
        assert_eq!(heard.lock().unwrap().len(), 2);
    }

    #[test]
    fn the_handler_set_hears_records_until_it_is_cleared() {
        // Other tests of this binary may log meanwhile: only this target counts.
        const TARGET: &str = "moshpit::host_log::the_handler_set";
        let (heard, handler) = hearing();
        set_handler(Some(Arc::new(handler)));
        forward(log::Level::Error, TARGET, "settings: post refused");
        forward(log::Level::Info, TARGET, "not this");
        set_handler(None);
        forward(log::Level::Error, TARGET, "nor this");
        let heard: Vec<_> = heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t, _)| t == TARGET)
            .cloned()
            .collect();
        assert_eq!(
            heard,
            vec![(
                log::Level::Error,
                TARGET.to_owned(),
                "settings: post refused".to_owned()
            )]
        );
    }

    #[test]
    fn a_nul_in_a_message_is_dropped_not_the_line() {
        assert_eq!(c_text("bad\0op").to_str().unwrap(), "badop");
        assert_eq!(c_text("plain").to_str().unwrap(), "plain");
    }
}
