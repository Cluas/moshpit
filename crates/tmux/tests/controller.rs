//! The controller's core, ported from `TmuxSessionControllerTests.swift`
//! (attach and discovery, listings, notifications, backfill, repair frames,
//! keys and paste, navigation, creating and killing) and its sibling suites
//! for the reconnect handoff, the channel closing, windows closed elsewhere,
//! refused listings and id order. The Swift suites wait on a real clock and
//! read a SwiftTerm grid; here time moves when told to and the pane's bytes
//! are read directly.

use moshpit_tmux::controller::{Config, Notification, ViewAction};
use moshpit_tmux::format::NAME_REFUSAL;
use moshpit_tmux::model::Selection;
use moshpit_tmux::rig::{Answer, Rig, Seen, err, ok};

/// How a repair frame blanks the screen (B364): never `ESC [2J`.
const BLANK: &str = "\u{1b}[0m\u{1b}[H\u{1b}[K\u{1b}[2;1H\u{1b}[J";

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

/// `makeAttachedController()`: attached, boot block swallowed, the three
/// discovery commands on the wire.
fn attached() -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.attach();
    assert_eq!(rig.commands.len(), 3);
    rig
}

fn attached_at(cols: i64, rows: i64) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| c.set_initial_client_size(cols, rows));
    rig.attach();
    rig
}

/// The three discovery replies, answered by hand.
fn discover(rig: &mut Rig, text: &str) {
    rig.push(text);
    rig.answered = rig.answered.max(3);
}

/// `pushOneWindowDiscovery`.
fn one_window(rig: &mut Rig, alternate: bool, copy_mode: bool) {
    let text = format!(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 {} {} bash\n%end 3 3 1\n\n",
        u8::from(alternate),
        u8::from(copy_mode)
    );
    discover(rig, &text);
}

const TWO_WINDOWS: &str = "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n$0 @1 1 81x24,0,0,1 0 1 work\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @1 0 80 24 1 0 0 vim\n%end 3 3 1\n\n";

const TWO_SESSIONS: &str = "%begin 1 1 1\n$0 1 main\n$1 0 dev\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n$1 @1 0 81x24,0,0,1 1 1 work\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @1 0 80 24 1 0 0 vim\n%end 3 3 1\n\n";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// MARK: - attach / discovery

#[test]
fn attach_sends_discovery_in_order() {
    let rig = attached();
    assert!(rig.controller.snapshot().is_attached);
    assert!(rig.commands[0].starts_with("list-sessions"));
    assert!(rig.commands[1].starts_with("list-windows"));
    assert!(rig.commands[2].starts_with("list-panes"));
}

#[test]
fn detach_clears_attached() {
    let mut rig = attached();
    rig.with(|c, now| c.detach(now));
    assert!(!rig.controller.snapshot().is_attached);
}

#[test]
fn list_sessions_picks_the_attached_session() {
    let mut rig = attached();
    rig.push("%begin 1 1 1\n$0 1 main\n$1 0 alt\n%end 1 1 1\n\n");
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.sessions.len(), 2);
    assert!(snapshot.sessions["$0"].is_attached);
    assert!(!snapshot.sessions["$1"].is_attached);
    assert_eq!(snapshot.active_session_id.as_deref(), Some("$0"));
}

#[test]
fn list_windows_takes_the_active_flag() {
    let mut rig = attached();
    rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n$0 @1 1 81x24,0,0,1 0 1 logs\n%end 2 2 1\n\n");
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.windows.len(), 2);
    assert_eq!(snapshot.windows["@0"].name, "main");
    assert_eq!(snapshot.windows["@0"].pane_count, 1);
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
}

#[test]
fn list_panes_builds_panes_and_the_active_one() {
    let mut rig = attached();
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 2 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @0 1 80 24 0 0 0 vim\n%end 3 3 1\n\n",
    );
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.panes.len(), 2);
    assert_eq!(snapshot.panes["%0"].command, "bash");
    assert!(snapshot.panes["%0"].is_active);
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%0"));
}

/// A window named "" (tmux ends the row on the separator) and a pane whose
/// command reads "" keep their rows (B36).
#[test]
fn empty_last_fields_keep_their_rows() {
    let mut rig = attached();
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @11 4 5960,80x24,0,0,11 1 1 \n%end 2 2 1\n%begin 3 3 1\n%3 @11 1 80 11 1 0 0 \n%end 3 3 1\n\n",
    );
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.windows["@11"].name, "");
    assert_eq!(snapshot.panes["%3"].command, "");
    // A name that starts with a combining mark (B35).
    rig.push("%window-renamed @11 \u{301}x\n");
    assert_eq!(rig.controller.snapshot().windows["@11"].name, "\u{301}x");
}

/// UI-DESIGN §10.5: every pane's working directory, the listing's last
/// field (`#{pane_current_path}`): with spaces in it and in the command,
/// empty, or missing from a reply in the format before it. A listing again
/// (here the refresh control) brings where the pane went since.
#[test]
fn list_panes_reads_each_panes_path_and_relisting_follows_it() {
    let mut rig = attached();
    assert!(
        rig.commands.iter().any(|c| c.starts_with("list-panes -a")
            && c.contains("#{pane_current_command} #{pane_current_path}'")),
        "{:?}",
        rig.commands
    );
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 4 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 zsh /Users/dev/code/app\n%1 @0 1 80 24 0 0 0 Google Chrome Helper /Users/dev/My Projects/site\n%2 @0 2 80 24 0 0 0 bash \n%3 @0 3 80 24 0 0 0 vim\n%end 3 3 1\n\n",
    );
    let read = |rig: &Rig, pane: &str| {
        let info = &rig.controller.snapshot().panes[pane];
        (info.command.clone(), info.path.clone())
    };
    assert_eq!(
        read(&rig, "%0"),
        ("zsh".into(), Some("/Users/dev/code/app".into()))
    );
    assert_eq!(
        read(&rig, "%1"),
        (
            "Google Chrome Helper".into(),
            Some("/Users/dev/My Projects/site".into())
        )
    );
    assert_eq!(read(&rig, "%2"), ("bash".into(), None), "an empty path");
    assert_eq!(
        read(&rig, "%3"),
        ("vim".into(), None),
        "the format before the path"
    );

    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-windows") {
            return ok(["$0 @0 0 81x24,0,0,0 1 4 main"]);
        }
        if command.starts_with("list-panes") && command.contains("pane_current_path") {
            return ok(lines(
                "%0 @0 0 80 24 1 0 0 zsh /Users/dev/code/app/src\n%1 @0 1 80 24 0 0 0 Google Chrome Helper /Users/dev/My Projects/site\n%2 @0 2 80 24 0 0 0 bash /tmp\n%3 @0 3 80 24 0 0 0 vim /etc",
            ));
        }
        None
    }));
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert_eq!(
        read(&rig, "%0").1.as_deref(),
        Some("/Users/dev/code/app/src")
    );
    assert_eq!(read(&rig, "%2").1.as_deref(), Some("/tmp"));
    assert_eq!(read(&rig, "%3").1.as_deref(), Some("/etc"));
}

#[test]
fn a_late_boot_banner_does_not_shift_pairing() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, now| c.attach(now));
    rig.boot();
    one_window(&mut rig, false, false);
    let snapshot = rig.controller.snapshot();
    assert!(snapshot.sessions.contains_key("$0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%0"));
}

#[test]
fn a_fresh_attach_pins_without_a_size_report() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    assert!(
        rig.has("resize-window -t @0 -x 69 -y 60"),
        "{:#?}",
        rig.commands
    );
}

#[test]
fn a_fresh_attach_polls_hooks_behind_the_tree() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, now| c.begin_control_mode(now));
    rig.push("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n");
    let tree = rig.index_of("list-panes -a -F '#{pane_id} ").unwrap();
    let stamps = rig.index_of("list-panes -a -F '#{pane_id}|").unwrap();
    assert!(stamps > tree);
    // Who is on the session first (B459: a desktop's control client keeps
    // the phone off a group of its own; the group follows the answer).
    // Then the session's layouts are read on both sides of our first size
    // report (B150), then the boot pre-reads for the size lease go ahead
    // of discovery.
    assert_eq!(rig.commands[0], "display-message -p 'self=#{client_name}'");
    assert!(rig.commands[1].starts_with("list-clients -F 'cc="));
    assert_eq!(rig.commands[2], "show-options -sqv @moshpit_cc");
    let layouts = format!(
        "list-windows -t '$0' -F '{}'",
        moshpit_tmux::controller::SIZE_PROBE_FORMAT
    );
    assert_eq!(rig.commands[3], layouts);
    assert!(rig.commands[4].starts_with("refresh-client -C 80x24"));
    assert_eq!(rig.commands[5], layouts);
    assert!(rig.commands[6].starts_with("display-message -p '#{client_name}'"));
}

// MARK: - sizes

#[test]
fn a_resize_storm_commits_once_at_the_final_size() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.with(|c, now| c.resize_client(40, 90, now));
    rig.with(|c, now| c.resize_client(37, 80, now));
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.wait(200);
    assert!(rig.has("refresh-client -C 70x35"));
    assert!(!rig.has("refresh-client -C 90x40"));
    assert!(!rig.has("refresh-client -C 80x37"));
}

