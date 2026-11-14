//! Attaching images to an agent's prompt, the host's model: the flow from
//! images picked to the text pasted at the prompt, and the session's
//! numbered upload log. One copy for iOS and Android; the orchestration
//! was `ImageAttachmentController` in `Moshpit/Services/ImageAttachment.swift`.
//!
//! The host does the platform's part: decoding, resizing and re-encoding
//! each picture (so no location leaves the phone), the thumbnails, the
//! clipboard, and the uploads themselves over the session's connection. It
//! tells this model what happened ([`Attachments::processed`],
//! [`Attachments::progress`], [`Attachments::landed`],
//! [`Attachments::failed`]) and asks it what to upload next and what to
//! paste.
//!
//! The rules:
//!
//! - Uploads run one after another, so progress reads honestly and a
//!   failure names one file; progress is weighted by bytes and never goes
//!   back (the hops that report it can land out of order).
//! - A retry uploads only what has not landed, without processing again.
//! - Nothing is pasted until every file landed.
//! - The paste follows the pane's agent ([`InsertStyle::for_agent`]).

use moshpit_install::commands::quote;

/// How the active pane's agent best receives an image path (the 2026-08
/// cross-agent survey):
///
/// - `BarePath`: Claude Code reads it with its Read tool, Codex's TUI
///   attaches a pasted image path natively, Goose has `read_image`. The
///   default, also for plain shells.
/// - `AtMention`: Gemini CLI and Qwen Code read files deterministically
///   only through `@path`.
/// - `AiderAdd`: aider attaches images only through `/add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InsertStyle {
    #[default]
    BarePath,
    AtMention,
    AiderAdd,
}

impl InsertStyle {
    /// From the pane's agent: the hook-stamped `@moshpit_agent`, else the
    /// pane's foreground command. Unknown or none: the default. Claude Code
    /// names its process by its version, so a bare semver is Claude.
    pub fn for_agent(agent: Option<&str>) -> Self {
        let Some(name) = agent.map(str::to_lowercase).filter(|n| !n.is_empty()) else {
            return InsertStyle::BarePath;
        };
        if is_semver(&name) {
            return InsertStyle::BarePath;
        }
        if name.contains("gemini") || name.contains("qwen") {
            InsertStyle::AtMention
        } else if name.contains("aider") {
            InsertStyle::AiderAdd
        } else {
            InsertStyle::BarePath
        }
    }
}

/// `^\d+\.\d+\.\d+$`, digits being ASCII.
fn is_semver(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The text pasted at the prompt for uploaded `paths`.
///
/// A bare path padded with spaces reads the same to every agent; Codex's
/// image-path detector attaches it, which quotes would break, so a path is
/// quoted only when it carries a space or a quote (the app's own file names
/// never do); and an `@` typed blind would open Claude Code's completion
/// menu. An `@`-parser reads to white space and does not strip quotes, so
/// a path that needs quoting is never `@`-mentioned: it goes bare, quoted.
/// aider's `/add` must lead the message: no padding.
pub fn insert_text(paths: &[String], style: InsertStyle) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let quoted = |p: &String| {
        if needs_quoting(p) {
            quote(p)
        } else {
            p.clone()
        }
    };
    match style {
        InsertStyle::BarePath => {
            format!(
                " {} ",
                paths.iter().map(quoted).collect::<Vec<_>>().join(" ")
            )
        }
        InsertStyle::AtMention => {
            let rendered: Vec<String> = paths
                .iter()
                .map(|p| {
                    if needs_quoting(p) {
                        quoted(p)
                    } else {
                        format!("@{p}")
                    }
                })
                .collect();
            format!(" {} ", rendered.join(" "))
        }
        InsertStyle::AiderAdd => {
            format!(
                "/add {}",
                paths.iter().map(quoted).collect::<Vec<_>>().join(" ")
            )
        }
    }
}

/// By scalar: a quote or a space with a combining mark on it is still one.
fn needs_quoting(path: &str) -> bool {
    path.contains([' ', '\'', '"'])
}

/// The session's numbered record of every image it uploaded, so a picture
/// can be referred to again (#1, #2, …) without uploading it again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UploadLog {
    paths: Vec<String>,
}

impl UploadLog {
    /// Records `path`; its number, from 1, stable once given.
    pub fn record(&mut self, path: &str) -> usize {
        self.paths.push(path.to_owned());
        self.paths.len()
    }

    /// Every path, in the order recorded (number = index + 1).
    pub fn paths(&self) -> &[String] {
        &self.paths
    }
}

/// Where the flow is.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Decoding and re-encoding what was picked.
    Processing,
    /// The overall fraction across every file, weighted by bytes.
    Uploading(f64),
    /// Every file landed: the paste is ready.
    Ready,
    /// Why it stopped (the host's words).
    Failed(String),
}

/// One picker-to-paste flow: the processed files' sizes, where each landed,
/// and the phase.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachments {
    phase: Phase,
    sizes: Vec<u64>,
    remote: Vec<Option<String>>,
    numbers: Vec<Option<usize>>,
}

impl Default for Attachments {
    fn default() -> Self {
        Self::new()
    }
}

