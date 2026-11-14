//! The attachment flow's rules, ported from `ImageAttachmentTests.swift`'s
//! paste-text, style and log parts, and the controller's orchestration
//! (which had no Swift test of its own).

use super::*;

fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn clean_paths_stay_bare_and_others_are_quoted() {
    assert_eq!(
        insert_text(
            &paths(&["/home/dev/.moshpit/uploads/IMG-1.jpg"]),
            InsertStyle::BarePath
        ),
        " /home/dev/.moshpit/uploads/IMG-1.jpg "
    );
    assert_eq!(
        insert_text(
            &paths(&["/home/dev user/.moshpit/uploads/IMG-1.jpg"]),
            InsertStyle::BarePath
        ),
        " '/home/dev user/.moshpit/uploads/IMG-1.jpg' "
    );
    assert_eq!(
        insert_text(&paths(&["/a/it's.jpg"]), InsertStyle::BarePath),
        r" '/a/it'\''s.jpg' "
    );
    assert_eq!(
        insert_text(&paths(&["/a/1.jpg", "/a/2.png"]), InsertStyle::BarePath),
        " /a/1.jpg /a/2.png "
    );
    assert_eq!(insert_text(&[], InsertStyle::BarePath), "");
    // A space with a combining mark on it is still a space (by scalar).
    assert_eq!(
        insert_text(&paths(&["/a/x \u{301}y.jpg"]), InsertStyle::BarePath),
        " '/a/x \u{301}y.jpg' "
    );
}

#[test]
fn the_style_follows_the_agent() {
    assert_eq!(InsertStyle::for_agent(None), InsertStyle::BarePath);
    assert_eq!(InsertStyle::for_agent(Some("")), InsertStyle::BarePath);
    for bare in ["zsh", "claude", "2.1.227", "codex", "goose"] {
        assert_eq!(
            InsertStyle::for_agent(Some(bare)),
            InsertStyle::BarePath,
            "{bare}"
        );
    }
    for at in ["gemini", "qwen", "Gemini-CLI", "qwen-code"] {
        assert_eq!(
            InsertStyle::for_agent(Some(at)),
            InsertStyle::AtMention,
            "{at}"
        );
    }
    assert_eq!(InsertStyle::for_agent(Some("aider")), InsertStyle::AiderAdd);
    assert_eq!(InsertStyle::for_agent(Some("1.2")), InsertStyle::BarePath);
}

#[test]
fn at_mentions_prefix_clean_paths_and_fall_back_on_quoted_ones() {
    assert_eq!(
        insert_text(&paths(&["/a/1.jpg"]), InsertStyle::AtMention),
        " @/a/1.jpg "
    );
    assert_eq!(
        insert_text(&paths(&["/a b/1.jpg"]), InsertStyle::AtMention),
        " '/a b/1.jpg' "
    );
    assert_eq!(
        insert_text(&paths(&["/a/1.jpg", "/a/2.png"]), InsertStyle::AiderAdd),
        "/add /a/1.jpg /a/2.png",
        "aider leads with /add, no padding"
    );
}

#[test]
fn the_upload_log_numbers_from_one_in_order() {
    let mut log = UploadLog::default();
    assert_eq!(log.record("/u/IMG-a.jpg"), 1);
    assert_eq!(log.record("/u/IMG-b.png"), 2);
    assert_eq!(log.paths(), ["/u/IMG-a.jpg", "/u/IMG-b.png"]);
}