#[test]
fn the_first_size_report_commits_even_when_it_matches_the_estimate() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    let before = rig.count("refresh-client -C 69x60");
    rig.with(|c, now| c.resize_client(60, 69, now));
    rig.wait(200);
    assert_eq!(rig.count("refresh-client -C 69x60"), before + 1);
    // A repeat of the confirmed size is deduped.
    rig.with(|c, now| c.resize_client(60, 69, now));
    rig.with(|c, now| c.resize_client(60, 69, now));
    rig.wait(400);
    assert_eq!(rig.count("refresh-client -C 69x60"), before + 1);
}

#[test]
fn a_settled_resize_captures_the_frame_then_the_cursor() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.wait(200);
    let since = rig.since(base);
    let capture = since
        .iter()
        .position(|c| c.starts_with("capture-pane -p -e -t %0"))
        .unwrap();
    let cursor = since
        .iter()
        .position(|c| c.starts_with("display-message -p -t %0 '#{cursor_x}"))
        .unwrap();
    assert!(capture < cursor);
}

// MARK: - repair frames

#[test]
fn a_frame_without_reveal_corrects_but_keeps_the_cover() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    // The attach's own repaint revealed the pane.
    assert!(rig.view_actions("%0").contains(&ViewAction::Reveal));
    let seen = rig.seen.len();
    rig.with(|c, now| c.resync_active_pane_now(false, now));
    rig.push("%begin 100 100 1\nresynced without reveal\n%end 100 100 1\n%begin 101 101 1\n0 0\n%end 101 101 1\n");
    rig.answered = rig.commands.len();
    let after: Vec<&Seen> = rig.seen[seen..].iter().collect();
    assert!(after.iter().any(|s| matches!(s, Seen::Pane(p, b) if p == "%0" && text(b).contains("resynced without reveal"))));
    assert!(
        !after
            .iter()
            .any(|s| matches!(s, Seen::View(_, ViewAction::Reveal)))
    );
    let seen = rig.seen.len();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    rig.push("%begin 102 102 1\nresynced and revealed\n%end 102 102 1\n%begin 103 103 1\n0 0\n%end 103 103 1\n");
    rig.answered = rig.commands.len();
    assert!(
        rig.seen[seen..]
            .iter()
            .any(|s| matches!(s, Seen::View(p, ViewAction::Reveal) if p == "%0"))
    );
}

#[test]
fn the_frame_erases_the_screen_it_replaces_then_the_cursor_lands() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let seen = rig.seen.len();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    rig.push("%begin 7 7 1\nrow one\nrow two\n\n   \n%end 7 7 1\n%begin 8 8 1\n4 1\n%end 8 8 1\n");
    rig.answered = rig.commands.len();
    let fed: Vec<u8> = rig.seen[seen..]
        .iter()
        .filter_map(|s| match s {
            Seen::Pane(p, b) if p == "%0" => Some(b.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    // Blanked without `ESC [2J`, which the terminal core answers by pushing
    // the screen into the history (B364).
    assert_eq!(
        text(&fed),
        format!("{BLANK}\u{1b}[Hrow one\r\nrow two\u{1b}[0m\u{1b}[2;5H")
    );
    assert!(!text(&fed).contains("\u{1b}[2J"));
}

#[test]
fn a_frame_captured_at_a_foreign_size_is_rejected_and_recaptured() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    let seen = rig.seen.len();
    let base = rig.commands.len();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    let tall: Vec<String> = (1..=70).map(|i| format!("wide-frame-row-{i}")).collect();
    let mut reply = moshpit_tmux::rig::block(200, &tall, false);
    reply.push_str("%begin 201 201 1\n0 0\n%end 201 201 1\n");
    rig.push(&reply);
    rig.answered = rig.commands.len().min(base + 2);
    // Not painted; the window is re-pinned; a recapture follows.
    assert!(
        !rig.seen[seen..]
            .iter()
            .any(|s| matches!(s, Seen::Pane(_, b) if text(b).contains("wide-frame-row")))
    );
    assert!(
        rig.since(base)
            .iter()
            .any(|c| c.starts_with("resize-window -t @0 -x 69"))
    );
    rig.settle();
    rig.wait(350);
    assert!(
        rig.since(base)
            .iter()
            .filter(|c| c.starts_with("capture-pane -p -e -t %0"))
            .count()
            >= 2
    );
}

#[test]
fn a_frame_with_foreign_width_lines_is_rejected_even_when_rows_fit() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    let seen = rig.seen.len();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    let wide: Vec<String> = (0..30).map(|_| "W".repeat(200)).collect();
    let mut reply = moshpit_tmux::rig::block(200, &wide, false);
    reply.push_str("%begin 201 201 1\n0 0\n%end 201 201 1\n");
    rig.push(&reply);
    assert!(
        !rig.seen[seen..]
            .iter()
            .any(|s| matches!(s, Seen::Pane(_, b) if text(b).contains("WWWW")))
    );
}

#[test]
fn rejected_frames_are_recaptured_until_one_lands() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    let mut oversized = 0;
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("capture-pane -p -e -t") {
            if oversized < 2 {
                oversized += 1;
                return ok((1..=70).map(|i| format!("flap-row-{i}")));
            }
            return ok(["REPAIRED-AFTER-RETRY"]);
        }
        if command.starts_with("display-message") {
            return ok(["0 0"]);
        }
        None
    }));
    rig.push("%layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\n");
    rig.push("%layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\n");
    rig.wait_settled(4000, 50);
    assert!(text(&rig.pane_bytes("%0")).contains("REPAIRED-AFTER-RETRY"));
}

#[test]
fn a_refused_capture_paints_nothing() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let seen = rig.seen.len();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    rig.push("%begin 7 7 1\ncan't find pane: %0\n%error 7 7 1\n%begin 8 8 1\ncan't find pane: %0\n%error 8 8 1\n");
    assert!(!rig.seen[seen..].iter().any(|s| matches!(s, Seen::Pane(..))));
}

#[test]
fn resyncs_coalesce_while_one_is_in_flight() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    for _ in 0..4 {
        rig.with(|c, now| c.resync_active_pane_now(true, now));
    }
    let count = |rig: &Rig| {
        rig.since(base)
            .iter()
            .filter(|c| c.starts_with("capture-pane -p -e -t %0"))
            .count()
    };
    assert_eq!(count(&rig), 1);
    rig.settle();
    assert_eq!(count(&rig), 2, "the original plus one coalesced follow-up");
}

#[test]
fn pause_resumes_with_the_real_verb_and_continue_repaints() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    rig.push("%pause %0\n");
    assert!(rig.has("refresh-client -A %0:continue"));
    let before = rig.count("capture-pane -p -e -t %0");
    rig.push("%continue %0\n");
    assert!(rig.count("capture-pane -p -e -t %0") > before);
}

// MARK: - window places (B586)

/// tmux answering the listings with the two windows swapped
/// (`swap-window` on a desktop, which notifies nobody).
fn swapped(rig: &mut Rig) {
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-windows -a") {
            return ok([
                "$0 @0 1 81x24,0,0,0 1 1 main",
                "$0 @1 0 81x24,0,0,1 0 1 work",
            ]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok(["%0 @0 0 80 24 1 0 0 bash", "%1 @1 0 80 24 1 0 0 vim"]);
        }
        ok(["0 0"])
    }));
}

fn index_of_window(rig: &Rig, window: &str) -> i64 {
    rig.controller.snapshot().windows[window].index
}

/// B586: a window's place changed with no word from tmux; the quiet relist
/// (Home or the picker showing) reads it — no spinner, no repaint.
#[test]
fn a_relist_reads_the_windows_places_quietly() {
    let mut rig = attached();
    discover(&mut rig, TWO_WINDOWS);
    rig.settle();
    assert_eq!(index_of_window(&rig, "@0"), 0);
    swapped(&mut rig);
    let from = rig.commands.len();
    rig.with(|c, now| c.relist(now));
    rig.settle();
    assert_eq!(index_of_window(&rig, "@0"), 1);
    assert_eq!(index_of_window(&rig, "@1"), 0);
    let sent = rig.since(from);
    assert!(
        !sent.iter().any(|c| c.starts_with("capture-pane") || c == "display-message -p done"),
        "{sent:?}"
    );
    assert!(!rig.controller.is_refreshing());
}

/// B586: back from the background, the tree is read again.
#[test]
fn the_return_from_the_background_reads_the_windows_places() {
    let mut rig = attached();
    discover(&mut rig, TWO_WINDOWS);
    rig.settle();
    rig.with(|c, now| c.release_window_pins(now));
    rig.settle();
    swapped(&mut rig);
    rig.with(|c, now| c.repin_active_window(now));
    rig.settle();
    assert_eq!(index_of_window(&rig, "@0"), 1, "{:?}", rig.commands);
}

// MARK: - the phone's own pane (B587)

/// One window split in two, the phone on `%0`.
const SPLIT_WINDOW: &str = "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 c3d4,81x24,0,0{40x24,0,0,0,40x24,41,0,1} 1 2 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 40 24 1 0 0 bash\n%1 @0 1 40 24 0 0 0 vim\n%end 3 3 1\n\n";

fn on_split_window() -> Rig {
    let mut rig = attached();
    discover(&mut rig, SPLIT_WINDOW);
    rig.settle();
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%0")
    );
    rig
}