impl Attachments {
    pub fn new() -> Self {
        Attachments {
            phase: Phase::Processing,
            sizes: Vec::new(),
            remote: Vec::new(),
            numbers: Vec::new(),
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Where each file landed, in the order picked.
    pub fn remote_paths(&self) -> &[Option<String>] {
        &self.remote
    }

    /// Each file's number in the session's log, once every file landed.
    pub fn numbers(&self) -> &[Option<usize>] {
        &self.numbers
    }

    /// The picked images are processed, these sizes in bytes; uploading
    /// starts.
    /// An empty batch is ready at once, with nothing to paste: only a
    /// file landing makes the flow ready otherwise, and none will.
    pub fn processed(&mut self, sizes: Vec<u64>) {
        self.remote = vec![None; sizes.len()];
        self.numbers = vec![None; sizes.len()];
        self.phase = if sizes.is_empty() {
            Phase::Ready
        } else {
            Phase::Uploading(0.0)
        };
        self.sizes = sizes;
    }

    fn total(&self) -> u64 {
        self.sizes.iter().sum::<u64>().max(1)
    }

    fn landed_bytes(&self) -> u64 {
        self.sizes
            .iter()
            .zip(&self.remote)
            .filter(|(_, r)| r.is_some())
            .map(|(s, _)| s)
            .sum()
    }

    /// The next file to upload (its index), or none when every one landed
    /// (the flow is ready then) or it is not uploading.
    #[allow(clippy::should_implement_trait)] // it moves the phase on: no iterator
    pub fn next(&mut self) -> Option<usize> {
        if !matches!(self.phase, Phase::Uploading(_)) {
            return None;
        }
        let next = self.remote.iter().position(Option::is_none);
        let fraction = self.landed_bytes() as f64 / self.total() as f64;
        self.raise(fraction);
        next
    }

    /// File `index` is `fraction` of the way up: the overall fraction moves
    /// on, never back. Returns the overall fraction shown now.
    pub fn progress(&mut self, index: usize, fraction: f64) -> Option<f64> {
        let size = *self.sizes.get(index)? as f64;
        let done_before: u64 = self
            .sizes
            .iter()
            .zip(&self.remote)
            .enumerate()
            .filter(|(i, (_, r))| *i != index && r.is_some())
            .map(|(_, (s, _))| s)
            .sum();
        let overall = (done_before as f64 + fraction.clamp(0.0, 1.0) * size) / self.total() as f64;
        self.raise(overall);
        match self.phase {
            Phase::Uploading(shown) => Some(shown),
            _ => None,
        }
    }

    fn raise(&mut self, fraction: f64) {
        if let Phase::Uploading(shown) = self.phase
            && fraction > shown
        {
            self.phase = Phase::Uploading(fraction.min(1.0));
        }
    }

    /// File `index` landed at `path`. True when it was the last: the flow
    /// is ready, and the host numbers the files in its session's log
    /// ([`Attachments::numbered`]).
    pub fn landed(&mut self, index: usize, path: &str) -> bool {
        let Some(slot) = self.remote.get_mut(index) else {
            return false;
        };
        *slot = Some(path.to_owned());
        let fraction = self.landed_bytes() as f64 / self.total() as f64;
        self.raise(fraction);
        if self.remote.iter().all(Option::is_some) && matches!(self.phase, Phase::Uploading(_)) {
            self.phase = Phase::Ready;
            return true;
        }
        false
    }

    /// The files' numbers in the session's log, in the order picked; taken
    /// only when there is one for each file.
    pub fn numbered(&mut self, numbers: &[usize]) {
        if numbers.len() == self.remote.len() {
            self.numbers = numbers.iter().map(|n| Some(*n)).collect();
        }
    }

    /// Records every landed file in `log`, in the order picked, and takes
    /// the numbers it gives (a host whose log is this model's).
    pub fn number_in(&mut self, log: &mut UploadLog) {
        let numbers: Vec<usize> = self
            .remote
            .iter()
            .flatten()
            .map(|p| log.record(p))
            .collect();
        self.numbered(&numbers);
    }

    /// It stopped: why, in the host's words. A cancel is not a failure (the
    /// host says nothing then).
    pub fn failed(&mut self, why: &str) {
        self.phase = Phase::Failed(why.to_owned());
    }

    /// Try again after a failure: only what has not landed is uploaded, and
    /// nothing is processed again. False when it had not failed, or had
    /// failed before anything was processed (the host processes again).
    pub fn retry(&mut self) -> bool {
        if !matches!(self.phase, Phase::Failed(_)) || self.sizes.is_empty() {
            return false;
        }
        self.phase = Phase::Uploading(0.0);
        true
    }

    /// The paste, in `style`; none until every file landed.
    pub fn insert_text(&self, style: InsertStyle) -> Option<String> {
        if self.phase != Phase::Ready {
            return None;
        }
        let paths: Vec<String> = self.remote.iter().flatten().cloned().collect();
        (!paths.is_empty()).then(|| insert_text(&paths, style))
    }
}

#[cfg(test)]
mod tests;
