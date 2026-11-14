//! Images a program put on the screen (kitty graphics, sixel, iTerm2 inline
//! images), decoded for drawing: the emulator keeps sixel and kitty images
//! as pixels and iTerm2 ones as the file the program sent; this cache
//! decodes each image once (its first frame, for an animation), keyed by
//! the emulator's hash of its data, within a byte budget (least recently
//! drawn out first).
//!
//! One cache for the process ([`ImageCache::shared`], 64 MB), every
//! terminal drawing from it: a budget per terminal let six tmux panes that
//! had each shown large images keep 64 MB apiece, what gets a terminal app
//! killed in the background (docs/nsh/TERMINAL-VIEW.md §5.8). It is a cache
//! only: [`ImageCache::purge`] lets it all go (a memory warning, the app
//! going to the background) and the next frame that draws an image decodes
//! it again.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use wezterm_term::image::{ImageData, ImageDataType};

/// A decoded image: RGBA, 8 bits a channel, row after row.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    /// The same for the same data, across frames and terminals.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Decoded images kept for drawing, within a byte budget, shared by every
/// terminal made with it (cheap to clone: one cache).
#[derive(Clone)]
pub struct ImageCache(Arc<Mutex<Store>>);

impl std::fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let store = self.lock();
        write!(f, "ImageCache({} of {} bytes)", store.used, store.budget)
    }
}

impl ImageCache {
    /// The process's budget.
    pub const SHARED_BUDGET: usize = 64 << 20;

    /// A cache of its own (tests, a host that wants one per window).
    pub fn new(budget: usize) -> Self {
        Self(Arc::new(Mutex::new(Store {
            budget,
            used: 0,
            tick: 0,
            entries: HashMap::new(),
        })))
    }

    /// The process's cache: what a terminal draws from unless it was made
    /// with its own (`Options::image_cache`).
    pub fn shared() -> Self {
        static SHARED: OnceLock<ImageCache> = OnceLock::new();
        SHARED
            .get_or_init(|| Self::new(Self::SHARED_BUDGET))
            .clone()
    }

    /// Bytes of decoded pixels kept.
    pub fn bytes(&self) -> usize {
        self.lock().used
    }

    /// Lets every decoded image go. The images stay on the screen (the
    /// emulator keeps them; a view keeps what it has drawn): the next
    /// frame that draws one decodes it again.
    pub fn purge(&self) {
        let mut store = self.lock();
        store.entries.clear();
        store.used = 0;
    }

    fn lock(&self) -> MutexGuard<'_, Store> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The decoded image of `data`; `None` if it does not decode (or its
    /// pixels alone are over the budget). Decoded outside the cache's
    /// lock: another terminal's frame does not wait for it.
    pub(crate) fn get(&self, data: &Arc<ImageData>) -> Option<DecodedImage> {
        let key = data.hash();
        let budget = {
            let mut store = self.lock();
            if let Some(found) = store.hit(&key) {
                return found;
            }
            store.budget
        };
        let decoded = decode(data, budget);
        self.lock().insert(key, decoded.clone());
        decoded
    }
}

struct Store {
    budget: usize,
    used: usize,
    tick: u64,
    entries: HashMap<[u8; 32], (Option<DecodedImage>, u64)>,
}

impl Store {
    fn hit(&mut self, key: &[u8; 32]) -> Option<Option<DecodedImage>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(img, used)| {
            *used = tick;
            img.clone()
        })
    }

    /// (Two terminals decoding the same image at once both insert it: the
    /// second replaces the first.)
    fn insert(&mut self, key: [u8; 32], decoded: Option<DecodedImage>) {
        self.tick += 1;
        if let Some((Some(old), _)) = self.entries.remove(&key) {
            self.used -= old.rgba.len();
        }
        self.used += decoded.as_ref().map_or(0, |d| d.rgba.len());
        self.entries.insert(key, (decoded, self.tick));
        self.evict(key);
    }

    /// Drops the least recently used images until the budget holds,
    /// never `keep` (the one just decoded).
    fn evict(&mut self, keep: [u8; 32]) {
        while self.used > self.budget {
            let Some((&oldest, _)) = self
                .entries
                .iter()
                .filter(|(k, _)| **k != keep)
                .min_by_key(|(_, (_, used))| *used)
            else {
                break;
            };
            if let Some((Some(img), _)) = self.entries.remove(&oldest) {
                self.used -= img.rgba.len();
            }
        }
    }
}

fn id_of(hash: &[u8; 32]) -> u64 {
    u64::from_le_bytes(hash[..8].try_into().unwrap())
}

fn decode(data: &Arc<ImageData>, budget: usize) -> Option<DecodedImage> {
    let id = id_of(&data.hash());
    let encoded = {
        let locked = data.data();
        match &*locked {
            ImageDataType::Rgba8 {
                data,
                width,
                height,
                ..
            } => {
                return checked(id, *width, *height, data.clone(), budget);
            }
            ImageDataType::AnimRgba8 {
                width,
                height,
                frames,
                ..
            } => {
                return checked(id, *width, *height, frames.first()?.clone(), budget);
            }
            ImageDataType::EncodedFile(bytes) => bytes.clone(),
            ImageDataType::EncodedLease(_) => return None,
        }
    };
    // Decoded outside the emulator's lock, from a copy.
    match ImageDataType::EncodedFile(encoded).decode() {
        ImageDataType::Rgba8 {
            data,
            width,
            height,
            ..
        } => checked(id, width, height, data, budget),
        ImageDataType::AnimRgba8 {
            width,
            height,
            mut frames,
            ..
        } if !frames.is_empty() => checked(id, width, height, frames.swap_remove(0), budget),
        _ => None,
    }
}

fn checked(id: u64, width: u32, height: u32, rgba: Vec<u8>, budget: usize) -> Option<DecodedImage> {
    if rgba.len() != width as usize * height as usize * 4 || rgba.len() > budget {
        return None;
    }
    Some(DecodedImage {
        id,
        width,
        height,
        rgba: Arc::new(rgba),
    })
}