/// B587: a desktop picked another pane in the window the phone shows (the
/// active pane is the window's, shared): the phone keeps its own under a
/// veil — it switched under the person — and a keystroke takes it back,
/// active and zoomed again.
#[test]
fn a_pane_picked_on_the_desktop_leaves_the_phone_on_its_own() {
    let mut rig = on_split_window();
    rig.push("%window-pane-changed @0 %1\n");
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%0"),
        "the phone's own pane"
    );
    assert_eq!(
        rig.controller.pane_follow().map(|f| f.pane_id.as_str()),
        Some("%0")
    );
    let from = rig.commands.len();
    rig.with(|c, now| c.view_input("%0", b"x", now));
    let sent = rig.since(from).to_vec();
    assert!(sent.iter().any(|c| c == "select-pane -t %0"), "{sent:?}");
    assert!(
        sent.iter().any(|c| c == "send-keys -t %0 -H 78"),
        "{sent:?}"
    );
    assert!(
        sent.iter()
            .any(|c| c.starts_with("display-message -p -t '@0' '#{window_panes}")),
        "the zoom's probe: {sent:?}"
    );
    assert!(rig.controller.pane_follow().is_none());
}

/// The veil's button (the size-lease's take-over) takes the pane back too.
#[test]
fn the_veils_button_takes_the_pane_back() {
    let mut rig = on_split_window();
    rig.push("%window-pane-changed @0 %1\n");
    let from = rig.commands.len();
    rig.with(|c, now| c.take_size_ownership(now));
    assert!(rig.since(from).iter().any(|c| c == "select-pane -t %0"));
    assert!(rig.controller.pane_follow().is_none());
}

/// The phone's own pick, and its own split, are no other client's: their
/// echo lands while the command is still unanswered.
#[test]
fn the_phones_own_pane_changes_are_followed() {
    let mut rig = on_split_window();
    rig.with(|c, now| c.select_pane("%1", now));
    rig.push("%window-pane-changed @0 %1\n");
    assert!(rig.controller.pane_follow().is_none());
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%1")
    );
    rig.settle();
    rig.with(|c, now| c.new_pane(now));
    rig.push("%window-pane-changed @0 %2\n");
    assert!(rig.controller.pane_follow().is_none());
}

/// The kept pane gone (exited, killed), the veil goes and the phone shows
/// what tmux has active.
#[test]
fn a_kept_pane_that_goes_drops_the_veil() {
    let mut rig = on_split_window();
    rig.push("%window-pane-changed @0 %1\n");
    assert!(rig.controller.pane_follow().is_some());
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 main"]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok(["%1 @0 1 81 24 1 0 0 vim"]);
        }
        if command.starts_with("list-windows -a") {
            return ok(["$0 @0 0 81x24,0,0,1 1 1 main"]);
        }
        ok(["0 0"])
    }));
    rig.with(|c, now| c.refresh(now));
    rig.settle();
    assert!(rig.controller.pane_follow().is_none());
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%1")
    );
}

// MARK: - pane modes (B614)

/// `%pane-mode-changed`: the controller asks which mode, and the pane's
/// input flags say it — the mosh path's copy-mode latch follows them.
#[test]
fn a_pane_mode_change_is_asked_and_kept() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    assert!(!rig.controller.active_input_flags().in_copy_mode);
    let mode = std::rc::Rc::new(std::cell::Cell::new("1 0"));
    let answer = mode.clone();
    rig.respond(Box::new(move |command, _| {
        if command == "display-message -p -t %0 '#{pane_in_mode} #{alternate_on}'" {
            return ok([answer.get()]);
        }
        ok(["0 0"])
    }));
    rig.push("%pane-mode-changed %0\n");
    assert!(rig.has("display-message -p -t %0 '#{pane_in_mode} #{alternate_on}'"));
    rig.settle();
    assert!(rig.controller.active_input_flags().in_copy_mode);
    mode.set("0 0");
    rig.push("%pane-mode-changed %0\n");
    rig.settle();
    assert!(!rig.controller.active_input_flags().in_copy_mode);
}

/// Discovery's `#{pane_in_mode}` counts too: a pane left in copy-mode.
#[test]
fn a_pane_found_in_copy_mode_says_so() {
    let mut rig = attached();
    one_window(&mut rig, false, true);
    assert!(rig.controller.active_input_flags().in_copy_mode);
}

// MARK: - backfill

/// The dump, then the screen right behind it — sent together, before the
/// dump's reply (B585: a round trip between the two lost what scrolled off
/// tmux's screen meanwhile). The replies keep that order.
#[test]
fn the_screen_goes_out_right_behind_the_dump() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    let dump = rig
        .index_of("capture-pane -p -e -S -400 -E -1 -t %0")
        .expect("the dump");
    let frame = rig
        .index_of("capture-pane -p -e -t %0")
        .expect("the screen");
    let cursor = rig
        .index_of("display-message -p -t %0 '#{cursor_x} #{cursor_y}")
        .expect("the cursor");
    assert_eq!((frame, cursor), (dump + 1, dump + 2), "{:?}", rig.commands);
    assert!(!rig.has("display-message -p -t %0 '#{alternate_on}"));
}

#[test]
fn an_alternate_screen_pane_gets_its_screen_only() {
    let mut rig = attached();
    one_window(&mut rig, true, false);
    assert!(rig.has("capture-pane -p -e  -t %0"));
}

#[test]
fn the_backfill_cancels_a_stale_copy_mode() {
    let mut rig = attached();
    one_window(&mut rig, false, true);
    assert!(rig.has("send-keys -t %0 -X cancel"));
}

#[test]
fn history_lands_before_the_frame_with_crlf() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.respond(Box::new(|command, _| {
        if command.starts_with("capture-pane -p -e -S") {
            return ok(["old 1", "old 2", ""]);
        }
        if command.starts_with("capture-pane") {
            return ok(["$ "]);
        }
        ok(["0 0"])
    }));
    rig.settle();
    let fed = text(&rig.pane_bytes("%0"));
    // The dump replaces the local history (B364): printed on a blank
    // screen, pushed whole into the history from the bottom row (two lines,
    // two line feeds), then the frame.
    assert!(
        fed.starts_with(&format!(
            "{BLANK}\u{1b}[3J\u{1b}[Hold 1\r\nold 2\u{1b}[0m\u{1b}[9999;1H\n\n{BLANK}\u{1b}[H$ \u{1b}[0m"
        )),
        "{fed:?}"
    );
}

#[test]
fn a_foreign_width_dump_is_recaptured_and_the_frame_repainted_on_top() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    let mut dumps = 0;
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("capture-pane -p -e -S") {
            dumps += 1;
            if dumps == 1 {
                return ok(vec!["W".repeat(80); 120]);
            }
            return ok((1..=120).map(|i| format!("history {i}")));
        }
        ok(["0 0"])
    }));
    rig.settle();
    assert!(!text(&rig.pane_bytes("%0")).contains("WWW"));
    rig.wait(800);
    rig.settle();
    assert_eq!(rig.count("capture-pane -p -e -S"), 2);
    let retry = rig
        .commands
        .iter()
        .rposition(|c| c.starts_with("capture-pane -p -e -S"))
        .unwrap();
    assert!(text(&rig.pane_bytes("%0")).contains("history 120"));
    assert!(
        rig.since(retry + 1)
            .iter()
            .any(|c| c.starts_with("capture-pane -p -e -t %0"))
    );
}

#[test]
fn only_the_on_screen_window_is_filled_on_attach() {
    let mut rig = attached();
    discover(&mut rig, &TWO_WINDOWS.replace("%1 @1", "%9 @1"));
    rig.settle();
    assert!(
        !rig.commands
            .iter()
            .any(|c| c.starts_with("capture-pane -p -e -S") && c.contains("-t %9"))
    );
    rig.with(|c, now| c.select_pane("%9", now));
    rig.settle();
    assert!(
        rig.commands
            .iter()
            .any(|c| c.starts_with("capture-pane -p -e -S") && c.contains("-t %9"))
    );
}

// MARK: - navigation

#[test]
fn select_window_resyncs_the_target_twice() {
    let mut rig = attached();
    discover(&mut rig, TWO_WINDOWS);
    rig.settle();
    rig.with(|c, now| c.select_window("@1", now));
    rig.settle();
    let select = rig.index_of("select-window -t @1").unwrap();
    let capture = rig.index_of("capture-pane -p -e -t %1").unwrap();
    assert!(select < capture);
    rig.wait_settled(800, 100);
    assert!(rig.count("capture-pane -p -e -t %1") >= 2);
}

#[test]
fn select_window_sizes_the_target_before_selecting_it() {
    let mut rig = attached();
    discover(&mut rig, TWO_WINDOWS);
    rig.with(|c, now| c.resize_client(35, 70, now));
    rig.with(|c, now| c.select_window("@1", now));
    let resize = rig.index_of("resize-window -t @1 -x 70").unwrap();
    let select = rig.index_of("select-window -t @1").unwrap();
    assert!(resize < select);
}

#[test]
fn select_window_is_optimistic_and_tells_tmux() {
    let mut rig = attached();
    rig.with(|c, now| c.select_window("@77", now));
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@77")
    );
    assert!(rig.has("select-window -t @77"));
    let before = rig.commands.len();
    rig.with(|c, now| c.select_window("@77", now));
    assert!(
        rig.commands.len() > before,
        "the same window again still commands"
    );
}

#[test]
fn select_pane_keeps_the_zoom() {
    let mut rig = attached();
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 2 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @0 1 80 24 0 0 0 vim\n%end 3 3 1\n\n",
    );
    rig.with(|c, now| c.select_pane("%1", now));
    assert!(rig.has("select-pane -Z -t %1"));
}

