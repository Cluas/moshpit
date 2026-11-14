//! SFTP: what remote file previews, uploads and the folder sheet's edits
//! need (`SSHSession`'s SFTP half). Requests are pipelined: a folder of
//! thumbnails or a 16-deep read window costs round trips in parallel, not in
//! a row. Directory handles are closed after listing — Citadel never closed
//! them, so every listing on the long-lived preview session left one open
//! in the host's sftp-server.
//!
//! The exec fallbacks for a host without SFTP (`RemoteFileCommands`,
//! `RemoteFileEdits`) belong to the remote-paths port; [`SftpError::Unavailable`]
//! and any other failure are the cue to use them, as before.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use russh::ChannelMsg;
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as RawError;
use russh_sftp::protocol::{
    Attrs, Data, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::connection::Ssh;
use crate::error::{SftpError, SshError};

/// Reads in flight while downloading, and their size: the app's window.
const READ_WINDOW: usize = 16;
const READ_CHUNK: u32 = 64 * 1024;
/// Writes in flight while uploading, and their size (the app's chunk).
const WRITE_WINDOW: usize = 8;
const WRITE_CHUNK: usize = 128 * 1024;
/// A request unanswered this long fails rather than hangs (B89's lesson).
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// `S_IFMT`, `S_IFDIR` and `S_IFLNK`.
const FILE_TYPE_MASK: u32 = 0o170000;
const DIRECTORY: u32 = 0o040000;
const SYMLINK: u32 = 0o120000;

/// One file or folder on the host (`RemoteFileEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub path: String,
    /// A symlink reads as what it points at (B460); one pointing nowhere as
    /// the link itself.
    pub is_directory: bool,
    pub size: Option<u64>,
    /// Seconds since 1970.
    pub modified: Option<u64>,
}

impl RemoteEntry {
    fn new(path: String, attrs: &FileAttributes) -> Self {
        Self {
            path,
            is_directory: attrs
                .permissions
                .is_some_and(|mode| mode & FILE_TYPE_MASK == DIRECTORY),
            size: attrs.size,
            modified: attrs.mtime.map(u64::from),
        }
    }
}

/// An SFTP session on a connection. Cheap to clone.
#[derive(Clone)]
pub struct Sftp {
    raw: Arc<Raw>,
    host: String,
    alive: Arc<AtomicBool>,
    /// What this handle's operations give up on ([`Self::cancelled_by`]).
    cancel: Option<CancellationToken>,
}

impl Sftp {
    /// Opens the `sftp` subsystem on a new channel.
    pub async fn start(ssh: &Ssh) -> Result<Sftp, SftpError> {
        ssh.answered("start SFTP", async { Ok(Self::begin(ssh).await) })
            .await?
    }

