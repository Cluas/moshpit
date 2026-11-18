//! The terminal page's host side, for W11g's NSH terminal screen
//! (`ui/src/terminal`, the one iOS runs too): what
//! its chrome and shortcut bar show (`TerminalScreen.swift`'s `nshChrome`
//! and `nshBar`, read from the Rust session), and what the person's
//! intents do. The terminal itself is the host's view in the screen's slot
//! (`moshpit/terminal`): the controller tells the host which pane's stream
//! the slot of a page shows ([`crate::platform::Platform::show_stream`]).
//!
//! Android has the screen's shell, covers, banners, notice and shortcut bar,
//! over tmux and over herdr (W2b's client on the Rust session: its tree in
//! the breadcrumb, its empty state, its notices and their one-tap remedy),
//! the breadcrumb's pickers (W11h's screen, [`super::picker`]), ⌘1–⌘9, and
//! the pill's switch (the page confirms it on its own sheet); not yet
//! mosh's counters (the session doesn't expose them), the host folder,
//! voice, images and previews. The banner's and the empty state's Install
//! open W11h's Install Assist ([`super::install_assist`]).

use moshpit_herdr::frame::ScrollSource;
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_session::{Degrade, HerdrAction, SessionState, TmuxAction};
use moshpit_store::connections::{ConnectionProtocol, Multiplexer};
use moshpit_store::shortcuts::ShortcutKind;
use moshpit_ui::terminal::model::{
    Banner, Bar, Chip, ChipKind, Chrome, Content, Cover, CoverKind, Crumbs, Empty, LinkState,
    Missing, Mux, Notice, PillKind,
};
use serde_json::Value;

use crate::app::sessions::Conn;
use crate::app::{App, Core, PageKind};

/// What a terminal page keeps of its own.
#[derive(Debug, Clone)]
pub struct TerminalPage {
    pub connection: Uuid,
    /// The person put the keyboard away (the bar's toggle offers it back).
    pub keyboard_down: bool,
    /// The sticky ctrl chip is armed for the next key.
    pub ctrl_armed: bool,
    /// The banner the person closed (until another one shows).
    pub banner_dismissed: Option<String>,
    /// The stream the host's slot shows now (0: none).
    pub shown_stream: u64,
}

impl TerminalPage {
    pub fn new(connection: Uuid) -> Self {
        TerminalPage {
            connection,
            keyboard_down: false,
            ctrl_armed: false,
            banner_dismissed: None,
            shown_stream: 0,
        }
    }
}

fn link_state(state: Option<&SessionState>) -> LinkState {
    match state {
        None | Some(SessionState::Connecting) => LinkState::Connecting,
        Some(SessionState::Connected) => LinkState::Live,
        Some(SessionState::Reconnecting) => LinkState::Reconnecting,
        Some(SessionState::Ended(_) | SessionState::Failed(_)) => LinkState::Offline,
    }
}

/// `{"op": op, …}` of a model part.
pub fn json(op: &str, part: &impl serde::Serialize) -> String {
    let mut value = serde_json::to_value(part).expect("a model part serializes");
    value["op"] = Value::String(op.to_owned());
    value.to_string()
}

/// The multiplexer the saved connection drives.
fn chrome_mux(app: &App, connection: Uuid) -> Mux {
    match app
        .connections
        .connections()
        .iter()
        .find(|c| c.id == connection)
        .map(|c| c.multiplexer())
    {
        Some(Multiplexer::Tmux) => Mux::Tmux,
        Some(Multiplexer::Herdr) => Mux::Herdr,
        _ => Mux::None,
    }
}