#[test]
fn a_pane_known_only_from_output_waits_for_discovery() {
    let mut rig = attached();
    rig.push("%output %5 streaming\n");
    assert!(rig.controller.snapshot().panes.contains_key("%5"));
    rig.with(|c, now| c.select_pane("%5", now));
    assert_ne!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("")
    );
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 0"]);
        }
        if command.starts_with("list-windows") {
            return ok(lines(
                "$0 @4 5 81x24,0,0,4 1 1 ref\n$0 @5 6 81x24,0,0,5 0 1 islander",
            ));
        }
        if command.starts_with("list-panes")
            && command.contains("pane_current_command")
            && !command.contains("moshpit")
        {
            return ok(lines(
                "%4 @4 0 80 24 1 0 0 claude\n%5 @5 0 80 24 1 0 0 claude",
            ));
        }
        None
    }));
    rig.answered = 0;
    rig.settle();
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%5"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@5"));
    assert!(rig.has("select-window -t @5"));
}

#[test]
fn new_window_lands_on_the_window_it_made() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.new_window(Some("logs"), now));
    assert!(rig.has("new-window -P -F '#{window_id}' -n \"logs\""));
    rig.push("%begin 20 20 1\n@1\n%end 20 20 1\n%begin 21 21 1\n$0 @0 0 81x24,0,0,0 0 1 main\n$0 @1 1 81x24,0,0,1 1 1 logs\n%end 21 21 1\n%begin 22 22 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @1 0 80 24 1 0 0 bash\n%end 22 22 1\n");
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@1"));
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%1"));
    assert!(
        rig.since(base)
            .iter()
            .any(|c| c.starts_with("select-window -t @1"))
    );
}

// B139: what a control-mode line carries.

#[test]
fn a_name_holding_a_line_break_is_refused_and_no_line_break_reaches_tmux() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let base = rig.commands.len();
    let name = "x\nrun-shell \"touch INJECTED\" #";
    rig.with(|c, now| c.rename_window("@1", name, now));
    rig.with(|c, now| c.rename_session("$1", name, now));
    rig.with(|c, now| c.new_session(Some(name), now));
    rig.with(|c, now| c.new_window(Some("a\rb"), now));
    rig.with(|c, now| c.new_window_in_session("$1", Some("tab\there"), now));
    rig.with(|c, now| c.run_command("display-message -p x\nrun-shell x", now));
    let sent = rig.since(base);
    assert!(
        !sent
            .iter()
            .any(|c| c.contains("run-shell") || c.contains(['\n', '\r', '\t'])),
        "{sent:?}"
    );
    assert_eq!(rig.controller.notice(), Some(NAME_REFUSAL));
}

#[test]
fn a_name_reaches_tmux_exactly_as_typed() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.rename_window("@1", r#"#{session_name} $HOME ~ "q" \b"#, now));
    rig.with(|c, now| c.rename_session("$1", "-x", now));
    rig.with(|c, now| c.new_session(Some("a#b"), now));
    rig.with(|c, now| c.new_window(Some("#(touch INJECTED)"), now));
    let sent = rig.since(base);
    for want in [
        r###"rename-window -t @1 -- "##{session_name} \$HOME \~ \"q\" \\b""###,
        r###"rename-session -t $1 -- "-x""###,
        r###"new-session -d -P -F '#{session_id} #{session_name}' -s "a##b""###,
        r###"new-window -P -F '#{window_id}' -n "##(touch INJECTED)""###,
    ] {
        assert!(sent.iter().any(|c| c == want), "{want}\n{sent:?}");
    }
}

#[test]
fn only_a_tmux_id_is_spliced_into_a_target() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.rename_window("@1 ; run-shell x", "n", now));
    rig.with(|c, now| c.rename_session("=main", "n", now));
    rig.with(|c, now| c.kill_window("@1;kill-server", now));
    rig.with(|c, now| c.kill_session("main", now));
    rig.with(|c, now| c.kill_pane("%1 ; kill-server", now));
    assert!(rig.since(base).is_empty(), "{:?}", rig.since(base));
}

#[test]
fn new_window_in_another_session_names_it() {
    let mut rig = attached();
    discover(&mut rig, TWO_SESSIONS);
    rig.with(|c, now| c.new_window_in_session("$1", Some("api"), now));
    // B580: made without becoming `$1`'s current window (a desktop there
    // stays put); the phone picks it in its own view.
    assert!(rig.has("new-window -d -t $1: -P -F '#{window_id}' -n"));
    let before = rig.commands.len();
    rig.with(|c, now| c.new_window_in_session("$0", None, now));
    assert!(
        rig.since(before)
            .iter()
            .any(|c| c.starts_with("new-window -P -F '#{window_id}'"))
    );
}

#[test]
fn new_pane_in_an_off_screen_window_switches_first() {
    let mut rig = attached();
    discover(&mut rig, TWO_SESSIONS);
    rig.with(|c, now| c.new_pane_in_window("@1", now));
    // B459: the hop is into the phone's own group of `$1`.
    let hop = rig.index_of("new-session -t '$1'").unwrap();
    let select = rig.index_of("select-window -t @1").unwrap();
    let split = rig.index_of("split-window -t %1").unwrap();
    let zoom = rig.index_of("resize-pane -Z -t @1").unwrap();
    assert!(hop < select && select < split && split < zoom);
}

/// A session of two windows, `@1` current with `%1`, `@2` with `%2`, that
/// tmux counts `attached` clients on; this control client `/dev/ttys003`,
/// plus `clients` (`control|pid|session|name` rows); a reconnect replaying
/// `@2`/`%2`, the person's pick or not.
fn restoring(attached: u32, clients: &'static [&'static str], chosen: bool) -> Rig {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| {
        c.set_pending_restore(Some(Selection {
            session: "$0".into(),
            window: Some("@2".into()),
            pane: Some("%2".into()),
        }));
        c.set_restore_chosen(chosen);
    });
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("list-sessions") {
            return ok([format!("$0 {attached} main")]);
        }
        if command.starts_with("list-windows -a -F '#{session_id}") {
            return ok([
                "$0 @1 0 b25d,80x24,0,0,1 1 1 main",
                "$0 @2 1 b25e,80x24,0,0,2 0 1 logs",
            ]);
        }
        if command.starts_with("list-panes -a -F '#{pane_id} ") {
            return ok(["%1 @1 0 80 24 1 0 0 bash", "%2 @2 0 80 24 1 0 0 bash"]);
        }
        if command.starts_with("display-message -p '#{client_name}'") {
            return ok(["/dev/ttys003"]);
        }
        if command.starts_with("list-clients -F '#{client_control_mode}") {
            let mut rows = vec!["1|111|$0|/dev/ttys003"];
            rows.extend_from_slice(clients);
            return ok(rows);
        }
        None
    }));
    rig.with(|c, now| c.begin_control_mode(now));
    rig.push("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 main\n");
    rig.settle();
    rig
}

/// B457: a session's current window and a window's active pane belong to
/// the session, and every client on it shows them. A reconnect replayed
/// where the app last was with `select-window`/`select-pane`: a desktop on
/// that session jumped to the phone's old window mid-typing, its keys going
/// to another pane. With another client on the session the replay keeps
/// the session's own window.
#[test]
fn a_reconnects_restore_leaves_a_desktop_on_the_session_where_it_is() {
    let rig = restoring(2, &["0|222|$0|/dev/ttys004"], false);
    assert!(
        rig.has("list-clients -F '#{client_control_mode}"),
        "the restore asks who is on the session"
    );
    assert!(!rig.has("select-window -t @2"));
    assert!(!rig.has("select-pane -Z -t %2"));
    assert_eq!(rig.controller.pending_restore(), None, "settled");
}

/// Nobody else there (a client on another session doesn't count): the
/// replay lands where the app was, as before.
#[test]
fn a_restore_with_nobody_else_on_the_session_selects_as_before() {
    let rig = restoring(2, &["0|222|$5|/dev/ttys004"], false);
    assert!(rig.has("select-window -t @2") || rig.has("select-pane -Z -t %2"));
}

/// Alone on its session by tmux's own count, the usual reconnect: no
/// listing, no round trip, the replay as before.
#[test]
fn a_restore_alone_on_its_session_asks_nobody() {
    let rig = restoring(1, &[], false);
    assert!(!rig.has("list-clients -F '#{client_control_mode}"));
    assert!(rig.has("select-window -t @2") || rig.has("select-pane -Z -t %2"));
}

/// The person's own pick (a notification tapped while redialling, B405)
/// moves what it names, as a tap does, desktop or not.
#[test]
fn a_persons_pick_selects_even_with_a_desktop_there() {
    let rig = restoring(2, &["0|222|$0|/dev/ttys004"], true);
    assert!(!rig.has("list-clients -F '#{client_control_mode}"));
    assert!(rig.has("select-window -t @2") || rig.has("select-pane -Z -t %2"));
}

