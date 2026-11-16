//! The remote-path model's rules, ported from `RemotePathsTests.swift`
//! (the parts after the scanner, which `crates/term` holds), with the
//! macOS-only runs of the probe and the exec edits run here for real
//! through zsh and sh.

use std::path::Path;
use std::process::Command;

use super::*;

fn entry(name: &str, dir: bool, at: Option<f64>) -> Entry {
    Entry {
        path: format!("/r/{name}"),
        is_directory: dir,
        size: Some(1),
        modified: at,
    }
}

#[test]
fn kinds_by_extension_any_case() {
    assert_eq!(kind("out/a.PNG"), Some(Kind::Image));
    assert_eq!(kind("clip.mov"), Some(Kind::Video));
    assert_eq!(kind("notes.md"), Some(Kind::Text));
    assert_eq!(kind("deck.key"), Some(Kind::Document));
    assert_eq!(kind("report.pdf"), Some(Kind::Pdf));
    assert_eq!(kind("song.flac"), Some(Kind::Audio));
    assert_eq!(kind("main.swift"), None, "source code is not worth a link");
    assert_eq!(kind("a."), None);
    assert_eq!(kind("v1.2/plots"), None, "a dot in a folder's name");
    assert_eq!(kind("Makefile"), None);
    let pattern = extension_pattern();
    assert!(
        pattern.starts_with("\\.(aac|aiff|avif|bmp|csv|doc|docx|"),
        "{pattern}"
    );
    assert!(pattern.ends_with("|webp|xls|xlsx)$"), "{pattern}");
}

#[test]
fn the_probes_output_maps_back_onto_its_batch_and_junk_is_ignored() {
    let out = "0\tf\t/repo/./out/a.png\n2\td\t/repo/plots/\n7\tf\t/nope\nbad line\n1\tx\t/repo/b\n3\tf\trelative\n";
    let hits = parse_probe(out, 4);
    assert_eq!(
        hits,
        BTreeMap::from([
            (
                0,
                Hit {
                    path: "/repo/out/a.png".into(),
                    is_directory: false
                }
            ),
            (
                2,
                Hit {
                    path: "/repo/plots".into(),
                    is_directory: true
                }
            ),
        ])
    );
    assert_eq!(parse_probe("-1\tf\t/a\n+1\tf\t/b\n-0\td\t/c\n", 4).len(), 2);
    assert_eq!(tidy("//repo/./out//a.png"), "/repo/out/a.png");
    assert_eq!(
        tidy("/repo/../x"),
        "/repo/../x",
        "`..` is the host's to resolve"
    );
}

fn candidates(line: &[(&str, bool)]) -> Vec<Candidate> {
    line.iter()
        .map(|(token, directory_only)| Candidate {
            token: (*token).to_owned(),
            directory_only: *directory_only,
        })
        .collect()
}