/// `{"op": "chrome", …}` for `page`.
pub fn chrome(app: &App, page: &TerminalPage) -> Chrome {
    let saved = app
        .connections
        .connections()
        .iter()
        .find(|c| c.id == page.connection);
    let conn = app.sessions.conn(page.connection);
    let state = link_state(conn.map(|c| &c.state));
    let mux = match saved.map(|c| c.multiplexer()) {
        Some(Multiplexer::Tmux) => Mux::Tmux,
        Some(Multiplexer::Herdr) => Mux::Herdr,
        _ => Mux::None,
    };
    let mosh = saved.is_some_and(|c| c.connection_protocol == ConnectionProtocol::Mosh);
    let degraded_to_ssh = conn.is_some_and(|c| c.degrade == Some(Degrade::MoshServerMissing));
    let mut chrome = Chrome {
        name: saved.map(|c| c.display_name()).unwrap_or_default(),
        user: saved.map(|c| c.username.clone()).unwrap_or_default(),
        host: saved.map(|c| c.host.clone()).unwrap_or_default(),
        port: saved.map_or(22, |c| u32::try_from(c.port).unwrap_or(22)),
        mosh,
        mux,
        pill: match (mosh && !degraded_to_ssh, conn.is_some_and(|c| c.roaming)) {
            (true, true) => PillKind::MoshRoaming,
            (true, false) => PillKind::Mosh,
            _ => PillKind::Ssh,
        },
        state,
        ..Chrome::default()
    };
    // A first connect has no cover (B657): the pane is the terminal's
    // background, the transport pill says it connects.
    let Some(conn) = conn else {
        return chrome;
    };
    chrome.crumbs = crumbs(conn).or_else(|| herdr_crumbs(conn));
    let ended = matches!(conn.state, SessionState::Ended(_));
    if conn.ever_live && state != LinkState::Live {
        chrome.cover = Cover {
            kind: CoverKind::Veil,
            state,
            ended,
            ..Cover::default()
        };
    }
    (chrome.content, chrome.empty) = content(conn, mux, state);
    chrome.banner = if conn.return_path_dead {
        // What was typed waiting for mosh-server (B854) isn't followed on
        // Android yet: the banner says the outage alone.
        Some(Banner::ReturnPath { typed: false, route: None, ports: String::new() })
    } else if conn.roaming {
        Some(Banner::Roam {
            srtt: conn.srtt_ms,
            history: Vec::new(),
        })
    } else {
        conn.degrade.map(|d| Banner::Degrade {
            missing: match d {
                Degrade::MoshServerMissing => Missing::MoshServer,
                Degrade::TmuxMissing => Missing::Tmux,
                Degrade::HerdrMissing | Degrade::HerdrUnavailable => Missing::Herdr,
            },
        })
    };
    if let (Some(banner), Some(dismissed)) = (&chrome.banner, &page.banner_dismissed)
        && banner_key(banner) == *dismissed
    {
        chrome.banner = None;
    }
    // A failed attempt says why, with the person's retry (the iOS screen
    // shows its failure card from SwiftUI).
    chrome.notice = App::failure_text(&conn.state)
        .map(|text| Notice {
            text,
            action: Some("Retry".to_owned()),
        })
        .or_else(|| {
            ended.then(|| Notice {
                text: "The session ended.".to_owned(),
                action: Some("Reconnect".to_owned()),
            })
        })
        .or_else(|| {
            conn.tmux
                .as_ref()
                .and_then(|t| t.notice.clone())
                .map(|text| Notice { text, action: None })
        })
        .or_else(|| {
            // herdr's banner and its remedy (restart the server, retry,
            // take the pane over from another device).
            conn.herdr
                .as_ref()
                .and_then(|h| h.notice.as_ref())
                .map(|(notice, action)| Notice {
                    text: notice.english(),
                    action: action.map(|a| a.english().to_owned()),
                })
        });
    chrome
}

fn banner_key(banner: &Banner) -> String {
    match banner {
        Banner::ReturnPath { .. } => "return-path".to_owned(),
        Banner::Roam { .. } => "roam".to_owned(),
        Banner::Degrade { missing } => format!("degrade-{missing:?}"),
    }
}

/// tmux's breadcrumb: session, window, pane.
fn crumbs(conn: &Conn) -> Option<Crumbs> {
    let snapshot = &conn.tmux.as_ref()?.snapshot;
    let session = snapshot
        .active_session_id
        .as_ref()
        .and_then(|id| snapshot.sessions.get(id))?;
    let window = snapshot
        .active_window_id
        .as_ref()
        .and_then(|id| snapshot.windows.get(id));
    let pane = snapshot
        .active_pane_id
        .as_ref()
        .and_then(|id| snapshot.panes.get(id));
    Some(Crumbs {
        session: session.name.clone(),
        window: window.map_or_else(String::new, |w| format!("{}:{}", w.index, w.name)),
        pane: pane.map(|p| {
            if p.command.is_empty() {
                p.id.clone()
            } else {
                p.command.clone()
            }
        }),
        session_icon: "square.stack.3d.up".to_owned(),
        window_icon: "macwindow".to_owned(),
        live: conn.state.is_live(),
        ..Crumbs::default()
    })
}