    async fn begin(ssh: &Ssh) -> Result<Sftp, SftpError> {
        let mut channel = ssh.open_session().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(SshError::Protocol)?;
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) => {
                    return Err(SftpError::Unavailable(
                        "the host refused the sftp subsystem".to_owned(),
                    ));
                }
                Some(ChannelMsg::Close | ChannelMsg::Eof) | None => {
                    return Err(SftpError::Unavailable(
                        "the channel closed before SFTP started".to_owned(),
                    ));
                }
                Some(_) => continue,
            }
        }
        let sftp = Sftp::over(channel.into_stream(), ssh.target().host.clone());
        sftp.raw
            .init()
            .await
            .map_err(|e| SftpError::Unavailable(format!("no SFTP greeting: {e}")))?;
        Ok(sftp)
    }

    /// A session over `stream` (the subsystem's channel), its greeting not
    /// yet asked for.
    fn over<S>(stream: S, host: String) -> Sftp
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Sftp {
            raw: Arc::new(Raw::over(stream)),
            host,
            alive: Arc::new(AtomicBool::new(true)),
            cancel: None,
        }
    }

    /// The same session, whose operations give up the moment `cancel` is
    /// cancelled (B243): a read-only request in flight (`home`, `stat`, a
    /// listing's pages, a transfer's window) is no longer waited for, and
    /// the operation answers [`SftpError::Cancelled`] at once; what it
    /// holds on the host is let go in the background — a handle closed, a
    /// half-made upload removed — so the answer never waits on a slow or
    /// silent link. An open in flight isn't waited for either: the handle
    /// the host makes for it is closed when it comes (and an exclusive
    /// upload's new file removed). A change of the folder (a new folder, a
    /// rename, a removal) is not cut in the middle: it is not sent once the
    /// operation is cancelled, and finishes once it is sent.
    pub fn cancelled_by(&self, cancel: CancellationToken) -> Sftp {
        Sftp {
            cancel: Some(cancel),
            ..self.clone()
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    /// `Cancelled` if the operation was given up before it began.
    fn not_cancelled(&self) -> Result<(), SftpError> {
        if self.is_cancelled() {
            Err(SftpError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// `request`'s answer, unless the operation is given up first.
    async fn unless_cancelled<T>(
        &self,
        request: impl std::future::Future<Output = Result<T, SftpError>>,
    ) -> Result<T, SftpError> {
        match &self.cancel {
            None => request.await,
            Some(cancel) => tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(SftpError::Cancelled),
                answer = request => answer,
            },
        }
    }

    /// A request that opens a handle on the host (`request` owns what it
    /// needs): its answer, unless the operation is given up first. Then it
    /// isn't waited for; the handle the host makes for it all the same is
    /// closed when it comes, and the file removed when `made` names one
    /// that only this request could have made (an exclusive create).
    async fn open_handle(
        &self,
        request: impl std::future::Future<Output = Result<String, RawError>> + Send + 'static,
        made: Option<String>,
    ) -> Result<Result<String, RawError>, SftpError> {
        let Some(cancel) = &self.cancel else {
            return Ok(request.await);
        };
        if cancel.is_cancelled() {
            return Err(SftpError::Cancelled);
        }
        let mut opening = tokio::spawn(request);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let raw = self.raw.clone();
                tokio::spawn(async move {
                    if let Ok(Ok(handle)) = opening.await {
                        let _ = raw.close(handle).await;
                        if let Some(made) = made {
                            let _ = raw.remove(made).await;
                        }
                    }
                });
                Err(SftpError::Cancelled)
            }
            answer = &mut opening => {
                answer.map_err(|e| SftpError::Transport(e.to_string()))
            }
        }
    }

    /// Opens `path` to read (`open` with READ): see [`Self::open_handle`].
    async fn open_to_read(&self, path: &str) -> Result<String, SftpError> {
        let (raw, at) = (self.raw.clone(), path.to_owned());
        self.open_handle(
            async move {
                raw.open(at, OpenFlags::READ, FileAttributes::empty())
                    .await
                    .map(|handle| handle.handle)
            },
            None,
        )
        .await?
        .map_err(|e| self.fail(e, path))
    }

    /// Closes `handle`: waited for, or — the operation given up — in the
    /// background, so the answer doesn't wait on the link.
    async fn let_go(&self, handle: String) {
        if self.is_cancelled() {
            let raw = self.raw.clone();
            tokio::spawn(async move {
                let _ = raw.close(handle).await;
            });
        } else {
            let _ = self.raw.close(handle).await;
        }
    }

    /// False once a request found the channel gone, or the channel ended.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.raw.ended.is_cancelled()
    }

    /// The login directory, where relative paths start (`realpath .`): SFTP
    /// never expands `~`.
    pub async fn home(&self) -> Result<String, SftpError> {
        let name = self
            .unless_cancelled(async { self.raw.realpath(".").await.map_err(|e| self.fail(e, ".")) })
            .await?;
        match name.files.into_iter().next() {
            Some(file) if file.filename.starts_with('/') => Ok(file.filename),
            _ => Err(SftpError::NotFound(".".to_owned())),
        }
    }

    /// What `path` is (following a symlink, as `stat` does).
    pub async fn stat(&self, path: &str) -> Result<RemoteEntry, SftpError> {
        let attrs = self
            .unless_cancelled(async { self.raw.stat(path).await.map_err(|e| self.fail(e, path)) })
            .await?;
        Ok(RemoteEntry::new(path.to_owned(), &attrs.attrs))
    }

    /// The entries of the folder at `path`, without `.` and `..`, unsorted;
    /// each path is `path` joined with the name.
    ///
    /// Fix (B460): readdir reports a symlink as the link itself (OpenSSH's
    /// sftp-server lstats each entry), and a link to a folder read as a file
    /// with no extension — which the folder sheet, showing folders and
    /// previewable files, dropped. Each link is stat'ed (pipelined) and read
    /// as what it points at, as the exec listing reads it (B333); a link
    /// pointing nowhere stays as listed.
    pub async fn list(&self, path: &str) -> Result<Vec<RemoteEntry>, SftpError> {
        let (raw, at) = (self.raw.clone(), path.to_owned());
        let handle = self
            .open_handle(
                async move { raw.opendir(at).await.map(|handle| handle.handle) },
                None,
            )
            .await?
            .map_err(|e| self.fail(e, path))?;
        let base = path.strip_suffix('/').unwrap_or(path);
        let mut entries = Vec::new();
        let result = loop {
            let page = self
                .unless_cancelled(async { Ok(self.raw.readdir(handle.as_str()).await) })
                .await;
            match page {
                Ok(Ok(name)) => entries.extend(
                    name.files
                        .into_iter()
                        .filter(|f| f.filename != "." && f.filename != "..")
                        .map(|f| (format!("{base}/{}", f.filename), f.attrs)),
                ),
                Ok(Err(RawError::Status(status))) if status.status_code == StatusCode::Eof => {
                    break Ok(entries);
                }
                Ok(Err(e)) => break Err(self.fail(e, path)),
                Err(cancelled) => break Err(cancelled),
            }
        };
        self.let_go(handle).await;
        let mut listed = result?;
        let mut follows = JoinSet::new();
        for (index, (at, attrs)) in listed.iter().enumerate() {
            if attrs
                .permissions
                .is_some_and(|mode| mode & FILE_TYPE_MASK == SYMLINK)
            {
                let (raw, at) = (self.raw.clone(), at.clone());
                follows.spawn(async move { (index, raw.stat(at).await) });
            }
        }
        while let Some(done) = self
            .unless_cancelled(async { Ok(follows.join_next().await) })
            .await?
        {
            if let Ok((index, Ok(target))) = done {
                listed[index].1 = target.attrs;
            }
        }
        Ok(listed
            .into_iter()
            .map(|(at, attrs)| RemoteEntry::new(at, &attrs))
            .collect())
    }

    /// Opens `path` for reads at any offset — what streaming playback pulls
    /// ranges through. An empty file is not worth a player (`openRemoteFile`).
    pub async fn open(&self, path: &str) -> Result<RemoteFile, SftpError> {
        let handle = self.open_to_read(path).await?;
        let mut file = RemoteFile {
            sftp: self.clone(),
            handle: Some(handle.clone()),
            size: 0,
            path: path.to_owned(),
        };
        let attrs = self
            .unless_cancelled(async { Ok(self.raw.fstat(handle).await) })
            .await;
        let attrs = match attrs {
            Ok(attrs) => attrs,
            Err(cancelled) => {
                if let Some(handle) = file.handle.take() {
                    self.let_go(handle).await;
                }
                return Err(cancelled);
            }
        };
        match attrs {
            Ok(attrs) if attrs.attrs.size.unwrap_or(0) > 0 => {
                file.size = attrs.attrs.size.unwrap_or(0);
                Ok(file)
            }
            Ok(_) => {
                file.close().await;
                Err(SftpError::NotFound(format!("{path} is empty")))
            }
            Err(e) => {
                let error = self.fail(e, path);
                file.close().await;
                Err(error)
            }
        }
    }

    /// Copies the file at `path` into `destination` (created or replaced),
    /// `READ_WINDOW` reads in flight. `progress` gets the fraction done and
    /// returns false to stop. Returns the bytes copied.
    pub async fn download(
        &self,
        path: &str,
        destination: &Path,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
    ) -> Result<u64, SftpError> {
        let handle = self.open_to_read(path).await?;
        let result = self.pull(&handle, path, destination, progress).await;
        self.let_go(handle).await;
        result
    }

    async fn pull(
        &self,
        handle: &str,
        path: &str,
        destination: &Path,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
    ) -> Result<u64, SftpError> {
        let size = self
            .unless_cancelled(async {
                self.raw.fstat(handle).await.map_err(|e| self.fail(e, path))
            })
            .await?
            .attrs
            .size
            .unwrap_or(0);
        let file = std::fs::File::create(destination)?;
        let mut window = ReadWindow::new(size, READ_CHUNK);
        let mut reads = JoinSet::new();
        let mut received = 0u64;
        loop {
            while reads.len() < READ_WINDOW
                && let Some((offset, length)) = window.next_range(reads.len())
            {
                let raw = self.raw.clone();
                let handle = handle.to_owned();
                reads.spawn(async move {
                    let answer = match raw.read(handle, offset, length).await {
                        Ok(data) => Ok(data.data),
                        Err(RawError::Status(s)) if s.status_code == StatusCode::Eof => {
                            Ok(Vec::new())
                        }
                        Err(e) => Err(e),
                    };
                    (offset, length, answer)
                });
            }
            // Dropped with the reads it holds when the download is given
            // up (a JoinSet aborts its tasks).
            let Some(done) = self
                .unless_cancelled(async { Ok(reads.join_next().await) })
                .await?
            else {
                break;
            };
            let (offset, length, answer) = done.map_err(|e| SftpError::Transport(e.to_string()))?;
            let bytes = answer.map_err(|e| self.fail(e, path))?;
            let bytes = &bytes[..bytes.len().min(length as usize)];
            if let Some(written) = window.landed(offset, length, bytes.len()) {
                write_at(&file, &bytes[..written], offset)?;
                received += written as u64;
                if size > 0 && !progress((received as f64 / size as f64).min(1.0)) {
                    return Err(SftpError::Cancelled);
                }
            }
        }
        Ok(window.length(received))
    }

    /// Streams `local` to `remote`, which must not exist yet (SFTP's
    /// exclusive create). Whatever a failed upload wrote is removed.
    pub async fn upload_new(
        &self,
        local: &Path,
        remote: &str,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
    ) -> Result<(), SftpError> {
        self.not_cancelled()?;
        let file = std::fs::File::open(local)?;
        let size = file.metadata()?.len();
        let (raw, at) = (self.raw.clone(), remote.to_owned());
        let handle = self
            .open_handle(
                async move {
                    raw.open(
                        at,
                        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                        FileAttributes::empty(),
                    )
                    .await
                    .map(|handle| handle.handle)
                },
                // Exclusive: a file there afterwards is this one.
                Some(remote.to_owned()),
            )
            .await?;
        let handle = match handle {
            Ok(handle) => handle,
            Err(e) => {
                let error = self.fail(e, remote);
                // OpenSSH answers an exclusive create on a name that is
                // taken with a bare "failure"; say which failure it was.
                if matches!(error, SftpError::Failure { .. })
                    && self.raw.lstat(remote).await.is_ok()
                {
                    return Err(SftpError::NameTaken(remote.to_owned()));
                }
                return Err(error);
            }
        };
        let pushed = self
            .push(&handle, remote, size, progress, |buf, offset| {
                read_at(&file, buf, offset)
            })
            .await;
        if self.is_cancelled() {
            // Given up: what it wrote goes, without the answer waiting.
            let raw = self.raw.clone();
            let remote = remote.to_owned();
            tokio::spawn(async move {
                let _ = raw.close(handle).await;
                let _ = raw.remove(remote).await;
            });
            return Err(SftpError::Cancelled);
        }
        let closed = self.raw.close(handle).await;
        match (pushed, closed) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(e), _) => {
                let _ = self.raw.remove(remote).await;
                Err(e)
            }
            (Ok(()), Err(e)) => {
                let _ = self.raw.remove(remote).await;
                Err(self.fail(e, remote))
            }
        }
    }

    /// Writes `data` to `remote`, creating it with `permissions` or
    /// truncating what is there.
    pub async fn write_file(
        &self,
        remote: &str,
        data: &[u8],
        permissions: u32,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
    ) -> Result<(), SftpError> {
        let attrs = FileAttributes {
            permissions: Some(permissions),
            ..FileAttributes::empty()
        };
        let (raw, at) = (self.raw.clone(), remote.to_owned());
        let handle = self
            .open_handle(
                async move {
                    raw.open(
                        at,
                        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
                        attrs,
                    )
                    .await
                    .map(|handle| handle.handle)
                },
                None,
            )
            .await?
            .map_err(|e| self.fail(e, remote))?;
        let pushed = self
            .push(
                &handle,
                remote,
                data.len() as u64,
                progress,
                |buf, offset| {
                    let start = offset as usize;
                    let n = buf.len().min(data.len() - start);
                    buf[..n].copy_from_slice(&data[start..start + n]);
                    Ok(n)
                },
            )
            .await;
        if self.is_cancelled() {
            self.let_go(handle).await;
            return Err(SftpError::Cancelled);
        }
        let closed = self.raw.close(handle).await;
        pushed?;
        closed.map_err(|e| self.fail(e, remote))?;
        Ok(())
    }

    /// `size` bytes from `read(buf, offset)` written at their offsets,
    /// `WRITE_WINDOW` writes in flight.
    async fn push(
        &self,
        handle: &str,
        remote: &str,
        size: u64,
        progress: &mut (dyn FnMut(f64) -> bool + Send),
        mut read: impl FnMut(&mut [u8], u64) -> std::io::Result<usize>,
    ) -> Result<(), SftpError> {
        let mut writes = JoinSet::new();
        let mut offset = 0u64;
        let mut written = 0u64;
        while offset < size || !writes.is_empty() {
            while writes.len() < WRITE_WINDOW && offset < size {
                let mut chunk = vec![0u8; WRITE_CHUNK.min((size - offset) as usize)];
                let n = read(&mut chunk, offset)?;
                if n == 0 {
                    // The local file shrank under us.
                    return Err(SftpError::Local(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the file got shorter while uploading",
                    )));
                }
                chunk.truncate(n);
                let raw = self.raw.clone();
                let handle = handle.to_owned();
                let at = offset;
                writes.spawn(async move {
                    let len = chunk.len() as u64;
                    raw.write(handle, at, chunk).await.map(|_| len)
                });
                offset += n as u64;
            }
            let Some(done) = self
                .unless_cancelled(async { Ok(writes.join_next().await) })
                .await?
            else {
                break;
            };
            let len = done
                .map_err(|e| SftpError::Transport(e.to_string()))?
                .map_err(|e| self.fail(e, remote))?;
            written += len;
            if !progress((written as f64 / size as f64).min(1.0)) {
                return Err(SftpError::Cancelled);
            }
        }
        Ok(())
    }

    /// Makes a folder (`mkdir` has no `-p`: the parent must exist).
    pub async fn create_dir(&self, path: &str) -> Result<(), SftpError> {
        self.create_dir_with(path, None).await
    }

    pub(crate) async fn create_dir_with(
        &self,
        path: &str,
        permissions: Option<u32>,
    ) -> Result<(), SftpError> {
        self.not_cancelled()?;
        let attrs = FileAttributes {
            permissions,
            ..FileAttributes::empty()
        };
        self.raw
            .mkdir(path, attrs)
            .await
            .map(|_| ())
            .map_err(|e| self.fail(e, path))
    }

    /// Renames, never over something: a taken name is refused before the
    /// rename is asked (OpenSSH's own rename refuses it too).
    ///
    /// A new name that only changes the old one's case is not taken by the
    /// file itself (r42). On a disk that ignores case (a Mac's) the old name
    /// answers for the new, and OpenSSH refuses to rename onto it, so the
    /// file goes to a free name first; the new name is taken only if it is
    /// still there once the file has left (another file, on a disk that
    /// keeps case), and the file goes back.
    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SftpError> {
        self.not_cancelled()?;
        if self.raw.lstat(to).await.is_ok() {
            if from == to || from.to_lowercase() != to.to_lowercase() {
                return Err(SftpError::NameTaken(to.to_owned()));
            }
            return self.rename_case(from, to).await;
        }
        self.raw
            .rename(from, to)
            .await
            .map(|_| ())
            .map_err(|e| self.fail(e, from))
    }

    /// [`Self::rename`] to another case of the same name, through a free
    /// name beside it.
    async fn rename_case(&self, from: &str, to: &str) -> Result<(), SftpError> {
        let mut aside = None;
        for n in 0..16 {
            let name = format!("{from}.moshpit-rename-{n}");
            if self.raw.lstat(name.as_str()).await.is_err() {
                aside = Some(name);
                break;
            }
        }
        let aside = aside.ok_or_else(|| SftpError::NameTaken(to.to_owned()))?;
        self.raw
            .rename(from, aside.as_str())
            .await
            .map_err(|e| self.fail(e, from))?;
        let moved = if self.raw.lstat(to).await.is_ok() {
            Err(SftpError::NameTaken(to.to_owned()))
        } else {
            self.raw
                .rename(aside.as_str(), to)
                .await
                .map(|_| ())
                .map_err(|e| self.fail(e, from))
        };
        if moved.is_err() {
            let _ = self.raw.rename(aside.as_str(), from).await;
        }
        moved
    }

    /// Removes a file (SFTP removes only empty folders; the sheet deletes
    /// folders over exec).
    pub async fn remove_file(&self, path: &str) -> Result<(), SftpError> {
        self.not_cancelled()?;
        self.raw
            .remove(path)
            .await
            .map(|_| ())
            .map_err(|e| self.fail(e, path))
    }

    /// Ends the session.
    pub fn close(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.raw.close_session();
    }

    fn fail(&self, e: RawError, path: &str) -> SftpError {
        match e {
            RawError::Status(status) => match status.status_code {
                StatusCode::NoSuchFile => SftpError::NotFound(path.to_owned()),
                StatusCode::PermissionDenied => SftpError::PermissionDenied(path.to_owned()),
                StatusCode::OpUnsupported => SftpError::Unavailable(status.error_message),
                StatusCode::NoConnection | StatusCode::ConnectionLost => {
                    self.alive.store(false, Ordering::SeqCst);
                    SftpError::Transport(status.error_message)
                }
                _ => SftpError::Failure {
                    message: if status.error_message.is_empty() {
                        status.status_code.to_string()
                    } else {
                        status.error_message
                    },
                },
            },
            RawError::Timeout => SftpError::Transport(format!(
                "{} didn't answer within {REQUEST_TIMEOUT_SECS} s",
                self.host
            )),
            other => {
                self.alive.store(false, Ordering::SeqCst);
                SftpError::Transport(other.to_string())
            }
        }
    }
}

