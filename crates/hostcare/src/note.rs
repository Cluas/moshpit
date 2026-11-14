//! What the screens say, typed: each is a text of the app's string catalog
//! with its arguments, which the screen puts in its own language
//! (`ui/src/host_setup.rs`). Text a host or the platform said is carried as
//! it came ([`Note::Said`]).

use moshpit_install::hooks;
use moshpit_install::installer::{Blocked, ChannelError, StepFailure};

/// Why a phone-side pairing step failed (`PushRelayClient.Failure`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayFailure {
    /// The relay's address isn't a URL the phone posts to.
    BadUrl,
    /// The pairings file wouldn't take the pairing.
    CouldNotSave,
    /// The pairings file wouldn't let go of it.
    CouldNotForget,
    /// No device token in time.
    NoDeviceToken,
    /// The relay answered, with something other than 200.
    Status { code: u16, body: String },
    /// The request didn't reach it (the platform's words).
    Network(String),
    /// A 200 whose body isn't a mint.
    Malformed,
    /// This build has no push service (Android without FCM).
    Unavailable,
}

/// A text a screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// As a host or the platform said it.
    Said(String),
    /// "Looking at the host…"
    LookingAtHost,
    /// "Installing hooks for %@…" (English in the app: its step label).
    InstallingHooks { agent: String },
    /// "Removing hooks…" (English in the app).
    RemovingHooks,
    /// "Unpairing…" (English in the app).
    Unpairing,
    /// "Removing Moshpit from this host…" (English in the app, as the
    /// other step labels; B375).
    RemovingEverything,
    /// "Minting a send token at the relay…"
    MintingSendToken,
    /// "Installing on %@…"
    InstallingOn { host: String },
    /// "Enter the address of your push relay first."
    EnterRelay,
    /// "This host is missing %@ — install those and try again."
    MissingTools(Vec<String>),
    /// A step's failure.
    Step(StepFailure),
    /// Codex's trust step.
    TrustCodexHooks,
    /// "Open a tmux pane on this host to test the hooks."
    OpenTmuxPane,
    /// "Install the hooks for %@ to test them." — the test runs what an
    /// install registered (B380).
    HooksNotInstalled { agent: String },
    /// "Update the hooks for %@ first: …" — the test runs what this build
    /// registers (B380).
    HooksOutOfDate { agent: String },
    /// "%@ doesn't register Moshpit for %@. …": the agent's config (as a
    /// person reads its path) lacks Moshpit's command for an event (B380).
    NotRegistered { config: String, event: String },
    /// "Moshpit found neither %@ nor a %@ command here. …": asked before
    /// installing hooks for an agent the host doesn't have (B381).
    AgentNotFound {
        agent: String,
        dir: String,
        command: String,
    },
    /// "This host's tmux isn't on the default socket, …"
    TmuxElsewhere,
    /// "The stamp script ran but the pane came back empty.%@", with the
    /// agent's trust step after it when it has one.
    StampEmpty { trust_hint: bool },
    /// "The pane came back as \"%@\" instead of the test stamp."
    StampOther(String),
    /// "The host reported: %@"
    HostReported(String),
    /// "The host sent one, but this screen never saw it arrive. …"
    PushNeverSeen,
    /// "The host isn't answering on this session's connection. Try again
    /// in a moment." — a command given up past its deadline.
    HostNotAnswering,
    /// A pairing step on the phone.
    Relay(RelayFailure),
}

impl Note {
    /// A channel's failure, in the platform's words — or the machine's,
    /// for a command the host never answered.
    pub fn channel(error: &ChannelError) -> Self {
        if error.0 == crate::exec::HOST_SILENT {
            return Note::HostNotAnswering;
        }
        Note::Said(error.0.clone())
    }

    /// Why nothing was attempted (`HostSetupModel.blockedMessage`).
    pub fn blocked(blocked: &Blocked) -> Self {
        match blocked {
            Blocked::MissingTools(tools) => Note::MissingTools(tools.clone()),
        }
    }