/// herdr's breadcrumb: workspace, tab, pane (its vocabulary's icons).
fn herdr_crumbs(conn: &Conn) -> Option<Crumbs> {
    let snapshot = &conn.herdr.as_ref()?.snapshot;
    let session = snapshot
        .active_session_id
        .as_ref()
        .and_then(|id| snapshot.sessions.get(id))?;
    let window = snapshot
        .active_window_id
        .as_ref()
        .and_then(|id| snapshot.windows.get(id));
    let pane = snapshot
        .active_pane_id
        .as_ref()
        .and_then(|id| snapshot.panes.get(id));
    Some(Crumbs {
        session: session.name.clone(),
        window: window.map_or_else(String::new, |w| format!("{}:{}", w.index, w.name)),
        pane: pane.map(|p| {
            if p.command.is_empty() {
                p.id.clone()
            } else {
                p.command.clone()
            }
        }),
        session_icon: "rectangle.grid.2x2".to_owned(),
        window_icon: "square.on.square".to_owned(),
        live: conn.state.is_live(),
        ..Crumbs::default()
    })
}

/// What the pane area holds (`nshContent`).
fn content(conn: &Conn, mux: Mux, state: LinkState) -> (Content, Option<Empty>) {
    if mux == Mux::Herdr {
        // herdr is installed and its server isn't running: no pane to show,
        // the empty state's Create starts one.
        if conn.herdr.as_ref().is_some_and(|h| h.server_not_running) {
            return (
                Content::Empty,
                Some(Empty {
                    mux: Mux::Herdr,
                    missing: conn.degrade == Some(Degrade::HerdrMissing),
                    // The server answered that it isn't running: known.
                    settled: true,
                    session: "Workspace".to_owned(),
                    session_lower: "workspace".to_owned(),
                    sessions_lower: "workspaces".to_owned(),
                }),
            );
        }
        return (Content::Terminal, None);
    }
    if mux != Mux::Tmux || conn.degrade == Some(Degrade::TmuxMissing) {
        return (Content::Terminal, None);
    }
    let Some(tmux) = &conn.tmux else {
        return (
            if conn.ever_live {
                Content::Blank
            } else {
                Content::None
            },
            None,
        );
    };
    if tmux.snapshot.is_attached {
        return (Content::Terminal, None);
    }
    if conn.ever_live && state != LinkState::Live {
        return (Content::Blank, None);
    }
    if !tmux.snapshot.sessions.is_empty()
        || !tmux.snapshot.ever_attached && state != LinkState::Live
    {
        return (Content::None, None);
    }
    (
        Content::Empty,
        Some(Empty {
            mux: Mux::Tmux,
            missing: false,
            // Known once the server said it has no session (B228, B337);
            // no sessions in the tree alone may be an attach on its way.
            settled: tmux.no_session,
            session: "Session".to_owned(),
            session_lower: "session".to_owned(),
            sessions_lower: "sessions".to_owned(),
        }),
    )
}

/// `{"op": "bar", …}` for `page`: the shortcut bar of its connection.
pub fn bar(app: &App, page: &TerminalPage) -> Bar {
    let name = app
        .connections
        .connections()
        .iter()
        .find(|c| c.id == page.connection)
        .map(|c| c.display_name());
    let in_multiplexer = app
        .sessions
        .conn(page.connection)
        .is_some_and(|c| c.tmux.is_some() || c.herdr.is_some());
    let chips = app
        .shortcuts
        .toolbar_for(name.as_deref(), in_multiplexer)
        .into_iter()
        .filter_map(|s| {
            let kind = match s.kind {
                ShortcutKind::KeyCombo => ChipKind::Key,
                ShortcutKind::Text | ShortcutKind::Command => ChipKind::Text,
                ShortcutKind::Ctrl => ChipKind::Ctrl,
                ShortcutKind::Paste => ChipKind::Paste,
                ShortcutKind::Arrows => ChipKind::Arrows,
                ShortcutKind::Scroll => ChipKind::Scroll,
                ShortcutKind::Dpad => ChipKind::Dpad,
                // Not on Android yet: voice, images, the folder sheet.
                ShortcutKind::Mic | ShortcutKind::Image | ShortcutKind::Files => return None,
            };
            Some(Chip {
                id: s.id.to_string(),
                kind,
                label: s.chip_label.clone(),
            })
        })
        .collect();
    Bar {
        chips,
        keyboard_down: page.keyboard_down,
        ctrl_armed: page.ctrl_armed,
        voice: false,
        mic_active: false,
        ..Bar::default()
    }
}

