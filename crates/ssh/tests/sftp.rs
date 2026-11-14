//! SFTP as the remote preview, uploads and the folder sheet use it: the
//! home, stat, listings, range reads, windowed downloads, exclusive
//! uploads, the uploads directory (and its exec fallback), mkdir, rename
//! without overwriting, remove.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use moshpit_ssh::{ExecUploadPlan, SftpError};
use support::*;

fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn the_home_is_where_relative_paths_start() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let sftp = ssh.sftp().await.unwrap();
    assert_eq!(
        sftp.home().await.unwrap(),
        host.home.path().to_string_lossy()
    );
}

#[tokio::test]
async fn stat_says_what_a_path_is() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("notes.txt"), b"twelve bytes").unwrap();
    std::fs::create_dir(host.home.join("shots")).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();

    let file = sftp
        .stat(&host.home.join("notes.txt").to_string_lossy())
        .await
        .unwrap();
    assert!(!file.is_directory);
    assert_eq!(file.size, Some(12));
    assert!(file.modified.is_some());

    let dir = sftp.stat("shots").await.unwrap();
    assert_eq!(dir.path, "shots");
    assert!(dir.is_directory);

    assert!(matches!(
        sftp.stat("missing").await,
        Err(SftpError::NotFound(p)) if p == "missing"
    ));
}

#[tokio::test]
async fn a_listing_drops_dot_entries_and_joins_paths() {
    let host = Host::start(Bench::default()).await;
    let dir = host.home.join("project");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("a.png"), b"png").unwrap();
    std::fs::write(dir.join("b.mp4"), vec![0u8; 4096]).unwrap();
    std::fs::create_dir(dir.join("src")).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let base = dir.to_string_lossy().into_owned();

    for asked in [base.clone(), format!("{base}/")] {
        let mut entries = sftp.list(&asked).await.unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let summary: Vec<(String, bool, Option<u64>)> = entries
            .iter()
            .map(|e| {
                (
                    e.path.clone(),
                    e.is_directory,
                    e.size.filter(|_| !e.is_directory),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (format!("{base}/a.png"), false, Some(3)),
                (format!("{base}/b.mp4"), false, Some(4096)),
                (format!("{base}/src"), true, None),
            ],
            "{asked}"
        );
    }
    assert!(matches!(
        sftp.list("nowhere").await,
        Err(SftpError::NotFound(_))
    ));
}

/// B460: SFTP's readdir reports a symlink as the link itself (OpenSSH's
/// sftp-server lstats each entry), and the listing took that as the entry:
/// a link to a folder (`~/code -> /data/code`) read as a file with no
/// extension, and the folder sheet, which shows folders and previewable
/// files, dropped it — on every host with SFTP. The exec listing follows
/// links (B333). A link is now read as what it points at; one pointing
/// nowhere stays as the listing said.
#[tokio::test]
async fn a_listing_reads_a_link_as_what_it_points_at() {
    let host = Host::start(Bench::default()).await;
    let dir = host.home.join("project");
    std::fs::create_dir_all(dir.join("real")).unwrap();
    std::fs::write(dir.join("real/a.png"), b"png").unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("linked")).unwrap();
    std::os::unix::fs::symlink(dir.join("real/a.png"), dir.join("pic.png")).unwrap();
    std::os::unix::fs::symlink(dir.join("missing"), dir.join("dangling")).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let base = dir.to_string_lossy().into_owned();

    let entries = sftp.list(&base).await.unwrap();
    let entry = |name: &str| {
        entries
            .iter()
            .find(|e| e.path == format!("{base}/{name}"))
            .unwrap_or_else(|| panic!("no {name} in {entries:?}"))
            .clone()
    };
    assert!(entry("real").is_directory);
    assert!(
        entry("linked").is_directory,
        "a link to a folder is a folder"
    );
    assert!(!entry("pic.png").is_directory);
    assert_eq!(
        entry("pic.png").size,
        Some(3),
        "a link to a file has its size"
    );
    assert!(!entry("dangling").is_directory);
    assert_eq!(host.seen().open_handles, 0);
}

/// Listing again and again on one session doesn't run the host out of
/// handles: each directory handle is closed.
#[tokio::test]
async fn listings_close_their_handles() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("x"), b"x").unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    for _ in 0..300 {
        assert_eq!(sftp.list(".").await.unwrap().len(), 1);
    }
    assert_eq!(host.seen().open_handles, 0);
    assert!(sftp.list("missing").await.is_err());
    assert_eq!(host.seen().open_handles, 0);
}

