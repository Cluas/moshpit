//! Uploads into `~/.moshpit/uploads/` (screenshots for an agent): SFTP
//! first, exec channels on a host without it (`SSHSession.
//! uploadToUploadsDirectory` and `ExecUploadCommands`).

use russh::keys::ssh_encoding::base64::{Base64, Encoding};
use tokio_util::sync::CancellationToken;

use crate::connection::Ssh;
use crate::error::SftpError;

/// The command lines of the no-SFTP upload: base64 chunks appended to a
/// temp file over exec channels, decoded once at the end (`ExecUploadCommands`).
/// Strict POSIX sh — the exec channel runs whatever `/bin/sh` the host has —
/// and every path through one shell quoter, since a quoting bug here writes
/// bytes to the wrong path on someone else's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecUploadPlan {
    pub begin: String,
    pub finish: String,
    /// What a stopped or failed upload leaves undone (B330): the half-file
    /// and whatever the decode got to, removed.
    pub abandon: String,
    pub final_path: String,
    temp_path: String,
}

impl ExecUploadPlan {
    /// Raw bytes per exec round trip: 48 KiB is 64 KiB of base64, inside
    /// every sshd and shell argument limit seen in the wild, and a 2 MB
    /// image in ~45 round trips.
    pub const CHUNK_BYTES: usize = 48 * 1024;
    pub const SUCCESS_MARKER: &'static str = "MOSHPIT_UPLOAD_OK";

    pub fn new(home: &str, filename: &str) -> Self {
        let dir_parent = format!("{home}/.moshpit");
        let dir = format!("{dir_parent}/uploads");
        let final_path = format!("{dir}/{filename}");
        let temp_path = format!("{final_path}.b64part");
        let (q_parent, q_dir) = (quote(&dir_parent), quote(&dir));
        let (q_final, q_temp) = (quote(&final_path), quote(&temp_path));
        Self {
            begin: format!("mkdir -p {q_dir} && chmod 700 {q_parent} {q_dir} && rm -f {q_temp}"),
            // `base64 -d` and `-D` differ across ages of BSD: try both.
            finish: format!(
                "{{ base64 -d < {q_temp} > {q_final} 2>/dev/null || base64 -D < {q_temp} > {q_final}; }} && chmod 600 {q_final} && rm -f {q_temp} && echo {}",
                Self::SUCCESS_MARKER
            ),
            abandon: format!("rm -f {q_temp} {q_final}"),
            final_path,
            temp_path,
        }
    }

    /// The base64 alphabet is inert inside single quotes.
    pub fn append(&self, chunk_base64: &str) -> String {
        format!("printf %s '{chunk_base64}' >> {}", quote(&self.temp_path))
    }
}

/// One shell word, in any login shell ([`crate::shell::quote`]).
pub fn quote(path: &str) -> String {
    crate::shell::quote(path)
}

/// `cancel`: given up when it is ([`Ssh::upload_to_uploads_directory_until`]).
pub(crate) async fn to_uploads_directory(
    ssh: &Ssh,
    data: &[u8],
    filename: &str,
    progress: &mut (dyn FnMut(f64) -> bool + Send),
    cancel: Option<&CancellationToken>,
) -> Result<String, SftpError> {
    if !ssh.sftp_known_unavailable() {
        match over_sftp(ssh, data, filename, progress, cancel).await {
            Ok(path) => return Ok(path),
            Err(SftpError::Cancelled) => return Err(SftpError::Cancelled),
            // Any SFTP failure falls through to exec, whose error wins if
            // it fails too: it is the one still standing.
            Err(e) => log::debug!("upload: SFTP failed ({e}); trying exec"),
        }
    }
    let path = over_exec(ssh, data, filename, progress, cancel).await?;
    // SFTP is given up for the connection only when the host refused the
    // subsystem, and once exec has actually worked (B723). A failure of
    // this upload's own (a path's permission, the network, a timeout) sent
    // every later upload over exec too: a third more bytes, a round trip
    // per chunk.
    if ssh.sftp_refused() {
        ssh.mark_sftp_unavailable();
    }
    Ok(path)
}

/// The home comes from `realpath .` (SFTP starts in `$HOME` but never
/// expands `~`). `mkdir` has no `-p` and fails on an existing directory, so
/// both creates are best effort; a directory that truly can't be made fails
/// at the open.
async fn over_sftp(
    ssh: &Ssh,
    data: &[u8],
    filename: &str,
    progress: &mut (dyn FnMut(f64) -> bool + Send),
    cancel: Option<&CancellationToken>,
) -> Result<String, SftpError> {
    let mut sftp = ssh.sftp().await?;
    if let Some(cancel) = cancel {
        sftp = sftp.cancelled_by(cancel.clone());
    }
    let home = sftp.home().await?;
    let app_dir = format!("{home}/.moshpit");
    let uploads = format!("{app_dir}/uploads");
    let _ = sftp.create_dir_with(&app_dir, Some(0o700)).await;
    let _ = sftp.create_dir_with(&uploads, Some(0o700)).await;
    let path = format!("{uploads}/{filename}");
    match sftp.write_file(&path, data, 0o600, progress).await {
        Ok(()) => Ok(path),
        // B330: what it wrote goes — an agent is given this name. A stop
        // answers at once and the removal follows on the connection's own
        // SFTP; a failure removes it before exec takes over the same name.
        Err(SftpError::Cancelled) => {
            let ssh = ssh.clone();
            tokio::spawn(async move {
                if let Ok(sftp) = ssh.sftp().await {
                    let _ = sftp.remove_file(&path).await;
                }
            });
            Err(SftpError::Cancelled)
        }
        Err(error) => {
            let _ = sftp.remove_file(&path).await;
            Err(error)
        }
    }
}