#[test]
fn every_candidate_is_one_quoted_argument_flagged_file_or_folder() {
    let batch = candidates(&[("out/a.png", false), ("out/plots", true)]);
    let cmd = probe_command(Some("/Users/me/it's here"), &batch);
    assert!(cmd.starts_with("sh -c '"));
    assert!(cmd.ends_with(" 'Fout/a.png' 'Dout/plots'"), "{cmd}");
    assert!(cmd.contains(r"'/Users/me/it'\''s here'"));
    // B138: a value ending in a backslash stays one word in fish too.
    let cmd = probe_command(Some(r"/Users/me/odd\"), &batch);
    assert!(cmd.contains(&quote(r"/Users/me/odd\")));
    assert!(cmd.contains(r"'/Users/me/odd'\\''"), "{cmd}");
    assert!(probe_command(None, &[]).contains(" moshpit-probe '' "));
}

fn shell(shell: &str, command: &str, home: &Path) -> String {
    let output = Command::new(shell)
        .arg("-c")
        .arg(command)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("the shell runs");
    String::from_utf8(output.stdout).expect("UTF-8")
}

fn scratch(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "remote-paths-{name}-{}",
        moshpit_seal::foundation::uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn shells() -> Vec<&'static str> {
    ["/bin/zsh", "/bin/sh"]
        .into_iter()
        .filter(|s| Path::new(s).exists())
        .collect()
}

#[test]
fn the_probe_resolves_searches_below_the_working_directory_and_filters_folders() {
    for sh in shells() {
        let root = scratch("probe");
        let files = [
            "repo/qchar/out/GripLab.png",
            "repo/v2/01_sheet.png",
            "repo/plots/a.png",
            "repo/src/main.swift",
            "repo/node_modules/x/y.png",
            "repo/out dir/c.png",
            "repo/episodes/ep/build/concept/v2/02_poses.png",
            "repo/old/b.png",
            "repo/new/b.png",
            // B743: one level past 1.0.8's six.
            "repo/a/b/c/d/e/f/07_deep.png",
        ];
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, [0x89]).unwrap();
        }
        // The old one an hour older than the new one.
        let status = Command::new("/usr/bin/touch")
            .args(["-t", "202001010000"])
            .arg(root.join("repo/old/b.png"))
            .status()
            .unwrap();
        assert!(status.success());
        let cwd = root.join("repo");
        let cwd_s = cwd.to_str().unwrap().to_owned();
        let absolute = format!("{cwd_s}/qchar/out/GripLab.png");
        let batch = candidates(&[
            ("qchar/out/GripLab.png", false),
            ("01_sheet.png", false),
            ("plots/", true),
            ("src/", true),
            ("missing.png", false),
            ("y.png", false),
            ("~/repo/plots/", true),
            (&absolute, false),
            ("build/concept/v2/", true),
            ("02_poses.png", false),
            ("b.png", false),
            ("07_deep.png", false),
        ]);
        let probe = |cwd: &str| {
            parse_probe(
                &shell(sh, &probe_command(Some(cwd), &batch), &root),
                batch.len(),
            )
        };
        let hit = |path: String, is_directory| Some(Hit { path, is_directory });
        let hits = probe(&cwd_s);
        assert_eq!(
            hits.get(&0).cloned(),
            hit(format!("{cwd_s}/qchar/out/GripLab.png"), false),
            "{sh}"
        );
        assert_eq!(
            hits.get(&1).cloned(),
            hit(format!("{cwd_s}/v2/01_sheet.png"), false),
            "a bare name written after `cd v2` is found below the working directory ({sh})"
        );
        assert_eq!(hits.get(&2).cloned(), hit(format!("{cwd_s}/plots"), true));
        assert_eq!(
            hits.get(&3),
            None,
            "a folder with nothing previewable is not a link"
        );
        assert_eq!(hits.get(&4), None);
        assert_eq!(hits.get(&5), None, "node_modules is never searched");
        assert_eq!(hits.get(&6).cloned(), hit(format!("{cwd_s}/plots"), true));
        assert_eq!(hits.get(&7).cloned(), hit(absolute.clone(), false));
        assert_eq!(
            hits.get(&8).cloned(),
            hit(format!("{cwd_s}/episodes/ep/build/concept/v2"), true),
            "a folder is searched for too"
        );
        assert_eq!(
            hits.get(&9).cloned(),
            hit(
                format!("{cwd_s}/episodes/ep/build/concept/v2/02_poses.png"),
                false
            ),
            "six levels down"
        );
        assert_eq!(
            hits.get(&10).cloned(),
            hit(format!("{cwd_s}/new/b.png"), false),
            "the newest match wins ({sh})"
        );
        assert_eq!(
            hits.get(&11),
            None,
            "seven levels down is past the search: up to six, as 1.0.8 ({sh})"
        );
        assert_eq!(SEARCH_DEPTH, 6);

        let from_home = probe(root.to_str().unwrap());
        assert_eq!(from_home.get(&1), None, "a pane in $HOME is never searched");
        assert_eq!(
            from_home.get(&6).cloned(),
            hit(format!("{cwd_s}/plots"), true)
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[test]
fn a_cjk_named_paths_starts_are_settled_by_the_host_on_folder_boundaries() {
    for sh in shells() {
        let root = scratch("probe-cjk");
        for file in ["repo/成品/西游-粗剪.mp4", "repo/episodes/ep/成品/粗剪.mp4"] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, [0]).unwrap();
        }
        let cwd = root.join("repo");
        let cwd_s = cwd.to_str().unwrap();
        // The scanner's runs for "输出到成品/西游-粗剪.mp4，已经成品/粗剪.mp4": every
        // start of a CJK head, longest first.
        let batch = candidates(&[
            ("到成品/西游-粗剪.mp4", false),
            ("成品/西游-粗剪.mp4", false),
            ("品/西游-粗剪.mp4", false),
            ("经成品/粗剪.mp4", false),
            ("成品/粗剪.mp4", false),
            ("品/粗剪.mp4", false),
        ]);
        let hits = parse_probe(
            &shell(sh, &probe_command(Some(cwd_s), &batch), &root),
            batch.len(),
        );
        let confirmed: Vec<(&str, String)> = batch
            .iter()
            .enumerate()
            .filter_map(|(i, c)| hits.get(&i).map(|h| (c.token.as_str(), h.path.clone())))
            .collect();
        assert_eq!(
            confirmed,
            [
                ("成品/西游-粗剪.mp4", format!("{cwd_s}/成品/西游-粗剪.mp4")),
                (
                    "成品/粗剪.mp4",
                    format!("{cwd_s}/episodes/ep/成品/粗剪.mp4")
                ),
            ],
            "\"品/…\" never matches inside \"成品/…\" ({sh})"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[test]
fn a_folder_lists_subfolders_and_previewable_files_newest_first_dot_files_hidden() {
    let shown = browsable(&[
        entry("old.png", false, Some(10.0)),
        entry("main.swift", false, Some(50.0)),
        entry(".hidden.png", false, Some(60.0)),
        entry("new.mp4", false, Some(40.0)),
        entry("plots", true, Some(20.0)),
        entry("undated.pdf", false, None),
    ]);
    let names: Vec<&str> = shown.iter().map(Entry::name).collect();
    assert_eq!(names, ["new.mp4", "plots", "old.png", "undated.pdf"]);
    let grid: Vec<String> = media(&shown).iter().map(|e| e.name().to_owned()).collect();
    assert_eq!(grid, ["new.mp4", "old.png"], "the grid: images and video");
    let list: Vec<String> = rows(&shown).iter().map(|e| e.name().to_owned()).collect();
    assert_eq!(
        list,
        ["plots", "undated.pdf"],
        "folders, then the other files"
    );
    let undated = browsable(&[
        entry("b.png", false, None),
        entry("a.png", false, None),
        entry("c.png", false, Some(1.0)),
    ]);
    let names: Vec<&str> = undated.iter().map(Entry::name).collect();
    assert_eq!(
        names,
        ["c.png", "b.png", "a.png"],
        "the undated keep their order, last"
    );
}

#[test]
fn the_title_menu_climbs_from_a_folder_to_the_root_nearest_first() {
    assert_eq!(
        enclosing_folders("/Users/c/code/x"),
        ["/Users/c/code", "/Users/c", "/Users", "/"]
    );
    assert_eq!(enclosing_folders("/tmp/out/"), ["/tmp", "/"]);
    assert!(enclosing_folders("/").is_empty());
    assert!(
        enclosing_folders("build/out").is_empty(),
        "only absolute paths"
    );
}

#[test]
fn the_fallback_stat_and_listing_parse_what_the_shell_prints() {
    assert!(parse_stat("d\n", "/r/plots").unwrap().is_directory);
    assert_eq!(
        parse_stat("f   1234\n", "/r/a.png").unwrap().size,
        Some(1234)
    );
    assert_eq!(parse_stat("", "/r/nope"), None);
    assert_eq!(parse_stat("f\n", "/r/a").unwrap().size, None);
    // Newest first; an entry without a time after the dated ones (B333's
    // NUL-separated records: kind, seconds or `-`, name).
    let listed = parse_list("f 100 a.png\0d 300.5 sub\0f - c.png\0f 200 b.png\0", "/r/");
    let paths: Vec<&str> = listed.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["/r/sub", "/r/b.png", "/r/a.png", "/r/c.png"]);
    let dirs: Vec<bool> = listed.iter().map(|e| e.is_directory).collect();
    assert_eq!(dirs, [true, false, false, false]);
    assert_eq!(listed[0].modified, Some(300.5));
    assert_eq!(listed[3].modified, None);
    assert!(parse_list("", "/r").is_empty(), "an empty folder");
}

#[test]
fn the_fallback_commands_run_against_a_real_folder() {
    let base = scratch("fallback");
    let root = base.join("it's here");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.png"), [1, 2, 3, 4, 5]).unwrap();
    let root_s = root.to_str().unwrap();
    let file = format!("{root_s}/a.png");
    let zsh = shells()[0];
    assert_eq!(
        parse_stat(&shell(zsh, &stat_command(&file), &base), &file)
            .unwrap()
            .size,
        Some(5)
    );
    assert!(
        parse_stat(&shell(zsh, &stat_command(root_s), &base), root_s)
            .unwrap()
            .is_directory
    );
    let names: BTreeSet<String> = parse_list(&shell(zsh, &list_command(root_s), &base), root_s)
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["a.png".to_owned(), "sub".to_owned()])
    );
    let encoded = shell(zsh, &cat_command(&file), &base);
    assert_eq!(encoded.trim(), "AQIDBAU=");
    std::fs::remove_dir_all(&base).ok();
}

/// B463, the exec side: bash reads `~/.bashrc` for sshd's `-c`, so an rc
/// file's `echo` comes ahead of every exec command's output. A host with
/// SFTP off read the noise as the stat's answer (the file "no longer on the
/// host") and took it into the listing's first entry. The readers print a
/// mark first, and what came before it is not theirs.
#[test]
fn the_fallback_reads_past_what_the_login_shell_prints() {
    let base = scratch("noisy");
    let root = base.join("out");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.png"), [1, 2, 3, 4, 5]).unwrap();
    let root_s = root.to_str().unwrap();
    let file = format!("{root_s}/a.png");
    for sh in shells() {
        let noisy = |command: &str| {
            shell(
                sh,
                &format!("echo 'Welcome to the build box! d'; {command}"),
                &base,
            )
        };
        let stat = parse_stat(&noisy(&stat_command(&file)), &file);
        assert_eq!(stat.and_then(|e| e.size), Some(5), "{sh}");
        let stat = parse_stat(&noisy(&stat_command(root_s)), root_s);
        assert!(stat.is_some_and(|e| e.is_directory), "{sh}");
        let listed = parse_list(&noisy(&list_command(root_s)), root_s);
        let names: BTreeSet<(String, bool)> = listed
            .iter()
            .map(|e| (e.name().to_owned(), e.is_directory))
            .collect();
        assert_eq!(
            names,
            BTreeSet::from([("a.png".to_owned(), false), ("sub".to_owned(), true)]),
            "{sh}"
        );
    }
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn names_are_trimmed_and_checked_uploads_never_overwrite_deletes_need_depth() {
    assert_eq!(valid_name("  plots ").as_deref(), Some("plots"));
    assert_eq!(valid_name(""), None);
    assert_eq!(valid_name(".."), None);
    assert_eq!(valid_name("."), None);
    assert_eq!(valid_name("a/b"), None);
    // B141: a `/` or NUL hidden in one character with a combining mark.
    assert_eq!(valid_name("a/\u{301}b"), None);
    assert_eq!(valid_name("x/\u{20DD}"), None);
    assert_eq!(valid_name("a\0\u{301}b"), None);
    assert_eq!(
        valid_name("e\u{301}t\u{E9}.md").as_deref(),
        Some("e\u{301}t\u{E9}.md")
    );
    let taken: BTreeSet<String> = [
        "shot.png",
        "shot 2.png",
        "Makefile",
        "archive.tar.gz",
        ".env",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(unique_name("new.png", &taken), "new.png");
    assert_eq!(unique_name("shot.png", &taken), "shot 3.png");
    assert_eq!(unique_name("Makefile", &taken), "Makefile 2");
    assert_eq!(unique_name("archive.tar.gz", &taken), "archive.tar 2.gz");
    assert_eq!(
        unique_name(".env", &taken),
        ".env 2",
        "a dot file's dot is no extension"
    );
    assert_eq!(child("a.png", "/r/out"), "/r/out/a.png");
    assert_eq!(child("a.png", "/r/out/"), "/r/out/a.png");
    assert!(deletable("/Users/me/out/a.png"));
    assert!(!deletable("/"));
    assert!(!deletable("/usr"));
    assert!(!deletable("relative/a.png"));
    assert!(!deletable("/Users/me/../.."));
    assert_eq!(remove_command("/", true), None);
}

#[test]
fn the_exec_edits_create_rename_refuse_a_taken_name_upload_and_delete() {
    for sh in shells() {
        let root = scratch("edits");
        let run = |command: &str| shell(sh, command, &root);
        let folder = format!("{}/it's plots", root.to_str().unwrap());
        assert!(run(&make_directory_command(&folder)).contains(EDIT_OK));
        assert!(Path::new(&folder).is_dir());

        let payload: Vec<u8> = (0..200_000u32)
            .map(|i| (i.wrapping_mul(13)) as u8)
            .collect();
        let file = format!("{folder}/a b.png");
        let plan = upload_plan(&file);
        assert!(run(&plan.begin).contains(EDIT_OK));
        for chunk in payload.chunks(48 * 1024) {
            run(&plan.append(&base64(chunk)));
        }
        assert!(run(&plan.finish).contains(EDIT_OK), "{sh}");
        assert_eq!(std::fs::read(&file).unwrap(), payload);
        assert!(!Path::new(&format!("{file}.b64part")).exists());
        assert!(
            run(&upload_plan(&file).begin).contains(EDIT_TAKEN),
            "an upload never starts over an existing file"
        );

        let taken = format!("{folder}/taken.png");
        std::fs::write(&taken, [1]).unwrap();
        assert!(run(&rename_command(&file, &taken)).contains(EDIT_TAKEN));
        assert_eq!(
            std::fs::read(&taken).unwrap(),
            [1],
            "a rename never overwrites"
        );
        let renamed = format!("{folder}/c.png");
        assert!(run(&rename_command(&file, &renamed)).contains(EDIT_OK));
        assert_eq!(std::fs::read(&renamed).unwrap(), payload);

        assert!(run(&remove_command(&renamed, false).unwrap()).contains(EDIT_OK));
        assert!(!Path::new(&renamed).exists());
        assert!(run(&remove_command(&folder, true).unwrap()).contains(EDIT_OK));
        assert!(!Path::new(&folder).exists());
        std::fs::remove_dir_all(&root).ok();
    }
}

/// B816: the folder sheet deletes any folder two levels deep, and the home
/// folder is one (`/Users/dev`, `/home/dev`): one Delete took it and
/// everything in it, and its parent the same way. The host refuses: never
/// the home folder, nor a folder it is in — by its name or its real path —
/// and says so; what is inside it goes as before.
#[test]
fn the_home_folder_and_the_folders_holding_it_are_never_deleted() {
    for sh in shells() {
        let base = scratch("home");
        let home = base.join("Users/dev");
        std::fs::create_dir_all(home.join("out")).unwrap();
        std::fs::write(home.join("keep.txt"), "x").unwrap();
        let real = std::fs::canonicalize(&home).unwrap();
        // A linked folder on the way to it.
        std::os::unix::fs::symlink(base.join("Users"), base.join("alias")).unwrap();
        let run = |command: &str| shell(sh, command, &home);
        let doomed = [
            home.to_str().unwrap().to_owned(),
            format!("{}/", home.display()),
            base.join("Users").to_str().unwrap().to_owned(),
            real.to_str().unwrap().to_owned(),
            real.parent().unwrap().to_str().unwrap().to_owned(),
            base.join("alias/dev").to_str().unwrap().to_owned(),
        ];
        for path in doomed {
            let said = run(&remove_command(&path, true).unwrap());
            assert!(said.contains(EDIT_HOME), "{sh}: {path}: {said:?}");
            assert!(!said.contains(EDIT_OK), "{sh}: {path}: {said:?}");
            assert!(
                home.join("keep.txt").exists(),
                "{sh}: deleting {path} took the home folder"
            );
        }
        let out = home.join("out");
        assert!(run(&remove_command(out.to_str().unwrap(), true).unwrap()).contains(EDIT_OK));
        assert!(!out.exists());
        std::fs::remove_dir_all(&base).ok();
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn a_host_path_survives_the_round_trip_through_the_link() {
    for path in [
        "/Users/me/out/a.png",
        "/tmp/with space/图.png",
        "/a?b#c.png",
        "/100%/x",
    ] {
        assert_eq!(
            link_path(&link(path)).as_deref(),
            Some(path),
            "{}",
            link(path)
        );
    }
    assert_eq!(
        link("/tmp/with space/a.png"),
        "moshpit-preview:///tmp/with%20space/a.png"
    );
    assert_eq!(
        link_path("file:///Users/me/out/a.png").as_deref(),
        Some("/Users/me/out/a.png")
    );
    assert_eq!(
        link_path("file://mac.local/Users/me/a.png").as_deref(),
        Some("/Users/me/a.png")
    );
    assert_eq!(
        link_path("  MOSHPIT-PREVIEW:///a.png\n").as_deref(),
        Some("/a.png")
    );
    assert_eq!(
        link_path("file://mac.local/Users/me/out/").as_deref(),
        Some("/Users/me/out"),
        "an OSC 7 directory, as Foundation reads it"
    );
    assert_eq!(link_path("file:///").as_deref(), Some("/"));
    assert_eq!(link_path("https://example.com/a.png"), None);
    assert_eq!(link_path("moshpit-preview:relative"), None);
    assert_eq!(link_path("not a link"), None);
}

/// r42: on a host whose disk ignores case (a Mac's APFS by default), the
/// exec rename of `a.png` to `A.png` found "A.png" there — the file itself
/// — and refused it as taken. The file being renamed is no conflict.
#[test]
fn an_exec_rename_that_only_changes_case_goes_through() {
    for sh in shells() {
        let root = scratch("case");
        let from = format!("{}/a.png", root.to_str().unwrap());
        let to = format!("{}/A.png", root.to_str().unwrap());
        std::fs::write(&from, [7]).unwrap();
        let said = shell(sh, &rename_command(&from, &to), &root);
        assert!(said.contains(EDIT_OK), "{sh}: {said}");
        let names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.png"], "{sh}");
        // Another file of that name is still taken.
        let other = format!("{}/b.png", root.to_str().unwrap());
        std::fs::write(&other, [8]).unwrap();
        let said = shell(sh, &rename_command(&other, &to), &root);
        assert!(said.contains(EDIT_TAKEN), "{sh}: {said}");
        assert_eq!(std::fs::read(&to).unwrap(), [7], "{sh}: a rename overwrote");
        std::fs::remove_dir_all(&root).ok();
    }
}

/// B331: the folder's exec upload (a host without SFTP) stopped or failed
/// part way left `<name>.b64part` in the person's folder, and a decode that
/// failed left a half-written `<name>` — which then made every retry say
/// the name was taken. A failed decode now leaves no `<name>`, and
/// `abandon` clears what an upload given up left.
#[test]
fn an_exec_upload_given_up_or_failed_leaves_nothing_in_the_folder() {
    for sh in shells() {
        let root = scratch("abandoned");
        let run = |command: &str| shell(sh, command, &root);
        let folder = root.to_str().unwrap().to_owned();

        // A decode that fails: not one byte of `<name>`.
        let file = format!("{folder}/broken.png");
        let plan = upload_plan(&file);
        assert!(run(&plan.begin).contains(EDIT_OK));
        // Valid bytes, then padding mid-stream: every decoder writes "ABC" and fails.
        run(&plan.append("QUJD=QUJD"));
        assert!(!run(&plan.finish).contains(EDIT_OK), "{sh}");
        assert!(
            !Path::new(&file).exists(),
            "{sh}: a half-decoded file stayed"
        );
        run(&plan.abandon);
        assert!(!Path::new(&format!("{file}.b64part")).exists(), "{sh}");
        assert!(
            run(&upload_plan(&file).begin).contains(EDIT_OK),
            "{sh}: a retry found the name taken"
        );
        run(&upload_plan(&file).abandon);

        // Given up between chunks: nothing left.
        let stopped = format!("{folder}/stopped.png");
        let plan = upload_plan(&stopped);
        assert!(run(&plan.begin).contains(EDIT_OK));
        run(&plan.append(&base64(&[1, 2, 3, 4, 5, 6])));
        run(&plan.abandon);
        assert!(!Path::new(&format!("{stopped}.b64part")).exists(), "{sh}");
        assert!(!Path::new(&stopped).exists(), "{sh}");
    }
}

/// B332: a name the phone writes composed (NFC: "é" one scalar) and the
/// host holds decomposed (NFD: "e" + U+0301, as an old Mac disk keeps
/// it), or the other way, is the same name to a person — and to a disk
/// that normalises. `unique_name` took them for two names, so the upload
/// went for the taken one and failed "name taken" instead of becoming
/// "café 2.png". What is made keeps the bytes the person gave.
#[test]
fn a_name_taken_in_another_unicode_form_is_taken() {
    let (nfc, nfd) = ("caf\u{e9}.png", "cafe\u{301}.png");
    let host_nfd: BTreeSet<String> = [nfd.to_owned()].into();
    assert_eq!(unique_name(nfc, &host_nfd), "caf\u{e9} 2.png");
    let host_nfc: BTreeSet<String> = [nfc.to_owned()].into();
    assert_eq!(unique_name(nfd, &host_nfc), "cafe\u{301} 2.png");
    assert_eq!(unique_name(nfc, &BTreeSet::new()), nfc, "the bytes given");
}

/// B333: the exec listing (a host without SFTP) read `ls -1Apt` a line at
/// a time. A name with a line break came back as two entries that aren't
/// there, and `-p` marks only real folders, so a link to a folder listed
/// as a file and opened as one. The listing is NUL-separated now, with
/// each entry's kind taken through links and its modified time.
#[test]
fn the_exec_listing_keeps_odd_names_whole_and_follows_links_to_folders() {
    for sh in shells() {
        let root = scratch("listing");
        let folder = root.join("it's \"plots\"");
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join("two\nlines.png"), [1]).unwrap();
        std::fs::write(folder.join("a b.png"), [1]).unwrap();
        std::fs::write(folder.join(".hidden"), [1]).unwrap();
        std::os::unix::fs::symlink(folder.join("sub"), folder.join("linked")).unwrap();
        std::os::unix::fs::symlink(folder.join("nowhere"), folder.join("dangling")).unwrap();
        let path = folder.to_str().unwrap();

        let output = shell(sh, &list_command(path), &root);
        let mut entries = parse_list(&output, path);
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let seen: Vec<(String, bool)> = entries
            .iter()
            .map(|e| (e.name().to_owned(), e.is_directory))
            .collect();
        let want = |name: &str, dir: bool| (name.to_owned(), dir);
        let mut expected = vec![
            want(".hidden", false),
            want("a b.png", false),
            want("dangling", false),
            want("linked", true),
            want("sub", true),
            want("two\nlines.png", false),
        ];
        expected.sort();
        let mut got = seen.clone();
        got.sort();
        assert_eq!(got, expected, "{sh}: {output:?}");
        assert!(
            entries.iter().all(|e| e.path.starts_with(path)),
            "{sh}: {entries:?}"
        );
    }
}