#[tokio::test]
async fn an_open_file_reads_any_range() {
    let host = Host::start(Bench::default()).await;
    let bytes = noise(1_000_000, 1);
    std::fs::write(host.home.join("clip.mp4"), &bytes).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let file = Arc::new(sftp.open("clip.mp4").await.unwrap());
    assert_eq!(file.size(), 1_000_000);

    assert_eq!(file.read(0, 16).await.unwrap(), bytes[..16]);
    assert_eq!(file.read(999_990, 64).await.unwrap(), bytes[999_990..]);
    assert!(file.read(1_000_000, 64).await.unwrap().is_empty());
    assert!(file.read(5_000_000, 64).await.unwrap().is_empty());

    // Overlapping reads, each its own request.
    let mut reads = tokio::task::JoinSet::new();
    for i in 0..12u64 {
        let file = file.clone();
        reads.spawn(async move { (i, file.read(i * 80_000, 65_536).await.unwrap()) });
    }
    while let Some(done) = reads.join_next().await {
        let (i, got) = done.unwrap();
        let at = (i * 80_000) as usize;
        assert_eq!(got, bytes[at..at + 65_536]);
    }
    assert_eq!(host.seen().open_handles, 1);
    Arc::into_inner(file).unwrap().close().await;
    assert_eq!(host.seen().open_handles, 0);
}

/// A playback given up without a close still lets the handle go.
#[tokio::test]
async fn a_dropped_file_lets_its_handle_go() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("clip.mov"), noise(10_000, 9)).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let file = sftp.open("clip.mov").await.unwrap();
    assert_eq!(file.read(0, 10).await.unwrap().len(), 10);
    assert_eq!(host.seen().open_handles, 1);
    drop(file);
    within(5, async {
        while host.seen().open_handles != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
}

#[tokio::test]
async fn an_empty_file_is_not_opened_for_playback() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("empty.mov"), b"").unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    assert!(matches!(
        sftp.open("empty.mov").await,
        Err(SftpError::NotFound(_))
    ));
    assert!(matches!(
        sftp.open("absent.mov").await,
        Err(SftpError::NotFound(_))
    ));
    assert_eq!(host.seen().open_handles, 0);
}

#[tokio::test]
async fn a_download_arrives_whole_with_progress() {
    let host = Host::start(Bench::default()).await;
    let bytes = noise(3 * 1024 * 1024 + 4321, 2);
    std::fs::write(host.home.join("big.bin"), &bytes).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let local = Scratch::new("download");
    let destination = local.join("big.bin");
    let mut fractions = Vec::new();
    let copied = sftp
        .download("big.bin", &destination, &mut |f| {
            fractions.push(f);
            true
        })
        .await
        .unwrap();
    assert_eq!(copied, bytes.len() as u64);
    assert!(std::fs::read(&destination).unwrap() == bytes);
    assert!(fractions.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(fractions.last(), Some(&1.0));
    assert_eq!(host.seen().open_handles, 0);
}

/// B463: sshd starts the SFTP server through the account's shell, and an
/// rc file that prints for every shell (bash reads `~/.bashrc` for sshd's
/// `-c`) puts its text in front of the server's greeting. That text read as
/// a packet length far past the limit, SFTP failed, and every preview fell
/// back to exec, whose output carries the same text.
#[tokio::test]
async fn a_shell_that_prints_before_sftp_starts_doesnt_break_it() {
    let host = Host::start(Bench {
        sftp_noise: b"Welcome to the build box!\r\nLast deploy: today\n".to_vec(),
        ..Bench::default()
    })
    .await;
    let bytes = noise(300_000, 7);
    std::fs::create_dir(host.home.join("out")).unwrap();
    std::fs::write(host.home.join("out/a.png"), &bytes).unwrap();
    let sftp = connect(&host)
        .await
        .sftp()
        .await
        .expect("SFTP past the noise");
    assert!(sftp.stat("out").await.unwrap().is_directory);
    let local = Scratch::new("noisy-download");
    let destination = local.join("a.png");
    let copied = sftp
        .download("out/a.png", &destination, &mut |_| true)
        .await
        .unwrap();
    assert_eq!(copied, bytes.len() as u64);
    assert!(std::fs::read(&destination).unwrap() == bytes);
}

#[tokio::test]
async fn a_download_can_be_stopped() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("big.bin"), noise(2 * 1024 * 1024, 3)).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let local = Scratch::new("stopped");
    let calls = AtomicUsize::new(0);
    let err = sftp
        .download("big.bin", &local.join("big.bin"), &mut |_| {
            calls.fetch_add(1, Ordering::SeqCst) < 3
        })
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SftpError::Cancelled));
    // The session is still good, and the file was let go.
    assert!(sftp.stat("big.bin").await.is_ok());
    assert_eq!(host.seen().open_handles, 0);
}