/// russh-sftp's session, every request bounded by the channel under it
/// (B277). When the channel's stream ends, russh-sftp's reader fails what is
/// waiting, and its writer stops a moment later; a request taken in between
/// is written nowhere and waits out its whole timeout (60 s). A download cut
/// by a dropped connection hung that long on the close it sends its handle
/// after the failed read, and a session started as the link went waited out
/// the channel timeout on its greeting. Here a request answers the moment
/// the stream has ended: no answer can come after that.
struct Raw {
    session: RawSftpSession,
    /// Cancelled when the channel's stream ended ([`Watched`]).
    ended: CancellationToken,
}

impl Raw {
    fn over<S>(stream: S) -> Raw
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ended = CancellationToken::new();
        let config = russh_sftp::client::Config {
            request_timeout_secs: REQUEST_TIMEOUT_SECS,
            ..Default::default()
        };
        let stream = Watched {
            inner: PastNoise::new(stream),
            ended: ended.clone(),
        };
        Raw {
            session: RawSftpSession::new_with_config(stream, config),
            ended,
        }
    }

    /// `request`'s answer, or — the channel ended first — its end.
    async fn bounded<T>(
        &self,
        request: impl Future<Output = Result<T, RawError>>,
    ) -> Result<T, RawError> {
        tokio::select! {
            biased;
            answer = request => answer,
            () = self.ended.cancelled() => Err(RawError::UnexpectedBehavior(
                "the SFTP channel closed".to_owned(),
            )),
        }
    }

    fn close_session(&self) -> Result<(), RawError> {
        self.session.close_session()
    }

    async fn init(&self) -> Result<Version, RawError> {
        self.bounded(self.session.init()).await
    }

    async fn open(
        &self,
        path: impl Into<String>,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, RawError> {
        self.bounded(self.session.open(path, flags, attrs)).await
    }

    async fn close(&self, handle: impl Into<String>) -> Result<Status, RawError> {
        self.bounded(self.session.close(handle)).await
    }

    async fn read(
        &self,
        handle: impl Into<String>,
        offset: u64,
        length: u32,
    ) -> Result<Data, RawError> {
        self.bounded(self.session.read(handle, offset, length))
            .await
    }

    async fn write(
        &self,
        handle: impl Into<String>,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, RawError> {
        self.bounded(self.session.write(handle, offset, data)).await
    }

    async fn lstat(&self, path: impl Into<String>) -> Result<Attrs, RawError> {
        self.bounded(self.session.lstat(path)).await
    }

    async fn fstat(&self, handle: impl Into<String>) -> Result<Attrs, RawError> {
        self.bounded(self.session.fstat(handle)).await
    }

    async fn stat(&self, path: impl Into<String>) -> Result<Attrs, RawError> {
        self.bounded(self.session.stat(path)).await
    }

    async fn opendir(&self, path: impl Into<String>) -> Result<Handle, RawError> {
        self.bounded(self.session.opendir(path)).await
    }

    async fn readdir(&self, handle: impl Into<String>) -> Result<Name, RawError> {
        self.bounded(self.session.readdir(handle)).await
    }

    async fn remove(&self, path: impl Into<String>) -> Result<Status, RawError> {
        self.bounded(self.session.remove(path)).await
    }

    async fn mkdir(
        &self,
        path: impl Into<String>,
        attrs: FileAttributes,
    ) -> Result<Status, RawError> {
        self.bounded(self.session.mkdir(path, attrs)).await
    }

    async fn realpath(&self, path: impl Into<String>) -> Result<Name, RawError> {
        self.bounded(self.session.realpath(path)).await
    }

    async fn rename(
        &self,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Result<Status, RawError> {
        self.bounded(self.session.rename(from, to)).await
    }
}

/// The subsystem channel's stream, cancelling `ended` the moment its read
/// side finishes (the end of the stream, or an error): the host's answers
/// come that way and no other.
struct Watched<S> {
    inner: S,
    ended: CancellationToken,
}

impl<S: AsyncRead + Unpin> AsyncRead for Watched<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let read = Pin::new(&mut self.inner).poll_read(cx, buf);
        let finished = match &read {
            Poll::Ready(Err(_)) => true,
            Poll::Ready(Ok(())) => buf.filled().len() == before && buf.remaining() > 0,
            Poll::Pending => false,
        };
        if finished {
            self.ended.cancel();
        }
        read
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Watched<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// How much an rc file may print before the SFTP greeting is given up on
/// and the stream handed over as it came.
const NOISE_LIMIT: usize = 64 * 1024;

/// The subsystem channel's stream, past whatever the account's shell
/// printed before the SFTP server started.
///
/// Fix (B463): sshd runs the server as `$SHELL -c sftp-server`, so an rc
/// file that prints for every shell — bash reads `~/.bashrc` for sshd's
/// `-c` — puts its text in front of the greeting. The client read that text
/// as a packet length far past the limit and SFTP failed; every preview and
/// listing then fell back to exec, whose output carries the same text, and
/// came back garbled. Until the greeting (`SSH_FXP_VERSION`: a short
/// length, type 2, version 1–6) has come, what comes first is set aside and
/// dropped up to it. Text never holds the zero bytes a length starts with,
/// so a clean stream's greeting is found at once, at its start. One that
/// never comes within [`NOISE_LIMIT`] is handed over as it came.
struct PastNoise<S> {
    inner: S,
    /// What has come while looking for the greeting; `None` once found.
    seeking: Option<Vec<u8>>,
    /// The greeting and what followed it, still to hand over.
    ready: Vec<u8>,
    handed: usize,
}

impl<S> PastNoise<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            seeking: Some(Vec::new()),
            ready: Vec::new(),
            handed: 0,
        }
    }

    /// Done looking: `bytes` from `from` on are the stream.
    fn found(&mut self, bytes: Vec<u8>, from: usize) {
        self.seeking = None;
        self.ready = bytes;
        self.handed = from;
    }
}

