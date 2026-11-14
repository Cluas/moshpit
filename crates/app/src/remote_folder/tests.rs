//! The folder sheet's model, ported from `TerminalNSHSheetsTests`' folder
//! failure numbers (B286) and `RemoteFolderModel`'s own rules, which had no
//! Swift test of their own.

use super::*;

fn file(name: &str, size: Option<u64>, at: Option<f64>) -> Entry {
    Entry {
        path: format!("/srv/work/{name}"),
        is_directory: false,
        size,
        modified: at,
    }
}

fn dir(name: &str) -> Entry {
    Entry {
        path: format!("/srv/work/{name}"),
        is_directory: true,
        size: None,
        modified: None,
    }
}

fn listed() -> Folder {
    let mut folder = Folder::new("/srv/work");
    folder.listed(&[
        file("a.png", Some(10), Some(5.0)),
        file("huge.png", Some(THUMBNAIL_LIMIT + 1), Some(4.0)),
        file("clip.mov", Some(THUMBNAIL_LIMIT * 10), Some(3.0)),
        file("notes.md", Some(10), Some(2.0)),
        file(".hidden.png", Some(10), Some(9.0)),
        file("main.swift", Some(10), Some(9.0)),
        dir("plots"),
    ]);
    folder
}

/// B286: the same file failing again after its alert was answered — a
/// cleared failure set again in the same words before the page drew the
/// cleared state — was the same snapshot, never posted, never said.
#[test]
fn a_failure_set_anew_gets_a_new_number() {
    let mut folder = Folder::new("/srv/work");
    folder.set_open_failure(Some("The file is gone."));
    let first = folder.open_failure().unwrap().id;
    assert_ne!(first, 0);
    folder.set_open_failure(Some("The file is gone."));
    assert_eq!(
        folder.open_failure().unwrap().id,
        first,
        "the same failure still standing is not new"
    );
    folder.set_open_failure(None);
    assert_eq!(folder.open_failure(), None);
    folder.set_open_failure(Some("The file is gone."));
    let again = folder.open_failure().unwrap().id;
    assert!(
        again > first,
        "the same words failing again kept its number"
    );

    folder.set_edit_failure(Some(("Couldn't rename", "taken")));
    let edit = folder.edit_failure().unwrap().id;
    assert!(edit > again, "numbers are never reused, across kinds");
    folder.set_edit_failure(Some(("Couldn't rename", "taken")));
    assert_eq!(folder.edit_failure().unwrap().id, edit);
    folder.set_edit_failure(Some(("Couldn't delete", "taken")));
    assert_ne!(
        folder.edit_failure().unwrap().id,
        edit,
        "other words are a new failure"
    );
    let mut other = Folder::new("/elsewhere");
    other.set_open_failure(Some("x"));
    assert!(
        other.open_failure().unwrap().id > edit,
        "one count for the process"
    );
}

#[test]
fn the_folder_shows_the_grid_then_folders_then_files() {
    let mut folder = Folder::new("/srv/work");
    assert!(folder.loading());
    folder = listed();
    assert!(!folder.loading());
    let grid: Vec<String> = folder.media().iter().map(|e| e.name().to_owned()).collect();
    assert_eq!(grid, ["a.png", "huge.png", "clip.mov"]);
    let rows: Vec<String> = folder.rows().iter().map(|e| e.name().to_owned()).collect();
    assert_eq!(rows, ["plots", "notes.md"], "no dot files, no source");
    assert!(folder.entry("/srv/work/a.png").is_some());
    assert!(folder.entry("/srv/work/main.swift").is_none());

    folder.load_started();
    assert!(folder.loading());
    folder.list_failed("The host doesn't let this account read it.");
    assert!(!folder.loading());
    assert_eq!(
        folder.failure(),
        Some("The host doesn't let this account read it.")
    );
    folder.load_started();
    assert_eq!(folder.failure(), None, "a new listing clears it");
}

#[test]
fn thumbnails_are_fetched_for_video_and_small_images_once() {
    let mut folder = listed();
    let queue: Vec<String> = folder
        .thumbnail_queue()
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    assert_eq!(
        queue,
        ["a.png", "clip.mov"],
        "a big image waits to be opened"
    );
    folder.thumbnail_arrived("/srv/work/a.png");
    folder.cover_arrived("/srv/work/clip.mov", 12.5);
    assert!(folder.thumbnail_queue().is_empty());
    assert!(folder.has_thumbnail("/srv/work/clip.mov"));
    assert_eq!(folder.duration("/srv/work/clip.mov"), Some(12.5));
    let mut no_size = Folder::new("/r");
    no_size.listed(&[Entry {
        path: "/r/x.png".into(),
        is_directory: false,
        size: None,
        modified: None,
    }]);
    assert_eq!(
        no_size.thumbnail_queue().len(),
        1,
        "an unknown size counts as small"
    );
}