#[tokio::test]
async fn an_upload_never_overwrites() {
    let host = Host::start(Bench::default()).await;
    let local = Scratch::new("upload");
    let bytes = noise(1_234_567, 4);
    std::fs::write(local.join("photo.heic"), &bytes).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();

    let mut last = 0.0;
    sftp.upload_new(&local.join("photo.heic"), "photo.heic", &mut |f| {
        assert!(f >= last);
        last = f;
        true
    })
    .await
    .unwrap();
    assert_eq!(last, 1.0);
    assert!(std::fs::read(host.home.join("photo.heic")).unwrap() == bytes);

    std::fs::write(local.join("other"), b"other").unwrap();
    let err = sftp
        .upload_new(&local.join("other"), "photo.heic", &mut |_| true)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SftpError::NameTaken(p) if p == "photo.heic"));
    assert!(std::fs::read(host.home.join("photo.heic")).unwrap() == bytes);
}

/// A stopped upload takes back what it wrote.
#[tokio::test]
async fn a_stopped_upload_leaves_nothing_behind() {
    let host = Host::start(Bench::default()).await;
    let local = Scratch::new("partial");
    std::fs::write(local.join("big"), noise(4 * 1024 * 1024, 5)).unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let mut calls = 0;
    let err = sftp
        .upload_new(&local.join("big"), "big", &mut |_| {
            calls += 1;
            calls < 4
        })
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SftpError::Cancelled));
    assert!(!host.home.join("big").exists());
    assert_eq!(host.seen().open_handles, 0);
}