/// Where an SFTP version packet starts in `bytes`.
fn greeting_at(bytes: &[u8]) -> Option<usize> {
    bytes.windows(9).position(|w| {
        let length = u32::from_be_bytes([w[0], w[1], w[2], w[3]]);
        let version = u32::from_be_bytes([w[5], w[6], w[7], w[8]]);
        (5..=64 * 1024).contains(&length) && w[4] == 2 && (1..=6).contains(&version)
    })
}

impl<S: AsyncRead + Unpin> AsyncRead for PastNoise<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.handed < this.ready.len() {
                let n = buf.remaining().min(this.ready.len() - this.handed);
                buf.put_slice(&this.ready[this.handed..this.handed + n]);
                this.handed += n;
                if this.handed == this.ready.len() {
                    this.ready = Vec::new();
                    this.handed = 0;
                }
                return Poll::Ready(Ok(()));
            }
            let Some(seen) = this.seeking.as_mut() else {
                return Pin::new(&mut this.inner).poll_read(cx, buf);
            };
            let mut chunk = [0u8; 4096];
            let mut read = ReadBuf::new(&mut chunk);
            match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) if read.filled().is_empty() => {
                    // The end, no greeting: what came, then the end.
                    let seen = std::mem::take(seen);
                    this.found(seen, 0);
                    if this.ready.is_empty() {
                        return Poll::Ready(Ok(()));
                    }
                }
                Poll::Ready(Ok(())) => {
                    seen.extend_from_slice(read.filled());
                    if let Some(at) = greeting_at(seen) {
                        if at > 0 {
                            log::warn!(
                                "sftp: dropped {at} bytes the account's shell printed before the server's greeting"
                            );
                        }
                        let seen = std::mem::take(seen);
                        this.found(seen, at);
                    } else if seen.len() > NOISE_LIMIT {
                        let seen = std::mem::take(seen);
                        this.found(seen, 0);
                    }
                }
            }
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PastNoise<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A file held open for reads at any offset. Reads may overlap — each is its
/// own request on the wire. Closed by [`Self::close`], or in the background
/// when dropped (a playback given up shouldn't hold a handle on the host).
pub struct RemoteFile {
    sftp: Sftp,
    /// `None` once closed.
    handle: Option<String>,
    size: u64,
    path: String,
}