async fn over_exec(
    ssh: &Ssh,
    data: &[u8],
    filename: &str,
    progress: &mut (dyn FnMut(f64) -> bool + Send),
    cancel: Option<&CancellationToken>,
) -> Result<String, SftpError> {
    let run_ok = |command: String| run_ok(ssh, command, cancel);
    let home = past_mark(&run_ok(HOME_COMMAND.to_owned()).await?);
    if !home.starts_with('/') {
        return Err(SftpError::Failure {
            message: "the host's $HOME is not an absolute path".to_owned(),
        });
    }
    let plan = ExecUploadPlan::new(&home, filename);
    run_ok(plan.begin.clone()).await?;
    let sent = async {
        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + ExecUploadPlan::CHUNK_BYTES).min(data.len());
            let chunk = Base64::encode_string(&data[offset..end]);
            run_ok(plan.append(&chunk)).await?;
            offset = end;
            // Short of 1: the decode is still ahead.
            if !progress(0.95 * offset as f64 / data.len() as f64) {
                return Err(SftpError::Cancelled);
            }
        }
        let reply = run_ok(plan.finish.clone()).await?;
        if !String::from_utf8_lossy(&reply).contains(ExecUploadPlan::SUCCESS_MARKER) {
            return Err(SftpError::Failure {
                message: "the host didn't confirm the upload".to_owned(),
            });
        }
        Ok(())
    }
    .await;
    if let Err(error) = sent {
        // B330: the half-file used to stay on the host for good. A stop
        // answers at once and the removal follows; a failure waits for it.
        let (ssh, abandon) = (ssh.clone(), plan.abandon.clone());
        let removal = async move {
            let _ = ssh.run(&abandon).await;
        };
        if matches!(error, SftpError::Cancelled) {
            tokio::spawn(removal);
        } else {
            removal.await;
        }
        return Err(error);
    }
    progress(1.0);
    Ok(plan.final_path)
}

/// The home, after a mark line: sshd runs every exec command as
/// `$SHELL -c`, and bash reads `~/.bashrc` for that, so an rc file's `echo`
/// came ahead of `$HOME` and the exec upload refused it as no absolute path
/// (B463, the exec side).
const HOME_COMMAND: &str = "printf 'MOSHPIT_OUTPUT\\n%s' \"$HOME\"";

/// What follows the mark line; output without one, whole.
fn past_mark(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    match text.find("MOSHPIT_OUTPUT\n") {
        Some(at) => text[at + "MOSHPIT_OUTPUT\n".len()..].to_owned(),
        None => text.into_owned(),
    }
}

/// A command's merged output, if it exited 0 (what `executeCommand` gave);
/// `Cancelled` once `cancel` is.
async fn run_ok(
    ssh: &Ssh,
    command: String,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>, SftpError> {
    let output = match cancel {
        None => ssh.run(&command).await?,
        Some(cancel) => ssh
            .run_until(&command, cancel)
            .await?
            .ok_or(SftpError::Cancelled)?,
    };
    if output.success() {
        Ok(output.merged())
    } else {
        Err(SftpError::Failure {
            message: format!(
                "a command failed ({:?}): {}",
                output.exit,
                String::from_utf8_lossy(&output.merged()).trim()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strings `ImageAttachmentTests.execUploadPlan` pins.
    #[test]
    fn the_plan_is_the_apps_byte_for_byte() {
        let plan = ExecUploadPlan::new("/home/dev", "IMG-1.jpg");
        assert_eq!(plan.final_path, "/home/dev/.moshpit/uploads/IMG-1.jpg");
        assert_eq!(
            plan.begin,
            "mkdir -p '/home/dev/.moshpit/uploads' && chmod 700 '/home/dev/.moshpit' '/home/dev/.moshpit/uploads' && rm -f '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part'"
        );
        assert_eq!(
            plan.append("QUJD"),
            "printf %s 'QUJD' >> '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part'"
        );
        assert_eq!(
            plan.finish,
            "{ base64 -d < '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part' > '/home/dev/.moshpit/uploads/IMG-1.jpg' 2>/dev/null || base64 -D < '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part' > '/home/dev/.moshpit/uploads/IMG-1.jpg'; } && chmod 600 '/home/dev/.moshpit/uploads/IMG-1.jpg' && rm -f '/home/dev/.moshpit/uploads/IMG-1.jpg.b64part' && echo MOSHPIT_UPLOAD_OK"
        );
    }

    #[test]
    fn a_home_with_a_space_and_a_quote_survives() {
        let plan = ExecUploadPlan::new("/Users/dev o'brien", "IMG-1.jpg");
        assert!(
            plan.begin
                .contains(r"'/Users/dev o'\''brien/.moshpit/uploads'")
        );
        assert_eq!(
            plan.final_path,
            "/Users/dev o'brien/.moshpit/uploads/IMG-1.jpg"
        );
    }

    #[test]
    fn a_chunk_stays_inside_the_argument_budget() {
        let encoded = ExecUploadPlan::CHUNK_BYTES.div_ceil(3) * 4;
        assert!(encoded < 128 * 1024);
    }
}