#[test]
fn uploads_go_one_by_one_progress_never_goes_back_and_the_paste_waits() {
    let mut log = UploadLog::default();
    log.record("/u/earlier.jpg");
    let mut flow = Attachments::new();
    assert_eq!(flow.phase(), &Phase::Processing);
    assert_eq!(flow.next(), None, "nothing until processed");
    flow.processed(vec![100, 300]);
    assert_eq!(flow.next(), Some(0));
    assert_eq!(flow.progress(0, 0.5), Some(0.125));
    assert_eq!(
        flow.progress(0, 0.25),
        Some(0.125),
        "a late report never goes back"
    );
    assert_eq!(flow.insert_text(InsertStyle::BarePath), None);
    assert!(!flow.landed(0, "/u/a.jpg"));
    assert_eq!(flow.phase(), &Phase::Uploading(0.25));
    assert_eq!(flow.next(), Some(1));
    assert_eq!(flow.progress(1, 1.0 / 3.0), Some(0.5));
    assert!(flow.landed(1, "/u/b.jpg"), "the last one");
    assert_eq!(flow.phase(), &Phase::Ready);
    assert_eq!(flow.next(), None);
    flow.numbered(&[7]);
    assert_eq!(
        flow.numbers(),
        [None, None],
        "one number for each file or none"
    );
    flow.number_in(&mut log);
    assert_eq!(
        flow.numbers(),
        [Some(2), Some(3)],
        "numbered after the session's earlier ones"
    );
    assert_eq!(
        flow.insert_text(InsertStyle::BarePath).as_deref(),
        Some(" /u/a.jpg /u/b.jpg ")
    );
}

#[test]
fn a_retry_uploads_only_what_did_not_land() {
    let mut log = UploadLog::default();
    let mut flow = Attachments::new();
    flow.processed(vec![100, 100, 200]);
    assert_eq!(flow.next(), Some(0));
    flow.landed(0, "/u/a.jpg");
    assert_eq!(flow.next(), Some(1));
    flow.failed("the connection dropped");
    assert_eq!(
        flow.phase(),
        &Phase::Failed("the connection dropped".into())
    );
    assert_eq!(flow.next(), None, "a failed flow uploads nothing");
    assert_eq!(flow.insert_text(InsertStyle::BarePath), None);
    assert!(flow.retry());
    assert!(!flow.retry(), "only after a failure");
    assert_eq!(flow.next(), Some(1), "the landed file is skipped");
    assert_eq!(flow.phase(), &Phase::Uploading(0.25), "and counted as done");
    assert!(!flow.landed(1, "/u/b.jpg"));
    assert!(flow.landed(2, "/u/c.jpg"));
    assert_eq!(flow.phase(), &Phase::Ready);
    flow.number_in(&mut log);
    assert_eq!(log.paths().len(), 3, "each file numbered once");
    assert_eq!(flow.numbers(), [Some(1), Some(2), Some(3)]);

    let mut early = Attachments::new();
    early.failed("can't read that photo");
    assert!(
        !early.retry(),
        "nothing was processed: the host processes again"
    );
}

/// r42: an empty batch went to `Uploading(0.0)` and stayed there, since
/// only `landed` makes the flow ready. Swift's controller went to `.ready`.
#[test]
fn an_empty_batch_is_ready_with_nothing_to_paste() {
    let mut flow = Attachments::new();
    flow.processed(vec![]);
    assert_eq!(flow.phase(), &Phase::Ready, "stuck uploading nothing");
    assert_eq!(flow.next(), None);
    assert_eq!(
        flow.insert_text(InsertStyle::BarePath),
        None,
        "nothing to paste"
    );
    let mut log = UploadLog::default();
    flow.number_in(&mut log);
    assert!(log.paths().is_empty());
}

/// A late report from a file that landed, arriving after the next one
/// started, moves the bar neither back nor past what landed.
#[test]
fn a_late_report_from_a_landed_file_changes_nothing() {
    let mut flow = Attachments::new();
    flow.processed(vec![100, 300]);
    flow.next();
    flow.landed(0, "/u/a.jpg");
    assert_eq!(flow.next(), Some(1));
    assert_eq!(flow.progress(1, 0.5), Some(0.625));
    assert_eq!(
        flow.progress(0, 0.4),
        Some(0.625),
        "a late report from file 0"
    );
    assert_eq!(flow.progress(0, 1.0), Some(0.625));
    assert_eq!(flow.progress(7, 0.9), None, "a file there is none of");
}