impl RemoteFile {
    pub fn size(&self) -> u64 {
        self.size
    }

    /// At most `length` bytes from `offset` (OpenSSH answers 255 KiB at the
    /// most); empty at the end of the file.
    pub async fn read(&self, offset: u64, length: u32) -> Result<Vec<u8>, SftpError> {
        let handle = self.handle.as_deref().unwrap_or_default();
        match self.sftp.raw.read(handle, offset, length).await {
            Ok(data) => Ok(data.data),
            Err(RawError::Status(s)) if s.status_code == StatusCode::Eof => Ok(Vec::new()),
            Err(e) => Err(self.sftp.fail(e, &self.path)),
        }
    }

    pub async fn close(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = self.sftp.raw.close(handle).await;
        }
    }
}

impl Drop for RemoteFile {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let raw = self.sftp.raw.clone();
            runtime.spawn(async move {
                let _ = raw.close(handle).await;
            });
        }
    }
}

/// The ranges a windowed download asks for (`WindowedRead.pull`): fixed
/// chunks up to the known size, short answers' remainders asked again, and
/// a file of unknown size read one range at a time so EOF is never raced.
struct ReadWindow {
    next: u64,
    end: Option<u64>,
    chunk: u32,
    gaps: Vec<(u64, u32)>,
    eof: bool,
}