#[tokio::test]
async fn an_upload_to_a_missing_folder_says_so() {
    let host = Host::start(Bench::default()).await;
    let local = Scratch::new("missing-parent");
    std::fs::write(local.join("f"), b"f").unwrap();
    let sftp = connect(&host).await.sftp().await.unwrap();
    let err = sftp
        .upload_new(&local.join("f"), "no/such/dir/f", &mut |_| true)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SftpError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn folders_are_made_and_names_are_never_taken_over() {
    let host = Host::start(Bench::default()).await;
    let sftp = connect(&host).await.sftp().await.unwrap();
    sftp.create_dir("drafts").await.unwrap();
    assert!(host.home.join("drafts").is_dir());
    assert!(matches!(
        sftp.create_dir("drafts").await,
        Err(SftpError::Failure { .. })
    ));

    std::fs::write(host.home.join("a.txt"), b"a").unwrap();
    std::fs::write(host.home.join("b.txt"), b"b").unwrap();
    assert!(matches!(
        sftp.rename("a.txt", "b.txt").await,
        Err(SftpError::NameTaken(p)) if p == "b.txt"
    ));
    assert_eq!(std::fs::read(host.home.join("b.txt")).unwrap(), b"b");
    sftp.rename("a.txt", "drafts/a.txt").await.unwrap();
    assert_eq!(std::fs::read(host.home.join("drafts/a.txt")).unwrap(), b"a");

    sftp.remove_file("b.txt").await.unwrap();
    assert!(!host.home.join("b.txt").exists());
    assert!(matches!(
        sftp.remove_file("b.txt").await,
        Err(SftpError::NotFound(_))
    ));
}

/// r42: a new name that only changes the case of the old is not taken by
/// the file itself. On a disk that ignores case (a Mac's, this bench's on
/// macOS) the new name answers for the old, and the host refuses to
/// rename onto it; the rename goes through a free name first. On a disk
/// that keeps case (CI's), a different file of that name is still never
/// taken over.
#[tokio::test]
async fn a_rename_that_only_changes_case_goes_through() {
    let host = Host::start(Bench::default()).await;
    let sftp = connect(&host).await.sftp().await.unwrap();
    let names = |dir: &str| {
        let mut names: Vec<String> = std::fs::read_dir(host.home.join(dir))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    };

    std::fs::create_dir(host.home.join("case")).unwrap();
    std::fs::write(host.home.join("case/a.png"), b"a").unwrap();
    sftp.rename("case/a.png", "case/A.png").await.unwrap();
    assert_eq!(names("case"), ["A.png"]);
    assert_eq!(std::fs::read(host.home.join("case/A.png")).unwrap(), b"a");
    std::fs::create_dir(host.home.join("case/drafts")).unwrap();
    sftp.rename("case/drafts", "case/Drafts").await.unwrap();
    assert_eq!(names("case"), ["A.png", "Drafts"]);

    // Another file is still taken: in another case only where the disk
    // keeps case.
    std::fs::write(host.home.join("case/b.png"), b"b").unwrap();
    assert!(matches!(
        sftp.rename("case/b.png", "case/A.png").await,
        Err(SftpError::NameTaken(p)) if p == "case/A.png"
    ));
    std::fs::create_dir(host.home.join("kept")).unwrap();
    std::fs::write(host.home.join("kept/a.png"), b"a").unwrap();
    std::fs::write(host.home.join("kept/A.png"), b"other").unwrap();
    if names("kept").len() == 2 {
        assert!(matches!(
            sftp.rename("kept/a.png", "kept/A.png").await,
            Err(SftpError::NameTaken(p)) if p == "kept/A.png"
        ));
        assert_eq!(names("kept"), ["A.png", "a.png"]);
        assert_eq!(std::fs::read(host.home.join("kept/a.png")).unwrap(), b"a");
        assert_eq!(
            std::fs::read(host.home.join("kept/A.png")).unwrap(),
            b"other"
        );
    }
    assert_eq!(names("case"), ["A.png", "Drafts", "b.png"]);
    assert_eq!(std::fs::read(host.home.join("case/A.png")).unwrap(), b"a");
}

/// Screenshots for an agent land in `~/.moshpit/uploads`, the folder 700,
/// the file 600; the same name uploaded again is replaced.
#[tokio::test]
async fn the_uploads_directory_is_made_private() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let image = noise(300_000, 6);
    let mut last = 0.0;
    let path = ssh
        .upload_to_uploads_directory(&image, "IMG-1.png", &mut |f| {
            last = f;
            true
        })
        .await
        .unwrap();
    let uploads = host.home.join(".moshpit/uploads");
    assert_eq!(path, uploads.join("IMG-1.png").to_string_lossy());
    assert!(std::fs::read(&path).unwrap() == image);
    assert_eq!(last, 1.0);
    assert_eq!(mode(&host.home.join(".moshpit")), 0o700);
    assert_eq!(mode(&uploads), 0o700);
    assert_eq!(mode(std::path::Path::new(&path)), 0o600);

    ssh.upload_to_uploads_directory(b"short", "IMG-1.png", &mut |_| true)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"short");
    assert!(host.seen().commands.is_empty(), "no exec on an SFTP host");
}

/// A host without the SFTP subsystem still takes uploads, over exec
/// channels and the app's exact commands, and the connection stops trying
/// SFTP for them.
#[tokio::test]
async fn without_sftp_uploads_go_over_exec() {
    let host = Host::start(Bench {
        sftp: false,
        shell_commands: true,
        ..Default::default()
    })
    .await;
    let ssh = connect(&host).await;
    assert!(matches!(ssh.sftp().await, Err(SftpError::Unavailable(_))));

    let image = noise(ExecUploadPlan::CHUNK_BYTES * 2 + 1000, 7);
    let mut fractions = Vec::new();
    let path = ssh
        .upload_to_uploads_directory(&image, "IMG-2.jpg", &mut |f| {
            fractions.push(f);
            true
        })
        .await
        .unwrap();
    let home = host.home.path().to_string_lossy().into_owned();
    assert_eq!(path, format!("{home}/.moshpit/uploads/IMG-2.jpg"));
    assert!(std::fs::read(&path).unwrap() == image);
    assert_eq!(mode(std::path::Path::new(&path)), 0o600);
    assert_eq!(fractions.last(), Some(&1.0));
    assert!(fractions[..fractions.len() - 1].iter().all(|f| *f <= 0.95));

    let plan = ExecUploadPlan::new(&home, "IMG-2.jpg");
    let commands = host.seen().commands.clone();
    assert_eq!(
        commands.first().map(String::as_str),
        Some("printf 'MOSHPIT_OUTPUT\\n%s' \"$HOME\"")
    );
    assert_eq!(commands[1], plan.begin);
    assert_eq!(commands.len(), 2 + 3 + 1);
    assert_eq!(commands.last(), Some(&plan.finish));
    assert!(!std::path::Path::new(&format!("{path}.b64part")).exists());

    // Remembered: the next upload goes straight to exec.
    ssh.upload_to_uploads_directory(b"again", "IMG-3.jpg", &mut |_| true)
        .await
        .unwrap();
    assert_eq!(
        host.seen().commands[6],
        "printf 'MOSHPIT_OUTPUT\\n%s' \"$HOME\""
    );
}

