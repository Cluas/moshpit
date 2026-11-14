//! What the terminal screen shows, as the host posts it.
//!
//! The host owns the session (`SessionHub.ActiveSession`, on either session
//! path: the Swift transports or the Rust core's, whose controllers mirror
//! the Swift ones' observable models) and the screen's own lifecycle state
//! (whether it has ever been live, the keyboard the person asked for, the
//! sheets it presents). It reads them as `TerminalScreen.swift` does and
//! posts what the chrome draws, already decided where the decision is the
//! session's (which cover is up, which breadcrumb plan holds, which banner
//! wins the slot): the page lays it out and animates it, and tells the host
//! what the person does ([`super::Intent`]).
//!
//! Four messages, so a change redraws only its part:
//!
//! - `{"op": "chrome", …}` ([`Chrome`]): the top bar, the cover over the
//!   pane, what the pane area holds, the banner, the notice, the link chip.
//! - `{"op": "bar", …}` ([`Bar`]): the shortcut bar.
//! - `{"op": "dictation", …}` ([`Dictation`]) and `{"op": "level", …}`: the
//!   voice input panel and its meter.
//! - `{"op": "attachment", …}` ([`Attachment`]), `{"op": "preview", …}`
//!   ([`Preview`]): the image upload panel and a host file's preview panel.

use serde::{Deserialize, Serialize};

/// The transport's state (`TransportConnState`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    #[default]
    Live,
    Connecting,
    Reconnecting,
    Offline,
}

/// The transport pill's kind (`TransportPillKind`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PillKind {
    #[default]
    Ssh,
    Mosh,
    /// mosh on a new network path.
    MoshRoaming,
}

/// The multiplexer a connection runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mux {
    #[default]
    None,
    Tmux,
    Herdr,
}

impl Mux {
    /// What the bar calls it.
    pub fn label(self) -> &'static str {
        match self {
            Mux::None => "",
            Mux::Tmux => "tmux",
            Mux::Herdr => "herdr",
        }
    }
}

/// An agent's signal in the pane crumb (`AgentSignal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Signal {
    /// Waiting on a human: the whole crumb turns amber.
    Attention,
    Working,
    Done,
}

/// The breadcrumb's plan (`BreadcrumbPlan`), as the host decided it holds:
/// the live tree's once it is whole, else the last one kept while the
/// connection comes back.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Crumbs {
    pub session: String,
    /// The pane carries an agent: the session crumb may give up its text
    /// (keep only its icon) when three full segments do not fit the bar.
    #[serde(default)]
    pub squeezable: bool,
    pub window: String,
    /// No pane crumb without an active pane.
    #[serde(default)]
    pub pane: Option<String>,
    #[serde(default)]
    pub signal: Option<Signal>,
    /// What VoiceOver reads for the pane crumb (its title and what the dot
    /// means).
    #[serde(default)]
    pub pane_label: Option<String>,
    /// The multiplexer's icons (`MultiplexerVocabulary`).
    pub session_icon: String,
    pub window_icon: String,
    /// The live tree's: tappable. Otherwise the remembered one, dimmed and
    /// inert while the connection comes back.
    #[serde(default = "yes")]
    pub live: bool,
    /// What the plan was made from: every session, the active window and
    /// pane, the pane's stamp. With it the segments are named by Home's
    /// rules ([`Crumbs::names`]); without it, the strings above.
    #[serde(default)]
    pub tree: Option<crate::home::model::Control>,
}

fn yes() -> bool {
    true
}

/// The breadcrumb's three names (UI-DESIGN §6.3): `session › project ›
/// agent` — `train › trainer › claude`, not `0 › 1:2.1.322 › claude`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    pub session: String,
    /// The window's own name when someone gave it one, else the folder
    /// the pane works in, else the window's number.
    pub project: String,
    /// The agent the pane runs, else its program (no pane crumb without an
    /// active pane).
    pub pane: Option<String>,
    /// What VoiceOver reads for the pane crumb.
    pub pane_label: Option<String>,
}