    /// The English of this text, as the app's catalog keys it: for logs,
    /// tests, and a host with no catalog.
    pub fn english(&self) -> String {
        match self {
            Note::Said(text) => text.clone(),
            Note::LookingAtHost => "Looking at the host\u{2026}".into(),
            Note::InstallingHooks { agent } => format!("Installing hooks for {agent}\u{2026}"),
            Note::RemovingHooks => "Removing hooks\u{2026}".into(),
            Note::Unpairing => "Unpairing\u{2026}".into(),
            Note::RemovingEverything => "Removing Moshpit from this host\u{2026}".into(),
            Note::MintingSendToken => "Minting a send token at the relay\u{2026}".into(),
            Note::InstallingOn { host } => format!("Installing on {host}\u{2026}"),
            Note::EnterRelay => "Enter the address of your push relay first.".into(),
            Note::MissingTools(tools) => format!(
                "This host is missing {} \u{2014} install those and try again.",
                tools.join(", ")
            ),
            Note::Step(failure) => failure.to_string(),
            Note::TrustCodexHooks => TRUST_CODEX.into(),
            Note::OpenTmuxPane => "Open a tmux pane on this host to test the hooks.".into(),
            Note::HooksNotInstalled { agent } => format!("Install the hooks for {agent} to test them."),
            Note::HooksOutOfDate { agent } => format!(
                "Update the hooks for {agent} first: the test runs what this version of Moshpit installs."
            ),
            Note::NotRegistered { config, event } => format!(
                "{config} doesn't register Moshpit for {event}. Reinstall to put it back."
            ),
            Note::AgentNotFound { dir, command, .. } => format!(
                "Moshpit found neither {dir} nor a {command} command here. Its hooks will run once it's installed."
            ),
            Note::TmuxElsewhere => "This host's tmux isn't on the default socket, so Moshpit can't run the test. The hooks themselves are unaffected.".into(),
            Note::StampEmpty { trust_hint } => format!(
                "The stamp script ran but the pane came back empty.{}",
                if *trust_hint { format!(" {TRUST_CODEX}") } else { String::new() }
            ),
            Note::StampOther(got) => {
                format!("The pane came back as \"{got}\" instead of the test stamp.")
            }
            Note::HostReported(output) => format!("The host reported: {output}"),
            Note::PushNeverSeen => PUSH_NEVER_SEEN.into(),
            Note::HostNotAnswering => {
                "The host isn't answering on this session's connection. Try again in a moment.".into()
            }
            Note::Relay(failure) => match failure {
                RelayFailure::BadUrl => "That relay address isn't a valid URL.".into(),
                RelayFailure::CouldNotSave => "Couldn't save the pairing on this phone, so nothing was installed on the host. The diagnostics log has the reason.".into(),
                RelayFailure::CouldNotForget => "Couldn't remove the pairing from this phone, so the host was left paired. The diagnostics log has the reason.".into(),
                RelayFailure::NoDeviceToken => "iOS hasn't issued a push token to this phone yet \u{2014} check that notifications are allowed, then try again.".into(),
                RelayFailure::Status { code, body } => {
                    format!("Relay refused to mint a send token ({code}): {body}")
                }
                RelayFailure::Network(said) => said.clone(),
                RelayFailure::Malformed => hooks::PARSE_ERROR.into(),
                RelayFailure::Unavailable => PUSH_UNAVAILABLE.into(),
            },
        }
    }
}

/// Codex's trust step (`HookAgent.trustStep`).
pub const TRUST_CODEX: &str = "Codex won't run a newly added hook until you trust it. Run /hooks inside Codex on this host and trust the four Moshpit entries \u{2014} until you do, nothing will be sent.";

/// `HostSetupModel.pushTimeoutReason` with no sender output.
pub const PUSH_NEVER_SEEN: &str = "The host sent one, but this screen never saw it arrive. Most likely Moshpit was not in the foreground \u{2014} the test can only be confirmed while you are looking at this screen, so try again without leaving it. If it still fails, check that notifications are allowed and that the relay can reach Apple.";

/// A build with no push service (Android's `NO_PUSH`).
pub const PUSH_UNAVAILABLE: &str = "Push notifications on Android need Firebase Cloud Messaging, which this build doesn't include yet.";