impl ReadWindow {
    fn new(size: u64, chunk: u32) -> Self {
        Self {
            next: 0,
            end: (size > 0).then_some(size),
            chunk,
            gaps: Vec::new(),
            eof: false,
        }
    }

    fn next_range(&mut self, in_flight: usize) -> Option<(u64, u32)> {
        if let Some(gap) = self.gaps.pop() {
            return Some(gap);
        }
        match self.end {
            Some(end) => {
                if self.next >= end {
                    return None;
                }
                let length = u64::from(self.chunk).min(end - self.next) as u32;
                let range = (self.next, length);
                self.next += u64::from(length);
                Some(range)
            }
            None => {
                if self.eof || in_flight > 0 {
                    return None;
                }
                let range = (self.next, self.chunk);
                self.next += u64::from(self.chunk);
                Some(range)
            }
        }
    }

    /// An answer of `got` bytes for the range at `offset`. Returns how many
    /// of them to keep, or `None` for EOF.
    fn landed(&mut self, offset: u64, length: u32, got: usize) -> Option<usize> {
        if got == 0 {
            // Nothing at or past this offset exists.
            self.eof = true;
            self.end = Some(self.end.map_or(offset, |end| end.min(offset)));
            self.gaps.retain(|gap| gap.0 < offset);
            return None;
        }
        if let Some(end) = self.end
            && offset >= end
        {
            return None;
        }
        let got = got.min(length as usize);
        if got < length as usize {
            self.gaps.push((offset + got as u64, length - got as u32));
        }
        Some(got)
    }