#[test]
fn progress_is_taken_only_while_under_way() {
    let mut folder = listed();
    folder.open_progress("/srv/work/a.png", 0.5);
    assert_eq!(folder.opening("/srv/work/a.png"), None, "not opened");
    folder.open_started("/srv/work/a.png");
    assert_eq!(folder.opening("/srv/work/a.png"), Some(0.0));
    folder.open_progress("/srv/work/a.png", 0.5);
    assert_eq!(folder.opening("/srv/work/a.png"), Some(0.5));
    folder.open_finished("/srv/work/a.png");
    folder.open_progress("/srv/work/a.png", 0.9);
    assert_eq!(folder.opening("/srv/work/a.png"), None, "a late report");

    folder.upload_progress(0.3);
    assert_eq!(folder.uploading(), None);
    folder.upload_started();
    folder.upload_progress(0.3);
    assert_eq!(folder.uploading(), Some(0.3));
    folder.upload_finished();
    folder.upload_progress(0.9);
    assert_eq!(folder.uploading(), None);
}

#[test]
fn names_are_checked_before_the_round_trip() {
    let mut folder = listed();
    assert_eq!(
        folder.new_folder_target("  out "),
        Ok("/srv/work/out".to_owned())
    );
    assert_eq!(folder.new_folder_target("plots"), Err(NameRefusal::Taken));
    assert_eq!(
        folder.new_folder_target(".hidden.png"),
        Err(NameRefusal::Taken),
        "hidden names are taken too"
    );
    assert_eq!(folder.new_folder_target("a/b"), Err(NameRefusal::Ignored));
    assert_eq!(folder.new_folder_target(" "), Err(NameRefusal::Ignored));
    let a = folder.entry("/srv/work/a.png").unwrap().clone();
    assert_eq!(
        folder.rename_target(&a, "a.png"),
        Err(NameRefusal::Ignored),
        "the name it has"
    );
    assert_eq!(
        folder.rename_target(&a, "notes.md"),
        Err(NameRefusal::Taken)
    );
    assert_eq!(
        folder.rename_target(&a, "b.png"),
        Ok("/srv/work/b.png".to_owned())
    );

    assert_eq!(
        folder.upload_target("a.png"),
        "/srv/work/a 2.png",
        "an upload never overwrites"
    );
    assert_eq!(
        folder.upload_target("a.png"),
        "/srv/work/a 3.png",
        "nor the one just sent"
    );
    assert_eq!(folder.upload_target("new.jpg"), "/srv/work/new.jpg");
    let mut root = Folder::new("/");
    root.listed(&[]);
    assert_eq!(root.upload_target("x.png"), "/x.png");
}

/// B332: names compare in one Unicode form. A folder holding "café"
/// decomposed (NFD) takes the composed name the phone writes (NFC) for a
/// new folder, a rename and an upload; a file renamed only into another
/// form meets itself, as a case-only rename does.
#[test]
fn names_in_another_unicode_form_are_the_same_name() {
    let (nfc, nfd) = ("caf\u{e9}.png", "cafe\u{301}.png");
    let mut folder = Folder::new("/srv/work");
    folder.listed(&[
        file(nfd, Some(1), Some(1.0)),
        file("a.png", Some(1), Some(1.0)),
    ]);
    assert_eq!(folder.new_folder_target(nfc), Err(NameRefusal::Taken));
    let a = folder.entry("/srv/work/a.png").unwrap().clone();
    assert_eq!(folder.rename_target(&a, nfc), Err(NameRefusal::Taken));
    assert_eq!(folder.upload_target(nfc), "/srv/work/caf\u{e9} 2.png");
    let cafe = folder.entry(&format!("/srv/work/{nfd}")).unwrap().clone();
    assert_eq!(
        folder.rename_target(&cafe, nfc),
        Ok(format!("/srv/work/{nfc}")),
        "the file meets itself"
    );
}

/// r42: a rename that only changes case is the file meeting itself, not a
/// name taken. Other names compare case-sensitively (most hosts are
/// Linux): "Notes.md" beside "notes.md" is a new name there.
#[test]
fn a_rename_that_only_changes_case_is_no_conflict() {
    let folder = listed();
    let a = folder.entry("/srv/work/a.png").unwrap().clone();
    assert_eq!(
        folder.rename_target(&a, "A.png"),
        Ok("/srv/work/A.png".to_owned())
    );
    assert_eq!(
        folder.rename_target(&a, "notes.md"),
        Err(NameRefusal::Taken)
    );
    assert_eq!(
        folder.rename_target(&a, "Notes.md"),
        Ok("/srv/work/Notes.md".to_owned()),
        "case-sensitive otherwise"
    );
}