impl Crumbs {
    /// The segments' names; `agent` is the word for an agent no stamp
    /// names. Each falls back to the host's string where the tree can't
    /// tell (no tree, a pane it doesn't hold).
    pub fn names(&self, agent: &str) -> Names {
        use crate::home::model::{folder_name, is_unnamed_window, pane_title};
        let mut names = Names {
            session: self.session.clone(),
            project: self.window.clone(),
            pane: self.pane.clone(),
            pane_label: self.pane_label.clone().or_else(|| self.pane.clone()),
        };
        let Some(tree) = &self.tree else {
            return names;
        };
        if let Some(session) = tree
            .active_session
            .as_deref()
            .and_then(|id| tree.sessions.iter().find(|s| s.id == id))
        {
            names.session = tree.session_display_name(session);
        }
        let Some(pane) = tree
            .active_pane
            .as_deref()
            .and_then(|id| tree.panes.iter().find(|p| p.id == id))
        else {
            return names;
        };
        if let Some(window) = tree.windows.iter().find(|w| w.id == pane.window) {
            let name = window.name.trim();
            names.project = if is_unnamed_window(name, &pane.command) {
                pane.path
                    .as_deref()
                    .and_then(folder_name)
                    .unwrap_or_else(|| window.index.to_string())
            } else {
                name.to_owned()
            };
        }
        let title = pane_title(tree, pane, agent);
        let who = title
            .agent
            .or_else(|| (!title.program.is_empty()).then_some(title.program));
        if let Some(who) = who {
            // The host's label is its title and what the dot means: the
            // name changes, the meaning stays.
            names.pane_label = match (&self.pane, &self.pane_label) {
                (Some(old), Some(label)) if label.starts_with(old.as_str()) => {
                    Some(format!("{who}{}", &label[old.len()..]))
                }
                _ => Some(who.clone()),
            };
            names.pane = Some(who);
        }
        names
    }
}

/// What covers the pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverKind {
    #[default]
    None,
    /// A first connect's poster (`TerminalConnectingView`).
    Poster,
    /// A reconnect's veil over the kept frame (`ReconnectVeil`).
    Veil,
    /// Another device holds the window's size (`SizeFollowVeil`).
    Follow,
}

/// The cover over the pane and what it says.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Cover {
    pub kind: CoverKind,
    /// The phase it announces (once the transport is back it keeps the
    /// phase it covered).
    pub state: LinkState,
    /// The server ended the session (the veil says so, calmly).
    pub ended: bool,
    /// Who holds the window's size ("iPad", "iPhone").
    pub owner: Option<String>,
    /// The poster takes the touches (nothing usable beneath it): not while
    /// it only waits for the first frame of a live session.
    pub touches: bool,
}

/// What the pane area holds, under the cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Content {
    /// No session yet: the terminal's background.
    #[default]
    None,
    /// The terminal (the host's view in the slot).
    Terminal,
    /// A reconnect with no frame to keep: the bare background.
    Blank,
    /// The multiplexer has no session to show: its empty state.
    Empty,
}

/// The multiplexer's empty state (`MultiplexerEmptyStateView`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Empty {
    pub mux: Mux,
    /// The binary is not on the host (else: no sessions).
    pub missing: bool,
    /// A reconnect: the wait before it decides shows nothing (the veil is
    /// up), not the poster.
    pub quiet: bool,
    /// The host has the answer already (the binary is known missing, the
    /// server said it has no session or no server): it shows at once. Else
    /// the empty state waits out the attach handshake's seconds first,
    /// in case the session is only on its way (SwiftUI's `startsSettled`).
    pub settled: bool,
    /// The vocabulary's words: "Session", "session", "sessions".
    pub session: String,
    pub session_lower: String,
    pub sessions_lower: String,
}

