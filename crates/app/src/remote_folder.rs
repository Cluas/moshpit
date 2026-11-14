//! One folder on the host as the folder sheet shows it, the host's model:
//! its listing, what is being fetched and uploaded, and the failures the
//! sheet says. One copy for iOS and Android; it was `RemoteFolderModel` in
//! `Moshpit/UI/Terminal/RemotePreview.swift`.
//!
//! The host does the I/O and the platform's part — listing, fetching,
//! uploading and changing files over the session's connection, the
//! thumbnails, QuickLook, the share sheet, the photo and file pickers — and
//! tells this model what happened. The model decides the rest:
//!
//! - What the folder shows ([`crate::remote_paths::browsable`]): images and
//!   video as the grid, then folders, then the other files.
//! - Which thumbnails to fetch: video (a frame, streamed) and images up to
//!   [`THUMBNAIL_LIMIT`], each once.
//! - A name typed for a new folder or a rename is trimmed and checked, and
//!   one already in the folder (hidden ones included) is refused before the
//!   round trip; an upload never overwrites (Finder's " 2").
//! - Progress for a fetch or an upload is taken only while it is under way
//!   (a late report from a finished one is dropped).
//! - Each failure set anew gets a number never used before in this process,
//!   and the sheet says each number once (B286): the same words failing
//!   again are said again, even when the cleared state between was never
//!   drawn; the same failure set again while it stands keeps its number.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::remote_paths::{self, Entry, Kind};

/// Images are fetched for a thumbnail up to this size; bigger ones show
/// their icon until opened.
pub const THUMBNAIL_LIMIT: u64 = 15 * 1024 * 1024;

/// How many thumbnails are fetched at once.
pub const THUMBNAIL_WORKERS: usize = 3;

/// The last failure number given, in this process.
static FAILURES_NUMBERED: AtomicU64 = AtomicU64::new(0);

fn next_failure_id() -> u64 {
    FAILURES_NUMBERED.fetch_add(1, Ordering::Relaxed) + 1
}

/// A failure the sheet says: its words (the host's, in the app's
/// language) and its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub title: String,
    pub message: String,
    pub id: u64,
}

/// Why a typed name was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRefusal {
    /// It can't name one item (empty, `.`, `..`, a `/` or a NUL), or a
    /// rename to the name it has: nothing to do, nothing said.
    Ignored,
    /// Something with that name is already there.
    Taken,
}

/// The folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    /// The folder's path on the host.
    pub path: String,
    entries: Vec<Entry>,
    /// Every name in the folder, hidden and unpreviewable ones included:
    /// what a new name must not collide with.
    names: BTreeSet<String>,
    loading: bool,
    failure: Option<String>,
    thumbnails: BTreeSet<String>,
    durations: BTreeMap<String, f64>,
    opening: BTreeMap<String, f64>,
    uploading: Option<f64>,
    open_failure: Option<Failure>,
    edit_failure: Option<Failure>,
}

impl Folder {
    /// The folder at `path`, before its listing.
    pub fn new(path: &str) -> Self {
        Folder {
            path: path.to_owned(),
            entries: Vec::new(),
            names: BTreeSet::new(),
            loading: true,
            failure: None,
            thumbnails: BTreeSet::new(),
            durations: BTreeMap::new(),
            opening: BTreeMap::new(),
            uploading: None,
            open_failure: None,
            edit_failure: None,
        }
    }

    // MARK: Listing

    /// A listing is on its way.
    pub fn load_started(&mut self) {
        self.loading = true;
        self.failure = None;
    }

    /// The host listed the folder: everything in it, as it said.
    pub fn listed(&mut self, listing: &[Entry]) {
        self.names = listing.iter().map(|e| e.name().to_owned()).collect();
        self.entries = remote_paths::browsable(listing);
        self.loading = false;
    }

    /// The listing failed, in the host's words.
    pub fn list_failed(&mut self, why: &str) {
        self.failure = Some(why.to_owned());
        self.loading = false;
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// What the folder shows, newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entry at `path`, if the folder shows it.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// The thumbnail grid: images and video.
    pub fn media(&self) -> Vec<Entry> {
        remote_paths::media(&self.entries)
    }

    /// The list under the grid: folders, then every other file.
    pub fn rows(&self) -> Vec<Entry> {
        remote_paths::rows(&self.entries)
    }

    // MARK: Thumbnails

    /// What still wants a thumbnail, in order: video (a frame and its
    /// running time, streamed) and images up to [`THUMBNAIL_LIMIT`] (their
    /// size unknown counts as small).
    pub fn thumbnail_queue(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|e| !self.thumbnails.contains(&e.path) && !self.durations.contains_key(&e.path))
            .filter(|e| match e.kind() {
                Some(Kind::Video) => true,
                Some(Kind::Image) => e.size.unwrap_or(0) <= THUMBNAIL_LIMIT,
                _ => false,
            })
            .cloned()
            .collect()
    }