#[test]
fn a_restore_into_a_split_window_stays_zoomed() {
    // tmux runs commands in order: the zoom probes and the toggles are
    // modelled the way it executes them.
    let mut rig = Rig::new(Config::default());
    rig.with(|c, _| {
        c.set_pending_restore(Some(Selection {
            session: "$0".into(),
            window: Some("@1".into()),
            pane: Some("%4".into()),
        }))
    });
    rig.with(|c, now| c.attach(now));
    rig.push("%begin 100 0 0\n%end 100 0 0\n\n%session-changed $0 0\n");
    let layout = "ed9b,499x62,0,0{215x62,0,0,0,283x62,216,0[283x11,216,0,1,283x10,216,12,2,283x9,216,23,3,283x9,216,33,4,283x9,216,43,5,283x9,216,53,6]}";
    let zoomed = std::rc::Rc::new(std::cell::Cell::new(false));
    let active = std::rc::Rc::new(std::cell::RefCell::new("%6".to_owned()));
    let (z, a) = (zoomed.clone(), active.clone());
    rig.respond(Box::new(move |command, _| {
        if command.starts_with("list-sessions") {
            return ok(["$0 1 0"]);
        }
        if command.starts_with("list-windows") {
            return ok([format!("$0 @1 1 {layout} 1 7 team")]);
        }
        if command.starts_with("list-panes")
            && command.contains("pane_current_command")
            && !command.contains("moshpit")
        {
            let active = a.borrow().clone();
            return ok((0..=6).map(move |i| {
                format!(
                    "%{i} @1 {i} 80 9 {} 0 0 claude",
                    u8::from(active == format!("%{i}"))
                )
            }));
        }
        if command.starts_with("display-message -p -t '@1'") {
            return ok([format!("7 {} {} {layout}", u8::from(z.get()), a.borrow())]);
        }
        if let Some(pane) = command.strip_prefix("select-pane -Z -t ") {
            *a.borrow_mut() = pane.to_owned();
            return Some(Answer {
                after: format!("%window-pane-changed @1 {pane}\n"),
                ..Answer::default()
            });
        }
        if command
            .starts_with("if-shell -F -t '@1' '#{?window_zoomed_flag,,1}' 'resize-pane -Z -t @1'")
        {
            z.set(true);
        } else if command.starts_with("resize-pane -Z -t '@1'")
            || command.starts_with("resize-pane -Z -t @1")
        {
            z.set(!z.get());
        }
        None
    }));
    rig.settle();
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%4")
    );
    assert!(rig.count("display-message -p -t '@1'") >= 2);
    assert!(zoomed.get(), "the split window ends up zoomed");
}

// MARK: - notifications

#[test]
fn output_feeds_the_pane_and_mints_on_demand() {
    let mut rig = attached();
    let stream = rig.with(|c, _| c.stream_for_pane("%0"));
    rig.push("%output %0 hello\n");
    assert_eq!(rig.controller.stream_of("%0"), Some(stream));
    assert_eq!(rig.pane_bytes("%0"), b"hello");
    rig.push("%output %42 surprise\n");
    assert!(rig.controller.snapshot().panes.contains_key("%42"));
    assert_eq!(rig.pane_bytes("%42"), b"surprise");
    assert_ne!(rig.controller.stream_of("%42"), Some(stream));
}

#[test]
fn output_and_replies_are_handled_in_stream_order() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    for _ in 0..20 {
        rig.with(|c, now| c.poll_agent_hooks(now));
    }
    let seen = rig.seen.len();
    for _ in 0..20 {
        rig.push("%output %0 x\n%begin 9 9 1\n%end 9 9 1\n");
        // A pane's output is noted once a quarter second at most (B739).
        rig.wait(moshpit_tmux::controller::ACTIVITY_NOTE_MS);
    }
    let order: Vec<&str> = rig.seen[seen..]
        .iter()
        .filter_map(|s| match s {
            Seen::Note(Notification::PaneActivity { .. }) => Some("out"),
            Seen::Note(Notification::AgentHooksUpdated) => Some("hooks"),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["out", "hooks"].repeat(20));
}

/// B739: every `%output` line was a note of the pane's activity — 2 000 a
/// second from `yes` over a fast link — and every note an event to the
/// app: a call into the agents' machine on the main thread and a state
/// sync, all queued ahead of the Ctrl-C that would stop the flood. A
/// pane's output is noted once a quarter second at most, at once when it
/// starts.
#[test]
fn a_flood_of_output_is_noted_once_a_quarter_second() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let activity = |rig: &Rig, from: usize| {
        rig.seen[from..]
            .iter()
            .filter(|s| matches!(s, Seen::Note(Notification::PaneActivity { .. })))
            .count()
    };
    let seen = rig.seen.len();
    for _ in 0..500 {
        rig.push("%output %0 yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\\012\n");
    }
    assert_eq!(activity(&rig, seen), 1, "noted at once, then not again");
    rig.wait(moshpit_tmux::controller::ACTIVITY_NOTE_MS);
    rig.push("%output %0 y\\012\n");
    assert_eq!(activity(&rig, seen), 2, "noted again a quarter second on");
}

#[test]
fn window_add_relists() {
    let mut rig = attached();
    let base = rig.commands.len();
    rig.push("%window-add @9\n");
    assert!(rig.controller.snapshot().windows.contains_key("@9"));
    let tail = rig.since(base);
    assert!(tail.iter().any(|c| c.starts_with("list-windows")));
    assert!(tail.iter().any(|c| c.starts_with("list-panes")));
}

#[test]
fn window_close_cascades_and_lands_locally() {
    let mut rig = attached();
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n$0 @1 1 81x24,0,0,1 0 1 logs\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @1 0 80 24 0 0 0 tail\n%end 3 3 1\n\n",
    );
    let base = rig.commands.len();
    rig.push("%window-close @0\n");
    let snapshot = rig.controller.snapshot();
    assert!(!snapshot.windows.contains_key("@0"));
    assert!(!snapshot.panes.contains_key("%0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@1"));
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%1"));
    assert_eq!(rig.controller.stream_of("%0"), None);
    assert!(
        rig.seen
            .iter()
            .any(|s| matches!(s, Seen::Closed(p, _) if p == "%0"))
    );
    assert!(
        !rig.since(base)
            .iter()
            .any(|c| c.starts_with("select-window"))
    );
    rig.push("%session-window-changed $0 @1\n");
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@1")
    );
}

#[test]
fn renames_and_unknown_layouts() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.push("%window-renamed @0 edge\n");
    assert_eq!(rig.controller.snapshot().windows["@0"].name, "edge");
    assert_eq!(rig.controller.snapshot().windows["@0"].index, 0);
    rig.push("%layout-change @99 81x24,0,0,77\n");
    assert_eq!(
        rig.controller.snapshot().windows["@99"].layout,
        "81x24,0,0,77"
    );
    rig.push("%session-changed $42 newone\n");
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$42")
    );
    assert_eq!(rig.controller.snapshot().sessions["$42"].name, "newone");
    rig.push("%session-renamed $42 x|y\n");
    assert_eq!(rig.controller.snapshot().sessions["$42"].name, "x|y");
}

#[test]
fn pane_focus_only_for_our_windows() {
    let mut rig = attached();
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%7 @0 0 80 24 1 0 0 zsh\n%end 3 3 1\n\n",
    );
    rig.push("%window-pane-changed @0 %7\n");
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%7")
    );
    rig.push("%window-pane-changed @77 %99\n");
    assert_eq!(
        rig.controller.snapshot().active_pane_id.as_deref(),
        Some("%7")
    );
}

#[test]
fn another_clients_window_switch_does_not_yank_ours() {
    let mut rig = attached();
    discover(&mut rig, &TWO_WINDOWS.replace(" bash", " zsh"));
    rig.push("%session-window-changed $0 @1\n");
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@0")
    );
    // Nor does a re-list flagging @1.
    rig.settle();
    rig.push("%window-add @9\n");
    let base = rig.answered;
    rig.answered = rig.commands.len();
    let _ = base;
    rig.push("%begin 11 11 1\n$0 @0 0 81x24,0,0,0 0 1 main\n$0 @1 1 81x24,0,0,1 1 1 logs\n%end 11 11 1\n%begin 12 12 1\n%0 @0 0 80 24 1 0 0 zsh\n%1 @1 0 80 24 1 0 0 vim\n%end 12 12 1\n");
    assert!(!rig.controller.snapshot().windows.contains_key("@9"));
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@0")
    );
    // Our own select's echo holds.
    rig.with(|c, now| c.select_window("@1", now));
    rig.push("%session-window-changed $0 @1\n");
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@1")
    );
}

#[test]
fn a_foreign_session_window_change_is_ignored() {
    let mut rig = attached();
    rig.push("%session-changed $0 main\n");
    rig.push("%session-window-changed $0 @9\n");
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@9")
    );
    rig.push("%session-window-changed $5 @42\n");
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@9")
    );
}

#[test]
fn exit_detaches_and_client_detached_does_not() {
    let mut rig = attached();
    rig.push("%client-detached client-7\n");
    assert!(rig.controller.snapshot().is_attached);
    rig.push("%exit detached\n");
    assert!(!rig.controller.snapshot().is_attached);
}

/// B786: `%exit` says the control client ended, with tmux's reason, for
/// the leg to ask the server which it was; a server found gone is
/// forgotten — its tree emptied (that it was attached once stays), each
/// pane's terminal closed, the agents' stamps dropped.
#[test]
fn an_exit_is_told_and_a_gone_server_is_forgotten() {
    let mut rig = attached();
    rig.push("%exit server exited unexpectedly\n");
    assert!(rig.notes().contains(&Notification::ControlExited {
        reason: Some("server exited unexpectedly".into())
    }));
    rig.controller.forget_server();
    let snapshot = rig.controller.snapshot();
    assert!(
        snapshot.sessions.is_empty() && snapshot.windows.is_empty() && snapshot.panes.is_empty()
    );
    assert!(snapshot.active_pane_id.is_none() && !snapshot.is_attached);
    assert!(
        !snapshot.ever_attached,
        "a forgotten server's next session is its first"
    );
    assert!(rig.controller.agent_hooks().is_empty());
}

// MARK: - listings