/// What the terminal page told the host.
pub fn on_event(app: &mut App, core: &Core, surface: u64, ev: &str, event: &Value) {
    let Some(PageKind::Terminal(page)) = app.page_mut(surface).map(|p| &mut p.kind) else {
        return;
    };
    let connection = page.connection;
    match ev {
        "back" | "sidebar" => {
            app.close(surface);
            app.sessions.set_visible(None);
            return;
        }
        // The breadcrumb's segments: their pickers (W11h's screen).
        "crumb" => {
            if let Some(crumb) = event["crumb"].as_str() {
                app.open_picker(core, connection, crumb);
            }
            return;
        }
        // ⌘1–⌘9 on a keyboard: the n-th window in the picker's order.
        "window" => {
            let window = event["number"].as_u64().and_then(|n| {
                app.sessions
                    .conn(connection)
                    .and_then(|c| crate::app::picker::nth_window(c, n as usize))
            });
            if let Some(window) = window {
                app.mux_action(
                    connection,
                    TmuxAction::SelectWindow(window.clone()),
                    HerdrAction::SelectWindow(window),
                );
            }
        }
        "keyboard" => {
            page.keyboard_down = !page.keyboard_down;
            let show = !page.keyboard_down;
            app.platform.keyboard(show);
        }
        "banner-action" => {
            let conn = app.sessions.conn(connection);
            let returning = conn.is_some_and(|c| c.return_path_dead);
            let degrade = conn.and_then(|c| c.degrade);
            if returning {
                app.sessions.switch_to_ssh(connection);
            } else if let Some(degrade) = degrade {
                let package = match degrade {
                    Degrade::MoshServerMissing => "mosh",
                    Degrade::TmuxMissing => "tmux",
                    Degrade::HerdrMissing | Degrade::HerdrUnavailable => "herdr",
                };
                app.open_install_assist(core, connection, vec![package.to_owned()]);
                return;
            }
        }
        // The empty state's Install: the multiplexer the connection drives.
        "install" => {
            let package =
                if app.sessions.is_herdr(connection) || chrome_mux(app, connection) == Mux::Herdr {
                    "herdr"
                } else {
                    "tmux"
                };
            app.open_install_assist(core, connection, vec![package.to_owned()]);
            return;
        }
        "banner-dismiss" => {
            let banner = chrome(app, &TerminalPage::new(connection)).banner;
            if let Some(PageKind::Terminal(page)) = app.page_mut(surface).map(|p| &mut p.kind) {
                page.banner_dismissed = banner.as_ref().map(banner_key);
            }
        }
        "take-over" => app.sessions.tmux(connection, TmuxAction::TakeSizeOwnership),
        "create" => {
            if app.sessions.is_herdr(connection) {
                // A workspace, or a server when there is none.
                app.sessions
                    .herdr(connection, HerdrAction::NewSession { name: None });
            } else {
                app.sessions
                    .tmux(connection, TmuxAction::NewSession { name: None });
            }
        }
        "chip" => {
            let id = super::uuid(event, "id");
            chip(app, surface, connection, id);
        }
        "arrow" => {
            let key = match event["dir"].as_str() {
                Some("up") => b'A',
                Some("down") => b'B',
                Some("right") => b'C',
                Some("left") => b'D',
                _ => return,
            };
            type_bytes(app, connection, &[0x1b, b'[', key]);
        }
        "scroll" => {
            let lines = match event["dir"].as_str() {
                Some("up") => 3,
                Some("down") => -3,
                _ => return,
            };
            if app.sessions.is_herdr(connection) {
                let pane = app
                    .sessions
                    .conn(connection)
                    .and_then(crate::app::sessions::active_pane)
                    .map(|p| p.pane.clone());
                if let Some(pane) = pane {
                    app.sessions.herdr(
                        connection,
                        HerdrAction::Scroll {
                            pane,
                            lines,
                            source: ScrollSource::Wheel,
                        },
                    );
                }
            } else {
                app.sessions
                    .tmux(connection, TmuxAction::ScrollLines(lines));
            }
        }
        "diagnostics" => {
            let srtt = app.sessions.conn(connection).and_then(|c| c.srtt_ms);
            app.platform.toast(&match srtt {
                Some(ms) => format!("mosh: smoothed round trip {ms:.0} ms"),
                None => "No mosh counters for this session".to_owned(),
            });
        }
        "notice-action" => {
            let conn = app.sessions.conn(connection);
            let failed = conn.is_some_and(|c| {
                matches!(c.state, SessionState::Failed(_) | SessionState::Ended(_))
            });
            let herdr_remedy = conn
                .and_then(|c| c.herdr.as_ref())
                .is_some_and(|h| h.notice.as_ref().is_some_and(|(_, a)| a.is_some()));
            if failed {
                app.connect(connection);
            } else if herdr_remedy {
                app.sessions.herdr(connection, HerdrAction::NoticeAction);
            }
        }
        // The pill's switch, once the page's own sheet confirmed it.
        "switch-transport" => {
            let to_mosh = app
                .connections
                .connections()
                .iter()
                .find(|c| c.id == connection)
                .is_some_and(|c| c.connection_protocol != ConnectionProtocol::Mosh);
            app.switch_transport(core, connection, to_mosh);
        }
        "open-link" | "mic" | "image" | "image-paste" | "image-camera" | "image-sent"
        | "dictation" | "attachment" | "preview" => {
            app.platform.toast("Not on Android yet");
        }
        // The stage's business on iOS (the keyboard across a rebuild, the
        // popovers' anchors): nothing to do here.
        _ => return,
    }
    app.refresh_terminals(connection);
    let _ = core;
}