/// What the dependency a degraded session lacks is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Missing {
    Tmux,
    Herdr,
    MoshServer,
}

/// The one banner under the top bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Banner {
    /// mosh's UDP return path is dead (`MoshReturnPathBannerView`).
    ReturnPath,
    /// mosh is roaming to a new network path (`RoamBanner`).
    Roam {
        #[serde(default)]
        srtt: Option<f64>,
        #[serde(default)]
        history: Vec<f64>,
    },
    /// The session runs at less than asked (`HostBannerView`).
    Degrade { missing: Missing },
}

/// A transient notice from the multiplexer (a failed command, herdr's
/// words about its pane), with a remedy the person may take.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Notice {
    pub text: String,
    #[serde(default)]
    pub action: Option<String>,
}

/// `{"op": "chrome", …}`: everything but the shortcut bar and the panels.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Chrome {
    /// The connection's name (`displayName`), user, host and SSH port.
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u32,
    /// It runs over mosh.
    pub mosh: bool,
    pub mux: Mux,
    /// The leading button toggles the iPad's sidebar instead of going back.
    pub sidebar: bool,
    pub pill: PillKind,
    pub state: LinkState,
    pub crumbs: Option<Crumbs>,
    pub cover: Cover,
    pub content: Content,
    pub empty: Option<Empty>,
    pub banner: Option<Banner>,
    pub notice: Option<Notice>,
    /// The host of a URL just copied in the terminal: the "open link" chip.
    pub link: Option<String>,
    /// iPad landscape: the chrome trims (top bar 40, shortcut bar 38).
    pub compact: bool,
    /// A protocol switch (mosh ⇄ SSH) is under way, from its confirm until
    /// the new session's first frame is uncovered: nothing on the page
    /// moves. Its veil comes and goes at once (its capsule too), the
    /// breadcrumb dims and comes back at once, a banner leaves at once (the
    /// owner: 「mosh 切换 ssh 有个动画 我们直接去掉」). Ordinary drops and
    /// resumes keep their motion.
    pub still: bool,
}

/// A chip of the shortcut bar, from the shortcut store.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Chip {
    /// What the host knows it by.
    pub id: String,
    pub kind: ChipKind,
    /// The keycap's text (`chipLabel`).
    #[serde(default)]
    pub label: String,
}

/// What a chip is (`TerminalShortcut.Kind`, as the bar draws it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChipKind {
    /// A single key or a combination: sends on a tap, repeats while held.
    #[default]
    Key,
    /// Text or a sequence: sends on a tap.
    Text,
    /// Sticky ctrl.
    Ctrl,
    Paste,
    /// Browse the pane's working directory.
    Files,
    /// The photo picker; its long press is a menu.
    Image,
    Mic,
    /// The four-zone arrow cluster.
    Arrows,
    /// The joystick.
    Dpad,
    /// The scroll thumb.
    Scroll,
}

/// The image chip's menu.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct ImageMenu {
    /// The clipboard holds an image.
    pub clipboard: bool,
    /// The device has a camera.
    pub camera: bool,
    /// The images sent this session, oldest first.
    pub sent: Vec<SentImage>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SentImage {
    pub id: String,
    pub number: u32,
    pub filename: String,
}

/// `{"op": "bar", …}`: the shortcut bar.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Bar {
    pub chips: Vec<Chip>,
    /// The keyboard is down (the toggle offers to show it).
    pub keyboard_down: bool,
    pub ctrl_armed: bool,
    /// Voice input is on in Settings: the mic chip shows.
    pub voice: bool,
    /// A dictation session is up.
    pub mic_active: bool,
    pub image: ImageMenu,
}

/// The voice panel's phase (`VoiceDictationController.Phase`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase", deny_unknown_fields)]
pub enum DictationPhase {
    #[default]
    Starting,
    Downloading {
        progress: f64,
    },
    Loading,
    Listening,
    Finishing,
    Interrupted,
    /// It failed: what to tell the person, and whether Settings can fix it
    /// (a permission was denied).
    Failed {
        message: String,
        #[serde(default)]
        settings: bool,
    },
}