#[test]
fn a_relist_never_moves_our_session() {
    let mut rig = attached();
    rig.push("%session-changed $0 0\n");
    rig.with(|c, now| c.parse_list_sessions(&["$0 1 0", "$1 1 dev"], now));
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$0")
    );
    assert!(rig.controller.snapshot().sessions["$1"].is_attached);
    rig.with(|c, now| c.parse_list_sessions(&["$1 1 dev", "$2 0 scratch"], now));
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$1")
    );
    let mut fresh = Rig::new(Config::default());
    fresh.with(|c, now| c.parse_list_sessions(&["$3 0 a", "$4 1 b"], now));
    assert_eq!(
        fresh.controller.snapshot().active_session_id.as_deref(),
        Some("$4")
    );
    let mut none = Rig::new(Config::default());
    none.with(|c, now| c.parse_list_sessions(&["$7 0 a", "$5 0 b"], now));
    assert_eq!(
        none.controller.snapshot().active_session_id.as_deref(),
        Some("$5")
    );
    let mut ten = Rig::new(Config::default());
    ten.with(|c, now| c.parse_list_sessions(&["$10 0 ten", "$2 0 two", "$11 0 eleven"], now));
    assert_eq!(
        ten.controller.snapshot().active_session_id.as_deref(),
        Some("$2")
    );
}

#[test]
fn the_sidecar_takes_no_session_from_attached_flags() {
    let mut sidecar = Rig::new(Config {
        renders_output: false,
        ..Config::default()
    });
    sidecar.with(|c, now| c.parse_list_sessions(&["$0 1 0"], now));
    assert_eq!(sidecar.controller.snapshot().active_session_id, None);
    sidecar.with(|c, now| c.parse_list_sessions(&["$0 0 0", "$1 1 dev"], now));
    assert_eq!(sidecar.controller.snapshot().active_session_id, None);
}

#[test]
fn killing_the_session_on_screen_falls_back_to_the_first_made() {
    let mut rig = Rig::new(Config::default());
    rig.with(|c, now| c.parse_list_sessions(&["$10 0 ten", "$2 0 two", "$3 1 three"], now));
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$3")
    );
    rig.with(|c, now| c.kill_session("$3", now));
    assert_eq!(
        rig.controller.snapshot().active_session_id.as_deref(),
        Some("$2")
    );
}

#[test]
fn a_refused_listing_keeps_what_was_listed() {
    let mut rig = attached();
    let refusing = std::rc::Rc::new(std::cell::Cell::new(false));
    let r = refusing.clone();
    rig.respond(Box::new(move |command, _| {
        if r.get() && command.starts_with("list-") {
            return err(["server exited unexpectedly"]);
        }
        if command.starts_with("list-sessions") {
            return ok(lines("$0 1 main\n$2 0 work"));
        }
        if command.starts_with("list-windows") {
            return ok(["$0 @0 0 b25d,80x24,0,0,0 1 1 main"]);
        }
        if command.starts_with("list-panes") && command.contains("@moshpit_state") {
            return ok(["%0|working|claude|1700000000|claude|fixing the scroll"]);
        }
        if command.starts_with("list-panes") {
            return ok(["%0 @0 0 80 24 1 0 0 bash"]);
        }
        None
    }));
    rig.settle();
    rig.with(|c, now| c.poll_agent_hooks(now));
    rig.settle();
    assert!(rig.controller.agent_hooks().contains_key("%0"));
    refusing.set(true);
    rig.with(|c, now| c.refresh(now));
    rig.with(|c, now| c.rename_session("$2", "work2", now));
    rig.push("%window-add @0\n");
    rig.with(|c, now| c.poll_agent_hooks(now));
    rig.settle();
    let snapshot = rig.controller.snapshot();
    assert_eq!(snapshot.sessions.keys().collect::<Vec<_>>(), ["$0", "$2"]);
    assert_eq!(snapshot.windows["@0"].name, "main");
    assert!(snapshot.panes.contains_key("%0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%0"));
    assert_eq!(
        rig.controller.agent_hooks()["%0"].state.as_deref(),
        Some("working")
    );
    // An answered listing with fewer rows still replaces.
    rig.with(|c, now| c.parse_list_sessions(&["$0 1 main"], now));
    assert_eq!(rig.controller.snapshot().sessions.len(), 1);
}

#[test]
fn a_dead_pane_loses_its_terminal_at_the_next_listing() {
    let mut rig = attached();
    discover(&mut rig, TWO_WINDOWS);
    rig.settle();
    assert!(rig.controller.stream_of("%0").is_some());
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-windows") {
            return ok(["$0 @0 0 81x24,0,0,0 1 1 main"]);
        }
        if command.starts_with("list-panes") && command.contains("pane_current_command") {
            return ok(["%0 @0 0 80 24 1 0 0 bash"]);
        }
        ok(["0 0"])
    }));
    rig.with(|c, now| c.kill_window("@1", now));
    rig.settle();
    assert_eq!(rig.controller.stream_of("%1"), None);
    assert!(rig.controller.stream_of("%0").is_some());
}

// MARK: - keys and paste

#[test]
fn input_goes_as_hex() {
    let mut rig = attached();
    rig.with(|c, now| c.send_input(&[0x61, 0x0A, 0x62], "%0", now));
    assert_eq!(rig.commands.last().unwrap(), "send-keys -t %0 -H 61 0a 62");
}

#[test]
fn a_lone_arrow_goes_as_its_key_name() {
    for (bytes, key) in [
        ([0x1B, b'[', b'A'], "Up"),
        ([0x1B, b'O', b'A'], "Up"),
        ([0x1B, b'[', b'B'], "Down"),
        ([0x1B, b'O', b'B'], "Down"),
        ([0x1B, b'[', b'C'], "Right"),
        ([0x1B, b'O', b'C'], "Right"),
        ([0x1B, b'[', b'D'], "Left"),
        ([0x1B, b'O', b'D'], "Left"),
    ] {
        let mut rig = attached();
        rig.with(|c, now| c.send_input(&bytes, "%0", now));
        assert_eq!(
            rig.commands.last().unwrap(),
            &format!("send-keys -t %0 {key}")
        );
    }
}

/// B360: what the view wrote between two reads of the pane's channel
/// comes as one chunk; held keys or a busy moment bring arrows together,
/// and each must still be a key name — sent as hex, tmux would get the
/// local cursor-key mode's bytes, which a fresh attach never learned.
#[test]
fn arrows_that_arrive_together_still_go_as_key_names() {
    let mut rig = attached();
    let before = rig.commands.len();
    rig.with(|c, now| c.send_input(b"\x1b[A\x1b[A\x1bOA", "%0", now));
    assert_eq!(
        rig.commands[before..],
        [
            "send-keys -t %0 Up",
            "send-keys -t %0 Up",
            "send-keys -t %0 Up"
        ],
        "key repeat"
    );
    let before = rig.commands.len();
    rig.with(|c, now| c.send_input(b"ls\x1b[Bx\x1b[D", "%0", now));
    assert_eq!(
        rig.commands[before..],
        [
            "send-keys -t %0 -H 6c 73",
            "send-keys -t %0 Down",
            "send-keys -t %0 -H 78",
            "send-keys -t %0 Left",
        ],
        "arrows among text, in the order typed"
    );
}

#[test]
fn a_modified_arrow_and_a_bare_escape_stay_hex() {
    let mut rig = attached();
    rig.with(|c, now| c.send_input(&[0x1B, 0x5B, 0x31, 0x3B, 0x35, 0x43], "%0", now));
    assert_eq!(
        rig.commands.last().unwrap(),
        "send-keys -t %0 -H 1b 5b 31 3b 35 43"
    );
    rig.with(|c, now| c.send_input(&[0x1B], "%0", now));
    assert_eq!(rig.commands.last().unwrap(), "send-keys -t %0 -H 1b");
}

#[test]
fn empty_input_and_bad_targets_send_nothing() {
    let mut rig = attached();
    let base = rig.commands.len();
    rig.with(|c, now| c.send_input(&[], "%0", now));
    rig.with(|c, now| c.send_input(b"x", "not-a-pane", now));
    rig.with(|c, now| c.send_paste("", "%0", now));
    rig.with(|c, now| c.send_paste("hello", "not-a-pane", now));
    rig.with(|c, now| c.send_paste("\u{1b}[201~", "%0", now));
    assert_eq!(rig.commands.len(), base);
}

#[test]
fn a_keystroke_keeps_its_place() {
    let mut rig = attached();
    rig.with(|c, _| c.stream_for_pane("%0"));
    rig.with(|c, now| c.view_input("%0", b"a", now));
    rig.with(|c, now| c.send_input(b"b", "%0", now));
    let keys: Vec<&String> = rig
        .commands
        .iter()
        .filter(|c| c.starts_with("send-keys"))
        .collect();
    assert_eq!(keys, ["send-keys -t %0 -H 61", "send-keys -t %0 -H 62"]);
    // Not while the transport is down.
    rig.with(|c, _| c.set_transport_live(false));
    rig.with(|c, now| c.view_input("%0", b"c", now));
    assert_eq!(rig.count("send-keys"), 2);
}