/// B463, the exec side: bash reads `~/.bashrc` for sshd's `-c`, so an rc
/// file's `echo` comes ahead of every exec command's output. The exec
/// upload read the noise as part of `$HOME`, refused it as no absolute
/// path, and the picture never went up. The home is read past a mark.
#[tokio::test]
async fn a_shell_that_prints_first_doesnt_break_the_exec_upload() {
    let host = Host::start(Bench {
        sftp: false,
        shell_commands: true,
        exec_noise: b"Welcome to the build box!\n".to_vec(),
        ..Default::default()
    })
    .await;
    let ssh = connect(&host).await;
    let image = noise(5000, 3);
    let path = ssh
        .upload_to_uploads_directory(&image, "IMG-4.jpg", &mut |_| true)
        .await
        .expect("an upload past the noise");
    let home = host.home.path().to_string_lossy().into_owned();
    assert_eq!(path, format!("{home}/.moshpit/uploads/IMG-4.jpg"));
    assert!(std::fs::read(&path).unwrap() == image);
}

/// Waits up to 3 s for `path` to be gone (the clean-up after a stop does
/// not hold the stop's answer up).
async fn gone(path: &std::path::Path) -> bool {
    for _ in 0..60 {
        if !path.exists() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    !path.exists()
}

/// B330: an upload into the uploads directory stopped part way left what it
/// had written on the host, under the name an agent is given — a
/// truncated image. Now it goes.
#[tokio::test]
async fn a_stopped_upload_to_the_uploads_directory_leaves_nothing_behind() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let image = noise(2 * 1024 * 1024, 11);
    let mut calls = 0;
    let err = ssh
        .upload_to_uploads_directory(&image, "IMG-stopped.png", &mut |_| {
            calls += 1;
            calls < 3
        })
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SftpError::Cancelled), "{err:?}");
    let path = host.home.join(".moshpit/uploads/IMG-stopped.png");
    assert!(gone(&path).await, "a partial upload stayed on the host");
}

/// B330, without SFTP: the exec upload stopped part way left its base64
/// half-file (`.b64part`) on the host for good.
#[tokio::test]
async fn a_stopped_upload_over_exec_leaves_nothing_behind() {
    let host = Host::start(Bench {
        sftp: false,
        shell_commands: true,
        ..Default::default()
    })
    .await;
    let ssh = connect(&host).await;
    let image = noise(ExecUploadPlan::CHUNK_BYTES * 3, 13);
    let mut calls = 0;
    let err = ssh
        .upload_to_uploads_directory(&image, "IMG-stopped.jpg", &mut |_| {
            calls += 1;
            calls < 2
        })
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SftpError::Cancelled), "{err:?}");
    let path = host.home.join(".moshpit/uploads/IMG-stopped.jpg");
    let part = host.home.join(".moshpit/uploads/IMG-stopped.jpg.b64part");
    assert!(gone(&part).await, "the half-file stayed on the host");
    assert!(gone(&path).await);
}

/// The preview session is kept between operations, and a fresh one is
/// opened once it has died.
#[tokio::test]
async fn the_preview_session_is_reused() {
    let host = Host::start(Bench::default()).await;
    std::fs::write(host.home.join("f"), b"f").unwrap();
    let ssh = connect(&host).await;
    let first = ssh.sftp().await.unwrap();
    first.stat("f").await.unwrap();
    let again = ssh.sftp().await.unwrap();
    again.stat("f").await.unwrap();
    first.close();
    let fresh = ssh.sftp().await.unwrap();
    assert!(fresh.is_alive());
    fresh.stat("f").await.unwrap();
}
