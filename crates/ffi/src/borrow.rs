//! The session's SSH for what runs beside its terminal (UniFFI): exec and
//! SFTP channels on the connection the session already has — no second
//! sign-in, no second Face ID — through `Sessions::borrow_ssh`.
//!
//! Which connection: the live SSH leg's own (SSH, SSH + tmux, SSH + herdr,
//! mosh's fall back to SSH); mosh + tmux's control plane (the bootstrap
//! connection its `-CC` sidecar rides); or, for plain mosh, one side
//! connection the session dials on the first borrow and keeps.
//!
//! Its life is the session's: a [`BorrowedSsh`] holds the connection, not
//! the session, and the session closes it when it reconnects or ends — what
//! is in flight then fails ([`BorrowFailure::Ssh`] with a `Lost` cause, or
//! [`BorrowFailure::Sftp`]'s `Transport`) and the next borrow gets the new
//! connection. A borrow is never the one to close it. After a suspension a
//! socket iOS killed still looks open: a caller that cares runs a cheap
//! command first (`true`) and borrows again on a failure.
//!
//! Every method waits on the core's runtime; the host awaits it on its own
//! executor (each call is one future, nothing here blocks a host thread).
//! A host's own cancellation doesn't reach a UniFFI future, so a call the
//! host may give up on goes through [`BorrowedSsh::cancellable`] with a
//! [`BorrowCancel`] the host trips (B243): the call answers
//! [`BorrowFailure::Cancelled`] at once, a read in flight is no longer
//! waited for, a transfer stops where it is and lets go of what it holds on
//! the host in the background. Before, a cancelled stat or listing ran to
//! its end, and a transfer to its next progress report.

use std::sync::{Arc, Mutex};

use moshpit_session::{Cause, Failure, Sessions};
use moshpit_ssh::{CancellationToken, RemoteEntry, RemoteFile, SftpError, Ssh};

use crate::session::{FailureCauseValue, cause_value};

/// Why a borrowed connection couldn't do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum BorrowFailure {
    /// The session isn't connected now (or there's no such session).
    NotConnected,
    /// The connection or a channel on it failed (`log`: English, for the
    /// log only).
    Ssh {
        cause: FailureCauseValue,
        log: String,
    },
    /// SFTP said no to `path`.
    Sftp {
        why: SftpWhyValue,
        path: String,
        log: String,
    },
    /// The progress callback asked to stop.
    Cancelled,
}

impl std::fmt::Display for BorrowFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => f.write_str("the session is not connected"),
            Self::Ssh { log, .. } | Self::Sftp { log, .. } => f.write_str(log),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for BorrowFailure {}

impl From<uniffi::UnexpectedUniFFICallbackError> for BorrowFailure {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Ssh {
            cause: FailureCauseValue::Plain,
            log: error.reason,
        }
    }
}

/// What SFTP said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpWhyValue {
    /// The host has no SFTP subsystem: fall back to exec.
    Unavailable,
    NotFound,
    PermissionDenied,
    /// Something with that name is already there.
    NameTaken,
    /// Any other refusal from the server.
    Refused,
    /// The channel went away or a request went unanswered.
    Transport,
    /// The phone's side of a transfer.
    Local,
}

impl BorrowFailure {
    fn ssh(failure: &Failure) -> Self {
        Self::Ssh {
            cause: cause_value(&failure.cause),
            log: failure.log.clone(),
        }
    }

    fn sftp(error: SftpError, path: &str) -> Self {
        let why = match &error {
            SftpError::Unavailable(_) => SftpWhyValue::Unavailable,
            SftpError::NotFound(_) => SftpWhyValue::NotFound,
            SftpError::PermissionDenied(_) => SftpWhyValue::PermissionDenied,
            SftpError::NameTaken(_) => SftpWhyValue::NameTaken,
            SftpError::Failure { .. } => SftpWhyValue::Refused,
            SftpError::Transport(_) => SftpWhyValue::Transport,
            SftpError::Cancelled => return Self::Cancelled,
            SftpError::Local(_) => SftpWhyValue::Local,
            SftpError::Ssh(ssh) => return Self::ssh(&Failure::from(ssh)),
            _ => SftpWhyValue::Transport,
        };
        Self::Sftp {
            why,
            path: path.to_owned(),
            log: error.to_string(),
        }
    }
}

/// What a command did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Its exit status, when the host sent one.
    pub status: Option<u32>,
    /// The channel closed in order: everything it printed is here. `false`:
    /// cut off (the connection dropped), however much arrived.
    pub finished: bool,
}

