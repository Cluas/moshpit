//! A small ring of plain-text lines the notification service leaves where
//! the app can read them (`PushDiagnostics`): the app's diagnostics screen
//! cannot see another process's log, and the two lines that matter most —
//! "opened a … push from …" and "no key opened this envelope (N available)"
//! — happen only in the service.
//!
//! The file (`push-diagnostics.json`) is a JSON array of `{"at", "text"}`
//! with ISO 8601 dates, at most [`CAPACITY`] lines, oldest dropped first.
//! Silent on failure by design: a diagnostic that could derail a
//! notification is worse than a missing line.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::foundation::date::{self, Date};
use crate::foundation::json::{self, Style};

/// Enough for a few pushes and their retries.
pub const CAPACITY: usize = 60;

/// What the diagnostics screen labels these with.
pub const SOURCE: &str = "push·extension";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    #[serde(with = "date::iso8601")]
    pub at: Date,
    pub text: String,
}

/// The ring's file, at a path of the platform's choosing.
#[derive(Debug, Clone)]
pub struct DiagnosticsFile {
    path: PathBuf,
}

impl DiagnosticsFile {
    /// The file's name in the iOS App Group container.
    pub const FILE_NAME: &'static str = "push-diagnostics.json";

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one line, oldest dropped past [`CAPACITY`]. A concurrent write
    /// from another process can lose a line; accepted, as in the app.
    pub fn record(&self, text: &str, now: Date) {
        let mut lines = self.read();
        lines.push(Line {
            at: now,
            text: text.to_owned(),
        });
        if lines.len() > CAPACITY {
            lines.drain(..lines.len() - CAPACITY);
        }
        let data = json::write_encoder(&json::to_value(&lines), Style::COMPACT);
        let _ = crate::store::atomic_write(&self.path, data.as_bytes());
    }

    /// Everything recorded, oldest first; nothing when the file is absent or
    /// does not decode as a whole.
    pub fn read(&self) -> Vec<Line> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|data| json::decode(&data).ok())
            .unwrap_or_default()
    }

    /// Lines at or after `since`.
    pub fn recent(&self, since: Date) -> Vec<Line> {
        self.read()
            .into_iter()
            .filter(|line| line.at >= since)
            .collect()
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::TempDir;

    // MARK: - PushDiagnosticsTests

    #[test]
    fn the_services_lines_survive_into_a_place_the_app_can_read() {
        let dir = TempDir::new("diag");
        let ring = DiagnosticsFile::new(dir.path(DiagnosticsFile::FILE_NAME));
        ring.record("woke for a push", Date::now());
        ring.record("opened a done push from m1-pro", Date::now());
        let lines = ring.read();
        assert_eq!(lines.len(), 2);
        assert!(lines.last().unwrap().text.contains("m1-pro"));
    }

    #[test]
    fn the_ring_is_bounded() {
        let dir = TempDir::new("diag");
        let ring = DiagnosticsFile::new(dir.path(DiagnosticsFile::FILE_NAME));
        for i in 0..CAPACITY + 20 {
            ring.record(&format!("line {i}"), Date::now());
        }
        let lines = ring.read();
        assert_eq!(lines.len(), CAPACITY);
        assert_eq!(
            lines.last().unwrap().text,
            format!("line {}", CAPACITY + 19)
        );
    }

    #[test]
    fn only_lines_inside_the_screens_window_are_offered() {
        let dir = TempDir::new("diag");
        let ring = DiagnosticsFile::new(dir.path(DiagnosticsFile::FILE_NAME));
        ring.record("ancient", Date::now().adding(-3600.0));
        ring.record("recent", Date::now());
        let recent = ring.recent(Date::now().adding(-600.0));
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].text, "recent");
        ring.clear();
        assert!(ring.read().is_empty());
    }

    #[test]
    fn the_file_is_what_the_app_writes() {
        let dir = TempDir::new("diag");
        let ring = DiagnosticsFile::new(dir.path(DiagnosticsFile::FILE_NAME));
        ring.record("a/b", Date::from_unix(1_762_800_000.75));
        assert_eq!(
            std::fs::read_to_string(ring.path()).unwrap(),
            r#"[{"at":"2025-11-10T18:40:00Z","text":"a\/b"}]"#
        );
    }
}