    /// `path`'s thumbnail is in.
    pub fn thumbnail_arrived(&mut self, path: &str) {
        self.thumbnails.insert(path.to_owned());
    }

    /// A video's cover is in, and its running time.
    pub fn cover_arrived(&mut self, path: &str, seconds: f64) {
        self.thumbnails.insert(path.to_owned());
        self.durations.insert(path.to_owned(), seconds);
    }

    pub fn has_thumbnail(&self, path: &str) -> bool {
        self.thumbnails.contains(path)
    }

    pub fn duration(&self, path: &str) -> Option<f64> {
        self.durations.get(path).copied()
    }

    // MARK: Opening a file

    /// The user opened `path`: it is on its way.
    pub fn open_started(&mut self, path: &str) {
        self.opening.insert(path.to_owned(), 0.0);
    }

    /// `path` is `fraction` fetched; dropped once it is no longer on its
    /// way.
    pub fn open_progress(&mut self, path: &str, fraction: f64) {
        if let Some(shown) = self.opening.get_mut(path) {
            *shown = fraction.clamp(0.0, 1.0);
        }
    }

    /// `path` is no longer on its way (fetched, failed or streamed).
    pub fn open_finished(&mut self, path: &str) {
        self.opening.remove(path);
    }

    /// How far `path`'s fetch is, while it is on its way.
    pub fn opening(&self, path: &str) -> Option<f64> {
        self.opening.get(path).copied()
    }

    // MARK: Changing the folder

    /// The path a new folder named `raw` goes to.
    pub fn new_folder_target(&self, raw: &str) -> Result<String, NameRefusal> {
        let name = remote_paths::valid_name(raw).ok_or(NameRefusal::Ignored)?;
        if remote_paths::is_taken(&name, &self.names) {
            return Err(NameRefusal::Taken);
        }
        Ok(remote_paths::child(&name, &self.path))
    }

    /// The path `entry` is renamed to, named `raw`.
    pub fn rename_target(&self, entry: &Entry, raw: &str) -> Result<String, NameRefusal> {
        let name = remote_paths::valid_name(raw).ok_or(NameRefusal::Ignored)?;
        if name == entry.name() {
            return Err(NameRefusal::Ignored);
        }
        // Another item's name, in either Unicode form (B332); the file's own
        // name in another form is the file meeting itself.
        if self
            .names
            .iter()
            .any(|taken| taken != entry.name() && remote_paths::same_name(taken, &name))
        {
            return Err(NameRefusal::Taken);
        }
        Ok(remote_paths::child(&name, &self.path))
    }

    /// Where an upload named `name` goes: its name, or the first one not
    /// taken, which is taken from now on.
    pub fn upload_target(&mut self, name: &str) -> String {
        let name = remote_paths::unique_name(name, &self.names);
        self.names.insert(name.clone());
        remote_paths::child(&name, &self.path)
    }

    /// An upload started.
    pub fn upload_started(&mut self) {
        self.uploading = Some(0.0);
    }

    /// The upload is `fraction` done; dropped once it is no longer under way.
    pub fn upload_progress(&mut self, fraction: f64) {
        if self.uploading.is_some() {
            self.uploading = Some(fraction.clamp(0.0, 1.0));
        }
    }

    pub fn upload_finished(&mut self) {
        self.uploading = None;
    }

    /// An upload under way, as a fraction.
    pub fn uploading(&self) -> Option<f64> {
        self.uploading
    }

    // MARK: Failures

    /// Why the file just tapped couldn't be opened (none: cleared).
    pub fn set_open_failure(&mut self, message: Option<&str>) {
        self.open_failure = numbered(self.open_failure.take(), message.map(|m| ("", m)));
    }

    /// A change that didn't go through: its title and why (none: cleared).
    pub fn set_edit_failure(&mut self, failure: Option<(&str, &str)>) {
        self.edit_failure = numbered(self.edit_failure.take(), failure);
    }

    pub fn open_failure(&self) -> Option<&Failure> {
        self.open_failure.as_ref()
    }

    pub fn edit_failure(&self) -> Option<&Failure> {
        self.edit_failure.as_ref()
    }
}

/// `new`, numbered: the number `old` had when it says the same, else a new
/// one.
fn numbered(old: Option<Failure>, new: Option<(&str, &str)>) -> Option<Failure> {
    let (title, message) = new?;
    if let Some(old) = old
        && old.title == title
        && old.message == message
    {
        return Some(old);
    }
    Some(Failure {
        title: title.to_owned(),
        message: message.to_owned(),
        id: next_failure_id(),
    })
}

#[cfg(test)]
mod tests;