/// One file or folder on the host.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteEntryValue {
    pub path: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    /// Seconds since 1970.
    pub modified: Option<u64>,
}

impl From<RemoteEntry> for RemoteEntryValue {
    fn from(entry: RemoteEntry) -> Self {
        Self {
            path: entry.path,
            is_directory: entry.is_directory,
            size: entry.size,
            modified: entry.modified,
        }
    }
}

/// A slice of a remote file, and the file's size.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileChunk {
    pub bytes: Vec<u8>,
    pub size: u64,
}

/// How far a transfer is (0…1); return `false` to stop it.
#[uniffi::export(with_foreign)]
pub trait TransferProgress: Send + Sync {
    fn progress(&self, fraction: f64) -> bool;
}

/// A host's way to give up on borrowed calls in flight: made by the host,
/// handed to [`BorrowedSsh::cancellable`], tripped with [`Self::cancel`]
/// (Swift: from its task's cancellation handler). Once tripped it stays
/// tripped; a call begun after it answers `Cancelled` at once.
#[derive(uniffi::Object)]
pub struct BorrowCancel(CancellationToken);

#[uniffi::export]
impl BorrowCancel {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self(CancellationToken::new()))
    }

    /// Gives up every call made through it.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

/// The session's connection, lent.
#[derive(uniffi::Object)]
pub struct BorrowedSsh {
    ssh: Ssh,
    /// What its calls give up on ([`Self::cancellable`]); none: they run
    /// to their end.
    cancel: Option<CancellationToken>,
}

/// Runs `work` on the core's runtime and waits for it from wherever the
/// host awaits — or, with `cancel`, until that is tripped: then
/// `Cancelled` at once, while `work`, which watches the same token, stops
/// on its own and lets go of what it holds.
async fn on_core<T: Send + 'static>(
    cancel: Option<&CancellationToken>,
    work: impl std::future::Future<Output = T> + Send + 'static,
) -> Result<T, BorrowFailure> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(BorrowFailure::Cancelled);
    }
    let task = moshpit_runtime::handle().spawn(work);
    let joined = match cancel {
        None => task.await,
        Some(cancel) => tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(BorrowFailure::Cancelled),
            joined = task => joined,
        },
    };
    joined.map_err(|error| BorrowFailure::Ssh {
        cause: FailureCauseValue::Plain,
        log: error.to_string(),
    })
}

/// Borrows session `session`'s SSH.
pub(crate) async fn borrow(
    sessions: Sessions,
    session: u64,
) -> Result<Arc<BorrowedSsh>, BorrowFailure> {
    let answer = sessions.borrow_ssh(session);
    match answer.await {
        Ok(Ok(ssh)) => Ok(Arc::new(BorrowedSsh { ssh, cancel: None })),
        Ok(Err(failure)) if matches!(failure.cause, Cause::Plain) => {
            Err(BorrowFailure::NotConnected)
        }
        Ok(Err(failure)) => Err(BorrowFailure::ssh(&failure)),
        Err(_) => Err(BorrowFailure::NotConnected),
    }
}

#[uniffi::export]
impl BorrowedSsh {
    /// The same connection, every call through it given up when `cancel`
    /// is tripped (B243): one per call the host may give up on.
    pub fn cancellable(&self, cancel: Arc<BorrowCancel>) -> Arc<BorrowedSsh> {
        Arc::new(BorrowedSsh {
            ssh: self.ssh.clone(),
            cancel: Some(cancel.0.clone()),
        })
    }

    /// Whether the connection is known to be gone.
    pub fn is_closed(&self) -> bool {
        self.ssh.is_closed()
    }

    /// Runs `command` (a POSIX sh script, whatever the login shell) on an
    /// exec channel with nothing on its stdin; a command that fails is not
    /// an error here, its status says so.
    pub async fn run(&self, command: String) -> Result<ExecOutput, BorrowFailure> {
        let ssh = self.ssh.clone();
        let cancel = self.cancel.clone();
        let output = on_core(self.cancel.as_ref(), async move {
            match cancel {
                None => ssh.run(&command).await.map(Some),
                Some(cancel) => ssh.run_until(&command, &cancel).await,
            }
        })
        .await?
        .map_err(|error| BorrowFailure::ssh(&Failure::from(&error)))?;
        let output = output.ok_or(BorrowFailure::Cancelled)?;
        Ok(ExecOutput {
            stdout: output.stdout(),
            stderr: output.stderr(),
            status: output.status(),
            finished: output.closed,
        })
    }