/// `{"op": "dictation", …}`: the voice panel (`DictationOverlayView`); a
/// `null` `panel` takes it away.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Dictation {
    pub phase: DictationPhase,
    /// The engine and language in use.
    pub engine: String,
    /// What is settled, and the engine's live guess after it.
    pub finalized: String,
    pub volatile: String,
    /// Insert and Send are offered.
    pub insertable: bool,
}

/// The image panel's phase (`ImageAttachmentController.Phase`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase", deny_unknown_fields)]
pub enum AttachmentPhase {
    #[default]
    Processing,
    Uploading {
        fraction: f64,
    },
    Ready,
    Failed {
        message: String,
    },
}

/// One image on its way.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Thumbnail {
    pub id: String,
    /// It landed on the host.
    pub landed: bool,
    /// The session number it can be inserted by again.
    pub number: Option<u32>,
}

/// `{"op": "attachment", …}`: the image panel
/// (`ImageAttachmentOverlayView`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Attachment {
    pub phase: AttachmentPhase,
    pub images: Vec<Thumbnail>,
    /// The images' size, in the system's words ("2.4 MB").
    pub size: Option<String>,
}

/// The preview panel's phase (`RemotePreviewModel.Phase`, the ones the
/// panel shows).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase", deny_unknown_fields)]
pub enum PreviewPhase {
    #[default]
    Opening,
    Downloading {
        fraction: f64,
        #[serde(default)]
        size: Option<String>,
    },
    /// A large file: download it?
    Confirm {
        size: String,
    },
    Failed {
        message: String,
    },
}

/// `{"op": "preview", …}`: a host file on its way to the preview
/// (`RemotePreviewHUD`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Preview {
    pub phase: PreviewPhase,
    pub name: String,
    /// The SF Symbol of its kind.
    pub symbol: String,
}

/// mosh's live counters (`MoshDiagnostics`): what came in, what was
/// applied, what went out and whether the server acknowledged it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Counters {
    pub datagrams: u64,
    pub applied: u64,
    pub parse_fails: u64,
    pub gaps: u64,
    pub sent: u64,
    pub user: u64,
    pub acked: u64,
    /// The UDP flow's state (`ready` when it is fine).
    pub channel: String,
    /// Input waits for the server's first answer.
    #[serde(default)]
    pub held: bool,
}