/// A shortcut chip: its bytes into the pane, as typed (paste and ctrl are
/// resolved here).
fn chip(app: &mut App, surface: u64, connection: Uuid, id: Option<Uuid>) {
    let Some(shortcut) = id.and_then(|id| app.shortcuts.shortcuts().iter().find(|s| s.id == id))
    else {
        return;
    };
    match shortcut.kind {
        ShortcutKind::Ctrl => {
            if let Some(PageKind::Terminal(page)) = app.page_mut(surface).map(|p| &mut p.kind) {
                page.ctrl_armed = !page.ctrl_armed;
                let armed = page.ctrl_armed;
                app.platform.set_ctrl(armed);
            }
        }
        ShortcutKind::Paste => app.platform.paste(),
        _ => {
            for stage in shortcut.encoded_stages() {
                type_bytes(app, connection, &stage);
            }
        }
    }
}

/// `bytes` into `connection`'s pane on screen, after what was typed before.
fn type_bytes(app: &App, connection: Uuid, bytes: &[u8]) {
    if let Some(stream) = app.sessions.stream(connection) {
        moshpit_stream::view::write(stream, bytes);
    }
}

impl App {
    /// The pill's switch, confirmed on the page's own sheet: the saved
    /// connection takes the other transport, its session goes, and a new
    /// one dials over it (`switchProtocol`).
    pub(crate) fn switch_transport(&mut self, core: &Core, connection: Uuid, to_mosh: bool) {
        let Some(mut saved) = self
            .connections
            .connections()
            .iter()
            .find(|c| c.id == connection)
            .cloned()
        else {
            return;
        };
        saved.connection_protocol = if to_mosh {
            ConnectionProtocol::Mosh
        } else {
            ConnectionProtocol::Ssh
        };
        self.connections.update(saved);
        log::info!(target: "moshpit", "terminal: switching to {}", if to_mosh { "mosh" } else { "ssh" });
        // The keyboard goes down while the new session dials, as on iOS.
        for page in &mut self.pages {
            if let PageKind::Terminal(t) = &mut page.kind
                && t.connection == connection
            {
                t.keyboard_down = true;
            }
        }
        self.platform.keyboard(false);
        if self.sessions.conn(connection).is_none() {
            self.connect(connection);
            self.refresh_terminals(connection);
            return;
        }
        self.redial(core, connection);
        self.refresh_terminals(connection);
    }

    /// The sticky ctrl was used by a key: the chip comes off.
    pub fn ctrl_used(&mut self) {
        let mut changed = Vec::new();
        for page in &mut self.pages {
            if let PageKind::Terminal(terminal) = &mut page.kind
                && terminal.ctrl_armed
            {
                terminal.ctrl_armed = false;
                changed.push(terminal.connection);
            }
        }
        for connection in changed {
            self.refresh_terminals(connection);
        }
    }
}