    /// The home directory, as SFTP sees it.
    pub async fn sftp_home(&self) -> Result<String, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.home().await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, "~"))
    }

    pub async fn sftp_list(&self, path: String) -> Result<Vec<RemoteEntryValue>, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.list(&at).await
        })
        .await?
        .map(|entries| entries.into_iter().map(RemoteEntryValue::from).collect())
        .map_err(|error| BorrowFailure::sftp(error, &path))
    }

    pub async fn sftp_stat(&self, path: String) -> Result<RemoteEntryValue, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.stat(&at).await
        })
        .await?
        .map(RemoteEntryValue::from)
        .map_err(|error| BorrowFailure::sftp(error, &path))
    }

    /// Opens the file at `path` for reads at any offset, as many in flight
    /// as the caller likes: what a video streams through (one open for the
    /// whole playback, where `sftp_read` opens and closes per call). An empty
    /// file is `NotFound`, as nothing to play.
    pub async fn sftp_open(&self, path: String) -> Result<Arc<BorrowedFile>, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        let opened = on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.open(&at).await
        })
        .await?;
        match opened {
            Ok(file) => Ok(Arc::new(BorrowedFile {
                size: file.size(),
                path,
                file: Mutex::new(Some(Arc::new(file))),
            })),
            Err(error) => Err(BorrowFailure::sftp(error, &path)),
        }
    }

    /// Up to `length` bytes of the file at `path` from `offset` (a
    /// preview's first screenful, a page of a log), with the file's size.
    pub async fn sftp_read(
        &self,
        path: String,
        offset: u64,
        length: u32,
    ) -> Result<FileChunk, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            let file = sftp.open(&at).await?;
            let size = file.size();
            let read = file.read(offset, length).await;
            file.close().await;
            read.map(|bytes| FileChunk { bytes, size })
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &path))
    }

    /// Copies the remote file at `path` to the local file `destination`
    /// (created or replaced); the bytes copied.
    pub async fn sftp_download(
        &self,
        path: String,
        destination: String,
        progress: Option<Arc<dyn TransferProgress>>,
    ) -> Result<u64, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            let mut report = |fraction: f64| progress.as_ref().is_none_or(|p| p.progress(fraction));
            sftp.download(&at, std::path::Path::new(&destination), &mut report)
                .await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &path))
    }

    /// Streams the local file `local` to `remote`, which must not exist yet.
    pub async fn sftp_upload_new(
        &self,
        local: String,
        remote: String,
        progress: Option<Arc<dyn TransferProgress>>,
    ) -> Result<(), BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = remote.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            let mut report = |fraction: f64| progress.as_ref().is_none_or(|p| p.progress(fraction));
            sftp.upload_new(std::path::Path::new(&local), &at, &mut report)
                .await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &remote))
    }

    /// Writes `data` to `remote` (created with `permissions`, or truncated).
    pub async fn sftp_write(
        &self,
        remote: String,
        data: Vec<u8>,
        permissions: u32,
    ) -> Result<(), BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = remote.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.write_file(&at, &data, permissions, &mut |_| true)
                .await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &remote))
    }

    pub async fn sftp_create_dir(&self, path: String) -> Result<(), BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.create_dir(&at).await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &path))
    }

    pub async fn sftp_rename(&self, from: String, to: String) -> Result<(), BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = from.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.rename(&at, &to).await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &from))
    }

    pub async fn sftp_remove_file(&self, path: String) -> Result<(), BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let at = path.clone();
        on_core(self.cancel.as_ref(), async move {
            let sftp = sftp(&ssh, cancel).await?;
            sftp.remove_file(&at).await
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &path))
    }

    /// `data` into `~/.moshpit/uploads/<filename>` (directory 700, file
    /// 600) over SFTP, or over exec channels on a host without it; the
    /// absolute remote path (what an image attachment types into the pane).
    pub async fn upload_to_uploads_directory(
        &self,
        data: Vec<u8>,
        filename: String,
        progress: Option<Arc<dyn TransferProgress>>,
    ) -> Result<String, BorrowFailure> {
        let (ssh, cancel) = self.lent();
        let name = filename.clone();
        on_core(self.cancel.as_ref(), async move {
            let mut report = |fraction: f64| progress.as_ref().is_none_or(|p| p.progress(fraction));
            match cancel {
                None => {
                    ssh.upload_to_uploads_directory(&data, &name, &mut report)
                        .await
                }
                Some(cancel) => {
                    ssh.upload_to_uploads_directory_until(&data, &name, &mut report, &cancel)
                        .await
                }
            }
        })
        .await?
        .map_err(|error| BorrowFailure::sftp(error, &filename))
    }
}