    fn length(&self, received: u64) -> u64 {
        self.end.map_or(received, |end| end.min(received))
    }
}

#[cfg(unix)]
fn write_at(file: &std::fs::File, bytes: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, bytes, offset)
}

#[cfg(unix)]
fn read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match std::os::unix::fs::FileExt::read_at(file, &mut buf[filled..], offset + filled as u64)?
        {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B277: the host's side of the channel ends while russh-sftp's writer
    /// is still busy (here held in a write the host never reads; on a real
    /// link, the moment before it notices). A request made then — the
    /// close a failed download sends its handle — is answered at once as
    /// the channel's end, not after the 60 s request timeout; so is one
    /// still waiting, and the session says it is gone.
    #[tokio::test]
    async fn a_request_after_the_channel_ended_answers_at_once() {
        use tokio::io::AsyncWriteExt;
        let (client, mut host) = tokio::io::duplex(64);
        let sftp = Sftp::over(client, "host".to_owned());
        // A write bigger than the pipe: the writer is held in it.
        let held = tokio::spawn({
            let sftp = sftp.clone();
            async move { sftp.raw.write("h", 0, vec![0; 64 << 10]).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // The host hangs up its side without reading.
        host.shutdown().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let asked = std::time::Instant::now();
        let closed =
            tokio::time::timeout(std::time::Duration::from_secs(5), sftp.raw.close("h")).await;
        assert!(
            matches!(closed, Ok(Err(_))),
            "a close after the end: {closed:?} after {:?}",
            asked.elapsed()
        );
        assert!(asked.elapsed() < std::time::Duration::from_secs(1));
        let stat = tokio::time::timeout(std::time::Duration::from_secs(5), sftp.stat("x")).await;
        assert!(matches!(stat, Ok(Err(SftpError::Transport(_)))), "{stat:?}");
        assert!(!sftp.is_alive());
        let held = tokio::time::timeout(std::time::Duration::from_secs(5), held).await;
        assert!(matches!(held, Ok(Ok(Err(_)))), "the held write: {held:?}");
        drop(host);
    }

    /// What a [`PastNoise`] over `parts` (written one at a time, the host
    /// side then closed) hands over.
    async fn past_noise(parts: Vec<Vec<u8>>) -> Vec<u8> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client, mut host) = tokio::io::duplex(1024);
        let writer = tokio::spawn(async move {
            for part in parts {
                host.write_all(&part).await.unwrap();
                host.flush().await.unwrap();
                tokio::task::yield_now().await;
            }
            host.shutdown().await.unwrap();
        });
        let mut out = Vec::new();
        PastNoise::new(client).read_to_end(&mut out).await.unwrap();
        writer.await.unwrap();
        out
    }

    /// OpenSSH's greeting: length 5, `SSH_FXP_VERSION`, version 3.
    const GREETING: [u8; 9] = [0, 0, 0, 5, 2, 0, 0, 0, 3];

    /// B463: the greeting found past text that spans many reads, and split
    /// across two itself; what follows it is handed over untouched. A
    /// clean stream comes through as it is, and so does one with no
    /// greeting at all.
    #[tokio::test]
    async fn the_greeting_is_found_past_what_the_shell_printed() {
        let after: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let text = "motd: welcome\n".repeat(700).into_bytes();
        let got = past_noise(vec![
            text.clone(),
            GREETING[..4].to_vec(),
            GREETING[4..].to_vec(),
            after.clone(),
        ])
        .await;
        assert_eq!(got, [&GREETING[..], &after].concat());

        let clean = [&GREETING[..], &after].concat();
        assert_eq!(past_noise(vec![clean.clone()]).await, clean);

        assert_eq!(past_noise(vec![text.clone()]).await, text);
        assert!(past_noise(Vec::new()).await.is_empty());
    }

    /// Runs a window against a file of `actual` bytes whose server answers
    /// at most `cap` bytes per read, in reverse order of asking.
    fn pull(announced: u64, actual: u64, cap: usize) -> (u64, Vec<(u64, usize)>) {
        let mut window = ReadWindow::new(announced, 10);
        let mut pieces = Vec::new();
        let mut received = 0;
        let mut in_flight: Vec<(u64, u32)> = Vec::new();
        loop {
            while in_flight.len() < 3
                && let Some(range) = window.next_range(in_flight.len())
            {
                in_flight.push(range);
            }
            let Some((offset, length)) = in_flight.pop() else {
                break;
            };
            let available = actual.saturating_sub(offset) as usize;
            let got = available.min(length as usize).min(cap);
            if let Some(kept) = window.landed(offset, length, got) {
                pieces.push((offset, kept));
                received += kept as u64;
            }
        }
        (window.length(received), pieces)
    }

    fn covered(pieces: &[(u64, usize)]) -> Vec<bool> {
        let end = pieces
            .iter()
            .map(|(o, n)| *o as usize + n)
            .max()
            .unwrap_or(0);
        let mut seen = vec![false; end];
        for (offset, n) in pieces {
            for byte in &mut seen[*offset as usize..*offset as usize + n] {
                assert!(!*byte, "a byte landed twice");
                *byte = true;
            }
        }
        seen
    }

    #[test]
    fn a_known_size_is_read_whole_in_any_order() {
        let (length, pieces) = pull(95, 95, 10);
        assert_eq!(length, 95);
        assert!(covered(&pieces).iter().all(|b| *b));
    }

    #[test]
    fn short_answers_are_asked_again() {
        let (length, pieces) = pull(95, 95, 4);
        assert_eq!(length, 95);
        let seen = covered(&pieces);
        assert_eq!(seen.len(), 95);
        assert!(seen.iter().all(|b| *b));
    }

    #[test]
    fn a_file_that_shrank_ends_at_its_eof() {
        let (length, pieces) = pull(95, 42, 10);
        assert_eq!(length, 42);
        assert_eq!(covered(&pieces).len(), 42);
    }

    #[test]
    fn an_unknown_size_reads_one_range_at_a_time_to_eof() {
        let (length, pieces) = pull(0, 37, 10);
        assert_eq!(length, 37);
        assert!(covered(&pieces).iter().all(|b| *b));
    }
}