/// `{"op": "diagnostics", …}`: the counters' popover, hung from the pill
/// while the host shows it (`counters` null: nothing came in yet).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct Diagnostics {
    pub shown: bool,
    pub counters: Option<Counters>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chrome_reads_with_what_it_leaves_out_defaulted() {
        let chrome: Chrome = serde_json::from_str(
            r#"{"name":"studio","user":"cluas","host":"studio.local","port":22,"mux":"tmux",
                "pill":"mosh","state":"reconnecting",
                "crumbs":{"session":"moshpit","window":"2:agents","pane":"claude","signal":"attention",
                          "sessionIcon":"square.stack.3d.up","windowIcon":"macwindow"},
                "cover":{"kind":"veil","state":"reconnecting"},"content":"terminal",
                "banner":{"kind":"degrade","missing":"moshServer"}}"#,
        )
        .unwrap();
        assert_eq!(chrome.mux, Mux::Tmux);
        assert_eq!(chrome.pill, PillKind::Mosh);
        let crumbs = chrome.crumbs.unwrap();
        assert!(crumbs.live, "a plan is live unless the host says not");
        assert_eq!(crumbs.signal, Some(Signal::Attention));
        assert_eq!(chrome.cover.kind, CoverKind::Veil);
        assert_eq!(
            chrome.banner,
            Some(Banner::Degrade {
                missing: Missing::MoshServer
            })
        );
        assert!(serde_json::from_str::<Chrome>(r#"{"colour":1}"#).is_err());
    }

    fn tree(json: &str) -> crate::home::model::Control {
        serde_json::from_str(json).unwrap()
    }

    fn crumbs(tree: Option<crate::home::model::Control>) -> Crumbs {
        Crumbs {
            session: "0".into(),
            window: "1:2.1.322".into(),
            pane: Some("claude".into()),
            pane_label: Some("claude, needs you".into()),
            signal: Some(Signal::Attention),
            session_icon: "square.stack.3d.up".into(),
            window_icon: "macwindow".into(),
            live: true,
            tree,
            ..Crumbs::default()
        }
    }

    #[test]
    fn the_breadcrumb_says_session_project_agent() {
        // tmux named the window after Claude Code's version: the project is
        // the folder the pane works in.
        let auto = tree(
            r#"{"mux":"tmux","attached":true,"sessions":[{"id":"$0","name":"train"}],
                "windows":[{"id":"@1","session":"$0","index":1,"name":"2.1.322","paneCount":1}],
                "panes":[{"id":"%3","window":"@1","index":0,"command":"2.1.322","path":"/home/cluas/src/trainer"}],
                "hooks":[{"pane":"%3","state":"attention","agent":"claude"}],
                "activeSession":"$0","activeWindow":"@1","activePane":"%3"}"#,
        );
        let names = crumbs(Some(auto)).names("agent");
        assert_eq!(
            (
                names.session.as_str(),
                names.project.as_str(),
                names.pane.as_deref()
            ),
            ("train", "trainer", Some("claude"))
        );
        assert_eq!(names.pane_label.as_deref(), Some("claude, needs you"));
        // A window someone named keeps its name; no path, no name: its number.
        let named = tree(
            r#"{"mux":"tmux","attached":true,"sessions":[{"id":"$0","name":"0"}],
                "windows":[{"id":"@1","session":"$0","index":2,"name":"api","paneCount":1},
                           {"id":"@2","session":"$0","index":3,"name":"zsh","paneCount":1}],
                "panes":[{"id":"%3","window":"@1","index":0,"command":"zsh"}],
                "activeSession":"$0","activeWindow":"@1","activePane":"%3"}"#,
        );
        let names = crumbs(Some(named.clone())).names("agent");
        assert_eq!(
            (names.project.as_str(), names.pane.as_deref()),
            ("api", Some("zsh"))
        );
        let mut bare = named;
        bare.active_pane = Some("%4".into());
        bare.panes.push(
            serde_json::from_str(r#"{"id":"%4","window":"@2","index":0,"command":"zsh"}"#).unwrap(),
        );
        assert_eq!(crumbs(Some(bare)).names("agent").project, "3");
        // No tree: the host's strings, as before.
        let plain = crumbs(None).names("agent");
        assert_eq!(
            (plain.session.as_str(), plain.project.as_str()),
            ("0", "1:2.1.322")
        );
    }

    #[test]
    fn panels_read_their_phases() {
        let d: Dictation = serde_json::from_str(
            r#"{"phase":{"phase":"failed","message":"No mic","settings":true},"engine":"Apple · en-US"}"#,
        )
        .unwrap();
        assert_eq!(
            d.phase,
            DictationPhase::Failed {
                message: "No mic".into(),
                settings: true
            }
        );
        let a: Attachment =
            serde_json::from_str(r#"{"phase":{"phase":"uploading","fraction":0.5}}"#).unwrap();
        assert_eq!(a.phase, AttachmentPhase::Uploading { fraction: 0.5 });
        let p: Preview =
            serde_json::from_str(r#"{"phase":{"phase":"confirm","size":"80 MB"},"name":"a.mov"}"#)
                .unwrap();
        assert_eq!(
            p.phase,
            PreviewPhase::Confirm {
                size: "80 MB".into()
            }
        );
    }
}
