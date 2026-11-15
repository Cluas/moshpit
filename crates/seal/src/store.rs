//! What the App Group files share: reading a file that may be absent or
//! unreadable, writing it atomically, keeping a copy of one this build could
//! not fully read before writing over it, and the lock a read-modify-write
//! takes across processes (the finishes and the standing set: the app and
//! the notification service both write them).
//!
//! The platform parts of the iOS stores are not here: the App Group path,
//! the `.completeFileProtectionUntilFirstUserAuthentication` class and the
//! backup exclusion. A platform hands in a path in its own private storage
//! and arranges protection and backup rules there.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// One read of a store's file.
#[derive(Debug)]
pub(crate) enum Read {
    /// No file: nothing stored yet.
    Absent,
    /// The file's bytes.
    Bytes(Vec<u8>),
    /// The file is there but could not be read (before the first unlock, an
    /// I/O error, a directory in its place). Nothing is known about it.
    Unreadable,
}

pub(crate) fn read(path: &Path) -> Read {
    match fs::read(path) {
        Ok(bytes) => Read::Bytes(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling symlink reads as "not found" but is something on
            // disk; Foundation's fileExists(atPath:) says no to it too.
            Read::Absent
        }
        Err(_) => Read::Unreadable,
    }
}

/// An exclusive `flock` on `<file>.lock`, held until dropped: one
/// read-modify-write of a file at a time, across processes (the app and the
/// notification service) and across opens within one. None (the lock file
/// can't be opened) lets the write go ahead unguarded, as before it existed.
pub(crate) struct Lock(Option<File>);

impl Lock {
    pub(crate) fn take(path: &Path) -> Self {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(with_extension(path, "lock"))
            .ok();
        #[cfg(unix)]
        if let Some(file) = &file {
            use std::os::fd::AsRawFd;
            // SAFETY: a valid descriptor for the file's lifetime.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        }
        Self(file)
    }
}

impl Lock {
    /// [`Lock::take`] that waits `within` at most (B636): `None` when another
    /// holder kept it that long — the caller goes on without what the file
    /// says, never later than its budget. A lock file that can't be opened
    /// lets it go ahead unguarded, as `take` does.
    pub(crate) fn try_take(path: &Path, within: std::time::Duration) -> Option<Self> {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(file) = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(with_extension(path, "lock"))
        else {
            return Some(Self(None));
        };
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let deadline = std::time::Instant::now() + within;
            loop {
                // SAFETY: a valid descriptor for the file's lifetime.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        Some(Self(Some(file)))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(file) = &self.0 {
            use std::os::fd::AsRawFd;
            // SAFETY: as in `take`.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

/// `path` with `.<extension>` appended (`appendingPathExtension`).
pub(crate) fn with_extension(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Write `data` to `path` atomically: a temporary file beside it, then a
/// rename, as `Data.write(to:options: .atomic)` does. The directory is made
/// first, as the Swift stores do.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("no file name"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        crate::random::hex(4)
    ));
    let tmp = dir.join(tmp_name);
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Keep an untouched copy of a file this build could not fully read, and say
/// whether a write over it is now safe (`PushPairingFile.secure`).
///
/// A file whose top level this build understood, with only some entries
/// skipped, carries those entries through every save, so its first copy is
/// all it needs; a file that is no list (no map) at all is the only copy of
/// whatever it holds, so each distinct one is kept, under
/// `<file>.backup-<unix seconds>` once `<file>.backup` is taken.
pub(crate) fn secure(path: &Path, raw: &[u8], top_level_understood: bool, now_unix: i64) -> bool {
    let backup = with_extension(path, "backup");
    if fs::read(&backup).is_ok_and(|existing| existing == raw) {
        return true;
    }
    let mut target = backup.clone();
    if backup.exists() {
        if top_level_understood {
            return true;
        }
        target = with_extension(path, &format!("backup-{now_unix}"));
    }
    atomic_write(&target, raw).is_ok() && fs::read(&target).is_ok_and(|copy| copy == raw)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;

    /// A fresh directory under the system temp dir, removed on drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("moshpit-seal-{label}-{}", crate::random::hex(8)));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Make `path` unreadable for the test's duration, as a store looks
    /// before the first unlock. `None` when the filesystem still lets the
    /// owner read it (running as root).
    #[cfg(unix)]
    pub struct Unreadable(PathBuf);

    #[cfg(unix)]
    impl Unreadable {
        pub fn new(path: &std::path::Path) -> Option<Self> {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
            let guard = Self(path.to_owned());
            std::fs::read(path).is_err().then_some(guard)
        }
    }

    #[cfg(unix)]
    impl Drop for Unreadable {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644));
        }
    }
}