#[test]
fn paste_goes_through_a_buffer_bracketed_by_tmux() {
    let mut rig = attached();
    let base = rig.commands.len();
    rig.with(|c, now| c.send_paste("hello world", "%0", now));
    assert_eq!(
        rig.commands[base],
        "set-buffer -b moshpit-paste -- \"hello world\""
    );
    assert_eq!(
        rig.commands[base + 1],
        "paste-buffer -p -b moshpit-paste -d -t %0"
    );
    // The next paste waits for tmux to have run this one (B485).
    rig.settle();
    rig.wait(150);
    let base = rig.commands.len() - 2;
    let text = "line1\nline2 has $HOME, ~root, \"quote\", back\\slash, 中文🎉";
    rig.with(|c, now| c.send_paste(text, "%0", now));
    let set = &rig.commands[base + 2];
    assert_eq!(
        set,
        &format!(
            "set-buffer -b moshpit-paste -- {}",
            moshpit_tmux::format::command_string_literal(text)
        )
    );
    assert!(!set.contains('\n'));
}

/// B485: a key typed right behind a paste (the image panel's Send: the
/// path, then Return) goes to tmux only once tmux has run the paste
/// (paste-buffer's answer) and 150 ms more, so the two never reach the
/// program together however the network bunches them (Claude Code drops a
/// Return that comes with an image path's paste). In order; a paste among
/// what waits waits in turn; with no answer they go after 2 s.
#[test]
fn a_key_behind_a_paste_waits_for_tmux_to_have_run_it() {
    let mut rig = attached();
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.send_paste(" /tmp/a.jpg ", "%0", now));
    rig.with(|c, now| c.send_input(b"\r", "%0", now));
    assert_eq!(
        rig.since(base).len(),
        2,
        "the paste only: {:?}",
        rig.since(base)
    );
    rig.wait(500);
    assert_eq!(rig.since(base).len(), 2, "no answer yet");
    rig.answer_pending();
    rig.wait(100);
    assert_eq!(rig.since(base).len(), 2, "150 ms after the answer");
    rig.wait(60);
    assert_eq!(rig.since(base)[2..], ["send-keys -t %0 -H 0d"]);

    // No answer: after 2 s, in order up to the next paste, which the rest
    // waits behind.
    let base = rig.commands.len();
    rig.with(|c, now| c.send_paste("x", "%0", now));
    rig.with(|c, now| c.send_input(b"y", "%0", now));
    rig.with(|c, now| c.send_paste("z", "%0", now));
    rig.with(|c, now| c.send_input(b"w", "%0", now));
    rig.wait(1_900);
    assert_eq!(rig.since(base).len(), 2);
    rig.wait(200);
    assert_eq!(
        rig.since(base)[2..],
        [
            "send-keys -t %0 -H 79",
            "set-buffer -b moshpit-paste -- \"z\"",
            "paste-buffer -p -b moshpit-paste -d -t %0",
        ]
    );
    rig.settle();
    rig.wait(150);
    assert_eq!(rig.since(base).last().unwrap(), "send-keys -t %0 -H 77");
}

/// B485: a paste's answer that comes after its wait ran out (a slow link)
/// settles only that paste: what follows the next one still waits for the
/// next one's own answer. It settled whichever paste was waiting, so the
/// key behind the next paste went before tmux had run it.
#[test]
fn a_late_answer_settles_only_its_own_paste() {
    let mut rig = attached();
    rig.settle();
    let base = rig.commands.len();
    rig.with(|c, now| c.send_paste("x", "%0", now));
    rig.with(|c, now| c.send_paste("z", "%0", now));
    rig.with(|c, now| c.send_input(b"w", "%0", now));
    // x's wait runs out unanswered: z goes, and w waits behind it.
    rig.wait(2_100);
    assert_eq!(rig.since(base).len(), 4, "{:?}", rig.since(base));
    // x's two answers, late; z's not yet.
    for _ in 0..2 {
        let num = 9_000 + rig.answered as i64;
        rig.answered += 1;
        rig.push(&moshpit_tmux::rig::block(num, &[], false));
    }
    rig.wait(500);
    assert_eq!(rig.since(base).len(), 4, "w still waits for z's answer");
    rig.answer_pending();
    rig.wait(150);
    assert_eq!(rig.since(base).last().unwrap(), "send-keys -t %0 -H 77");
}

#[test]
fn paste_markers_come_out_of_the_text() {
    let mut rig = attached();
    let base = rig.commands.len();
    rig.with(|c, now| c.send_paste("a\u{1b}[20\u{1b}[201~1~b\u{1b}[201~\u{301}c", "%0", now));
    // Markers out (the escape itself becomes \u001b), the combining mark kept.
    assert_eq!(
        rig.commands[base],
        "set-buffer -b moshpit-paste -- \"ab\u{301}c\""
    );
}

/// B738: a paste's control characters went out as they were, tmux
/// turning `\u001b` back into ESC: an escape sequence ran as keys where
/// the pane's program hadn't asked for bracketed paste, a ^C or ^D in it hit
/// the tty as an interrupt or an end of file, bracketed or not. And
/// paste-buffer sends each line feed as a carriage return, so a CRLF text
/// went out CR CR, every line break doubled. Only tab, line feed and
/// carriage return go now, and one line feed a line break.
#[test]
fn a_paste_sends_no_control_and_one_return_a_line() {
    let mut rig = attached();
    let base = rig.commands.len();
    rig.with(|c, now| {
        c.send_paste(
            "a\r\nb\nc\u{1b}[31mred\u{3}\u{4}\u{7f}\u{9b}2Jend\tx",
            "%0",
            now,
        )
    });
    assert_eq!(
        rig.commands[base],
        "set-buffer -b moshpit-paste -- \"a\\nb\\nc[31mred2Jend\\tx\""
    );
    // Tabs and line breaks stay (each line break one line feed, which
    // paste-buffer sends as Return); neither an end marker nor the 8-bit
    // CSI reaches the buffer.
    for (pasted, buffer) in [
        ("a\tb\r\nc\nd", "\"a\\tb\\nc\\nd\""),
        ("x\u{1b}[201~y", "\"xy\""),
        ("x\u{9b}201~y", "\"x201\\~y\""),
    ] {
        rig.settle();
        rig.wait(150);
        let base = rig.commands.len();
        rig.with(|c, now| c.send_paste(pasted, "%0", now));
        assert_eq!(
            rig.commands[base],
            format!("set-buffer -b moshpit-paste -- {buffer}"),
            "{pasted:?}"
        );
    }
}

// MARK: - lifecycle

#[test]
fn waiting_requests_hear_when_the_channel_closes() {
    for close in ["detach", "exit", "stream"] {
        let mut rig = attached();
        rig.push("%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n%end 2 2 1\n%begin 3 3 1\n%end 3 3 1\n");
        let request = rig.with(|c, now| c.count_non_control_clients(now));
        match close {
            "detach" => rig.with(|c, now| c.detach(now)),
            "exit" => rig.push("%exit\n"),
            _ => rig.with(|c, now| c.channel_closed(now)),
        }
        assert!(
            rig.notes()
                .contains(&Notification::ClientCount { request, count: -1 }),
            "{close}"
        );
    }
}

#[test]
fn after_exit_the_next_session_pairs_its_own_replies() {
    let mut rig = attached();
    rig.push(
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n%end 2 2 1\n%begin 3 3 1\n%end 3 3 1\n",
    );
    rig.with(|c, now| c.count_non_control_clients(now));
    rig.push("%exit\n");
    assert!(!rig.controller.snapshot().is_attached);
    rig.answered = rig.commands.len();
    rig.push("%begin 300 300 1\n%end 300 300 1\n%session-changed $7 again\n");
    rig.respond(Box::new(|command, _| {
        if command.starts_with("list-sessions") {
            return ok(lines("$7 1 again\n$8 0 other"));
        }
        None
    }));
    rig.settle();
    assert_eq!(rig.controller.snapshot().sessions["$8"].name, "other");
    assert!(!rig.controller.snapshot().sessions.contains_key("$0"));
}

#[test]
fn the_first_revealed_pane_is_the_first_frame_once() {
    let mut rig = attached();
    discover(
        &mut rig,
        "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 81x24,0,0,0 1 1 main\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%1 @0 1 80 24 0 0 0 vim\n%end 3 3 1\n\n",
    );
    assert!(!rig.controller.has_painted_since_attach());
    rig.with(|c, now| c.view_revealed("%0", now));
    rig.with(|c, now| c.view_revealed("%0", now));
    rig.with(|c, now| c.view_revealed("%1", now));
    assert!(rig.controller.has_painted_since_attach());
    let firsts = rig
        .notes()
        .iter()
        .filter(|n| **n == Notification::FirstFramePainted)
        .count();
    assert_eq!(firsts, 1);
}

#[test]
fn agent_hooks_hand_over_only_once_read() {
    let never = Rig::new(Config::default());
    assert!(!never.controller.agent_hooks_loaded());
    assert_eq!(never.controller.take_agent_hooks(), None);
    let mut rig = attached();
    rig.push("%begin 1 1 1\n%end 1 1 1\n%begin 2 2 1\n%end 2 2 1\n%begin 3 3 1\n%end 3 3 1\n");
    rig.with(|c, now| c.poll_agent_hooks(now));
    rig.push("%begin 9 9 1\n%5|working|claude|1758000000|2.1.269|\n%end 9 9 1\n");
    assert!(rig.controller.agent_hooks_loaded());
    assert_eq!(
        rig.controller.agent_hooks()["%5"].agent.as_deref(),
        Some("claude")
    );
    let handed = rig.controller.take_agent_hooks().unwrap();
    let mut replacement = Rig::new(Config::default());
    replacement.with(|c, _| c.inherit_agent_hooks(handed));
    assert!(replacement.controller.agent_hooks_loaded());
    replacement.with(|c, _| c.inherit_agent_hooks(Default::default()));
    assert!(replacement.controller.agent_hooks().contains_key("%5"));
}

