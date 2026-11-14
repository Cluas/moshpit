//! The app's log: every line of ours to the platform's log (logcat on
//! Android, under `MoshpitCore`) and the last half hour of them kept for
//! Settings › Recent Log (`DiagnosticsLog`), newest first.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moshpit_ui::recent_log::{Entry, LogState, Message};

/// How long a line is kept.
pub const KEPT: Duration = Duration::from_secs(30 * 60);
/// At most this many lines.
const MOST: usize = 2000;

struct Line {
    at: SystemTime,
    category: String,
    message: String,
}

static LINES: Mutex<VecDeque<Line>> = Mutex::new(VecDeque::new());

/// Keeps one line.
pub fn keep(category: &str, message: &str) {
    let mut lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
    let now = SystemTime::now();
    while lines
        .front()
        .is_some_and(|l| now.duration_since(l.at).unwrap_or_default() > KEPT)
        || lines.len() >= MOST
    {
        lines.pop_front();
    }
    lines.push_back(Line {
        at: now,
        category: category.to_owned(),
        message: message.to_owned(),
    });
}

/// `HH:mm:ss.SSS` (UTC: the device's zone is the host's to know).
fn clock(at: SystemTime) -> String {
    let ms = at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let day = ms % 86_400_000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        day / 3_600_000,
        day / 60_000 % 60,
        day / 1000 % 60,
        day % 1000
    )
}

/// `{"op": "log", "state": "loaded", …}`: what was kept, newest first.
pub fn message() -> String {
    let lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
    let entries = lines
        .iter()
        .rev()
        .map(|l| Entry {
            time: clock(l.at),
            category: l.category.clone(),
            message: l.message.clone(),
        })
        .collect();
    Message::Log(LogState::Loaded { entries }).to_json()
}

/// The `log` facade's logger: warnings from everyone, everything of ours.
pub struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Warn || m.target().starts_with("moshpit")
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        keep(record.target(), &message);
        platform_log(record.level(), record.target(), &message);
    }

    fn flush(&self) {}
}

#[cfg(target_os = "android")]
fn platform_log(level: log::Level, target: &str, message: &str) {
    crate::android::alog::write(level, target, message);
}

#[cfg(not(target_os = "android"))]
fn platform_log(level: log::Level, target: &str, message: &str) {
    eprintln!("[{target}] {level}: {message}");
}

/// Installs [`Logger`] (once; later calls change nothing).
pub fn init() {
    static LOGGER: Logger = Logger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_page_gets_the_newest_line_first() {
        keep("moshpit", "first");
        keep("moshpit", "second");
        let json: serde_json::Value = serde_json::from_str(&message()).unwrap();
        let entries = json["entries"].as_array().unwrap();
        let mine: Vec<_> = entries
            .iter()
            .filter_map(|e| e["message"].as_str())
            .filter(|m| *m == "first" || *m == "second")
            .collect();
        assert_eq!(mine, ["second", "first"]);
        assert_eq!(json["state"], "loaded");
    }

    #[test]
    fn the_clock_is_hours_minutes_seconds_and_milliseconds() {
        let at = UNIX_EPOCH + Duration::from_millis(3_723_004);
        assert_eq!(clock(at), "01:02:03.004");
    }
}