impl BorrowedSsh {
    /// The connection and the token its calls give up on, for a call.
    fn lent(&self) -> (Ssh, Option<CancellationToken>) {
        (self.ssh.clone(), self.cancel.clone())
    }
}

/// The connection's SFTP session, its operations given up when `cancel` is.
async fn sftp(
    ssh: &Ssh,
    cancel: Option<CancellationToken>,
) -> Result<moshpit_ssh::Sftp, SftpError> {
    let sftp = ssh.sftp().await?;
    Ok(match cancel {
        Some(cancel) => sftp.cancelled_by(cancel),
        None => sftp,
    })
}

/// A host file held open over the session's SFTP ([`BorrowedSsh::sftp_open`]).
/// Reads may overlap — each is its own request on the wire. Closed by
/// [`BorrowedFile::close`]; one dropped open is closed on the core's runtime
/// (a playback given up shouldn't hold a handle on the host). Its life is
/// the connection's: once that is gone, reads fail.
#[derive(uniffi::Object)]
pub struct BorrowedFile {
    size: u64,
    path: String,
    /// `None` once closed. Each read holds the file for as long as it is in
    /// flight, so a close never pulls it from under one.
    file: Mutex<Option<Arc<RemoteFile>>>,
}

impl BorrowedFile {
    fn take(&self) -> Option<Arc<RemoteFile>> {
        self.file.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// Closes `file` once nothing reads it any more: now if this is the last
/// hold on it, else when the last read lets go (`RemoteFile`'s drop, on the
/// runtime the read ran on). Runs on the core's runtime.
async fn close_when_free(file: Arc<RemoteFile>) {
    if let Ok(file) = Arc::try_unwrap(file) {
        file.close().await;
    }
}

#[uniffi::export]
impl BorrowedFile {
    /// The file's size when it was opened.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// At most `length` bytes from `offset` (OpenSSH answers 255 KiB at the
    /// most, and a short answer is not the end); empty at the end of the
    /// file. A read after the close fails.
    pub async fn read(&self, offset: u64, length: u32) -> Result<Vec<u8>, BorrowFailure> {
        let file = self.file.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(file) = file else {
            return Err(BorrowFailure::Sftp {
                why: SftpWhyValue::Transport,
                path: self.path.clone(),
                log: format!("{} was closed", self.path),
            });
        };
        on_core(None, async move { file.read(offset, length).await })
            .await?
            .map_err(|error| BorrowFailure::sftp(error, &self.path))
    }

    /// Lets the host's handle go (once; later calls do nothing).
    pub async fn close(&self) {
        if let Some(file) = self.take() {
            let _ = on_core(None, close_when_free(file)).await;
        }
    }
}

impl Drop for BorrowedFile {
    fn drop(&mut self) {
        if let Some(file) = self.take() {
            // A host thread may drop the last reference: the close goes out
            // from the core's runtime, where `RemoteFile` can reach the wire.
            moshpit_runtime::handle().spawn(close_when_free(file));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    /// B243: a host's cancellation never reached a borrowed call (a UniFFI
    /// future isn't cancelled with the Swift task awaiting it); a call made
    /// with a token answers `Cancelled` the moment the host trips it, and
    /// one begun after that never runs.
    #[test]
    fn a_call_the_host_gives_up_answers_cancelled_at_once() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let cancel = BorrowCancel::new();
        let tripper = cancel.clone();
        let answer = runtime.block_on(async {
            let trip = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                tripper.cancel();
            });
            let started = Instant::now();
            let answer = tokio::time::timeout(
                Duration::from_secs(5),
                on_core(Some(&cancel.0), std::future::pending::<()>()),
            )
            .await;
            trip.await.unwrap();
            (answer, started.elapsed())
        });
        let (answer, took) = answer;
        assert_eq!(answer, Ok(Err(BorrowFailure::Cancelled)));
        assert!(took < Duration::from_secs(1), "{took:?}");
        assert!(cancel.is_cancelled());

        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let answer = runtime.block_on(on_core(Some(&cancel.0), async move {
            flag.store(true, Ordering::SeqCst);
        }));
        assert_eq!(answer, Err(BorrowFailure::Cancelled));
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !ran.load(Ordering::SeqCst),
            "a call begun after the cancel ran"
        );

        // Without a token a call runs to its answer, as before.
        assert_eq!(runtime.block_on(on_core(None, async { 7 })), Ok(7));
    }
}