#[test]
fn the_watchdog_declares_a_silent_channel_dead() {
    let mut rig = attached();
    // Three discovery commands unanswered, nothing incoming after the boot
    // block: the first beat takes that as its baseline, three silent beats
    // after it are death.
    for _ in 0..3 {
        rig.wait(4000);
    }
    assert!(
        !rig.notes()
            .iter()
            .any(|n| matches!(n, Notification::ChannelDead { .. }))
    );
    rig.wait(4000);
    assert!(rig.notes().contains(&Notification::ChannelDead {
        reason: "silent".into()
    }));
}

/// A suspension (iOS runs nothing) skips the watchdog's beats instead of
/// replaying them: the overdue beat fires once, the replies waiting in the
/// socket are read, and a live channel survives any time away. Replayed
/// beat by beat, three "silent" beats came before the first byte was read
/// whenever a command was unanswered at the suspension (the pins handed
/// back on the way out): every return after 12 s or more tore it down.
#[test]
fn a_suspension_skips_the_watchdogs_beats_instead_of_replaying_them() {
    let dead = |rig: &Rig| {
        rig.notes()
            .iter()
            .any(|n| matches!(n, Notification::ChannelDead { .. }))
    };
    for away in [12_000, 30_000, 300_000] {
        let mut rig = attached();
        rig.settle();
        rig.wait(4000);
        // Written on the way out, unanswered when the app is suspended.
        rig.with(|c, now| c.request_pane_path("%0", now));
        // Nothing ran meanwhile; the reply waits in the socket.
        rig.now += away;
        rig.answer_pending();
        assert!(!dead(&rig), "{away} ms away: {:?}", rig.notes());
        // Still a watchdog: a channel silent from here on dies as before.
        rig.with(|c, now| c.request_pane_path("%0", now));
        for _ in 0..5 {
            rig.wait(4000);
        }
        assert!(
            rig.notes().contains(&Notification::ChannelDead {
                reason: "silent".into()
            }),
            "{away} ms away, then silent: {:?}",
            rig.notes()
        );
    }
}

#[test]
fn the_watchdog_leaves_an_answering_channel_alone() {
    let mut rig = attached();
    rig.settle();
    for _ in 0..12 {
        rig.wait(4000);
    }
    assert!(
        !rig.notes()
            .iter()
            .any(|n| matches!(n, Notification::ChannelDead { .. }))
    );
}

#[test]
fn pane_paths_are_kept_as_printed() {
    let mut rig = attached();
    rig.settle();
    let request = rig.with(|c, now| c.request_pane_path("%0", now));
    rig.push("%begin 5 5 1\n/home/me/proj \n%end 5 5 1\n");
    assert!(rig.notes().contains(&Notification::PanePath {
        request,
        path: Some("/home/me/proj ".into())
    }));
    let relative = rig.with(|c, now| c.request_pane_path("%0", now));
    rig.push("%begin 6 6 1\n\u{301}x\n%end 6 6 1\n");
    assert!(rig.notes().contains(&Notification::PanePath {
        request: relative,
        path: None
    }));
    let silent = rig.with(|c, now| c.request_pane_path("%0", now));
    rig.wait(4100);
    assert!(rig.notes().contains(&Notification::PanePath {
        request: silent,
        path: None
    }));
}

// MARK: - Weak-network recovery, as the controller sees it

#[test]
fn a_begin_glued_onto_truncated_output_is_recovered() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    let paired = rig.commands.len();
    // A lossy hop cut the %output line and glued the next reply's %begin on.
    rig.push("%output %0 visible%begin 900 950 1\n%end 900 950 1\n");
    let fed = text(&rig.pane_bytes("%0"));
    assert!(
        fed.ends_with("visible"),
        "the payload before the glue paints: {fed:?}"
    );
    assert!(!fed.contains("%begin"));
    rig.push("%output %0 -after\n");
    assert!(text(&rig.pane_bytes("%0")).ends_with("visible-after"));
    assert_eq!(rig.commands.len(), paired);
}

#[test]
fn pane_content_quoting_an_old_begin_paints_verbatim() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    rig.push("%output %0 quote:%begin 100 5 1\n");
    assert!(text(&rig.pane_bytes("%0")).ends_with("quote:%begin 100 5 1"));
}

#[test]
fn a_terminator_glued_onto_content_still_closes_the_block() {
    let mut rig = attached_at(69, 60);
    one_window(&mut rig, false, false);
    rig.settle();
    rig.with(|c, now| c.resync_active_pane_now(true, now));
    rig.push("%begin 400 400 1\nROW-A%end 400 400 1\n%begin 401 401 1\n0 0\n%end 401 401 1\n");
    rig.answered = rig.commands.len();
    assert!(text(&rig.pane_bytes("%0")).contains(&format!("{BLANK}\u{1b}[HROW-A\u{1b}[0m")));
    rig.push("%output %0 alive\n");
    assert!(text(&rig.pane_bytes("%0")).ends_with("alive"));
}

// MARK: - Windows closed by another client (TmuxUnlinkedWindowCloseTests)

const BY_INDEX: &str = "%begin 1 1 1\n$0 1 main\n%end 1 1 1\n%begin 2 2 1\n$0 @0 0 b25d,80x24,0,0,0 0 1 main\n$0 @2 1 b25e,80x24,0,0,2 0 1 logs\n$0 @1 2 b25f,80x24,0,0,1 1 1 vim\n%end 2 2 1\n%begin 3 3 1\n%0 @0 0 80 24 1 0 0 bash\n%2 @2 0 80 24 1 0 0 tail\n%1 @1 0 80 24 1 0 0 vim\n%end 3 3 1\n\n";

#[test]
fn a_window_killed_elsewhere_leaves_the_tree_with_its_pane() {
    let mut rig = attached();
    discover(
        &mut rig,
        &TWO_WINDOWS.replace("%1 @1 0 80 24 1 0 0 vim", "%1 @1 0 80 24 1 0 0 tail"),
    );
    rig.push("%unlinked-window-close @1\n");
    let snapshot = rig.controller.snapshot();
    assert!(!snapshot.windows.contains_key("@1"));
    assert!(!snapshot.panes.contains_key("%1"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@0"));
}

#[test]
fn the_window_on_screen_killed_elsewhere_lands_on_the_survivor() {
    let mut rig = attached();
    discover(&mut rig, TWO_WINDOWS);
    rig.push("%session-window-changed $0 @1\n%unlinked-window-close @0\n");
    let snapshot = rig.controller.snapshot();
    assert!(!snapshot.windows.contains_key("@0"));
    assert_eq!(snapshot.active_window_id.as_deref(), Some("@1"));
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%1"));
}

#[test]
fn the_window_on_screen_killed_elsewhere_lands_where_tmux_did() {
    let mut rig = attached();
    discover(&mut rig, BY_INDEX);
    assert_eq!(
        rig.controller.snapshot().active_window_id.as_deref(),
        Some("@1")
    );
    rig.push("%session-window-changed $0 @2\n%unlinked-window-close @1\n");
    let snapshot = rig.controller.snapshot();
    assert!(!snapshot.windows.contains_key("@1"));
    assert_eq!(
        snapshot.active_window_id.as_deref(),
        Some("@2"),
        "tmux's choice, not the first by index"
    );
    assert_eq!(snapshot.active_pane_id.as_deref(), Some("%2"));
}

// MARK: - the ping (the way back from the background)

fn pongs(rig: &Rig) -> Vec<(u64, bool)> {
    rig.notes()
        .into_iter()
        .filter_map(|note| match note {
            Notification::Pong { request, alive } => Some((request, alive)),
            _ => None,
        })
        .collect()
}

/// One round trip on the control channel: answered after the output tmux
/// queued before it (a suspension's backlog) has reached the pane, and not
/// before its own reply.
#[test]
fn a_ping_is_answered_after_the_output_queued_before_it() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let request = rig.with(|c, now| c.ping(now));
    assert_eq!(rig.commands.last().unwrap(), "display-message -p ok");
    assert!(pongs(&rig).is_empty(), "no answer before tmux's");
    rig.push("%output %0 while-you-were-away\\015\\012\n");
    assert!(pongs(&rig).is_empty());
    rig.settle();
    assert_eq!(pongs(&rig), vec![(request, true)]);
    let bytes = text(&rig.pane_bytes("%0"));
    assert!(bytes.contains("while-you-were-away"), "{bytes}");
}

/// tmux refusing the command still answered: the channel lives.
#[test]
fn a_refused_ping_is_still_an_answer() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    rig.respond(Box::new(|command, _| {
        if command == "display-message -p ok" {
            err(["no current client"])
        } else {
            ok(["0 0"])
        }
    }));
    let request = rig.with(|c, now| c.ping(now));
    rig.settle();
    assert_eq!(pongs(&rig), vec![(request, true)]);
}

/// The channel closing first, or nothing attached to ask: not alive.
#[test]
fn a_ping_the_channel_never_answers_is_not_alive() {
    let mut rig = attached();
    one_window(&mut rig, false, false);
    rig.settle();
    let request = rig.with(|c, now| c.ping(now));
    rig.with(|c, now| c.channel_closed(now));
    assert_eq!(pongs(&rig), vec![(request, false)]);

    let mut idle = Rig::new(Config::default());
    let request = idle.with(|c, now| c.ping(now));
    assert_eq!(pongs(&idle), vec![(request, false)]);
    assert!(idle.commands.is_empty(), "nothing sent before an attach");
}
