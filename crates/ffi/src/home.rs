//! Home's record for the iOS host (`moshpit_app::home`): the
//! `{"op": "home"}` message, built here from the registry's view of each
//! saved connection's session and the agents monitor's clock. The host says
//! only what is its own — which connections are saved and with which
//! session, which one it is disconnecting, the version line, whether Home
//! is the iPad's sidebar, the app's language — and posts what comes back
//! to the screen as it is.
//!
//! A session on the Swift session path isn't in the registry: its card
//! reads as saved. iOS shows the NSH Home only with sessions on the core.

use std::sync::Arc;

use moshpit_app::home::{self, HomeInput, SavedConnection};

use crate::agents::AgentMachine;
use crate::session::CoreSessions;

/// One saved connection, as the host's store has it, and its session.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HomeConnection {
    /// The store's id (an upper-case UUID string).
    pub id: String,
    /// The name the store displays it by.
    pub name: String,
    pub user: String,
    pub host: String,
    /// Its SSH port: with the host, where its trusted host key is found.
    pub port: u16,
    /// The emoji picked for it, which its mark shows in place of its face.
    pub emoji: Option<String>,
    pub mosh: bool,
    /// The first port of its mosh UDP range.
    pub udp_port: u32,
    /// Its multiplexer is tmux.
    pub tmux: bool,
    /// The host is disconnecting it.
    pub disconnecting: bool,
    /// Its session in this registry (`connect`'s id), when it has one.
    pub session: Option<u64>,
}

#[uniffi::export]
impl CoreSessions {
    /// Home's message now: `connections` in the store's order, each with
    /// its session's view; `agents`, the monitor whose clock dates the
    /// panes' stamps (none: no times but the stamps' own); `version`, the
    /// footer's line; `sidebar`, Home is the iPad's sidebar; `locale`, the
    /// language the app shows.
    pub fn home_message(
        &self,
        connections: Vec<HomeConnection>,
        agents: Option<Arc<AgentMachine>>,
        version: String,
        sidebar: bool,
        locale: String,
    ) -> String {
        let saved: Vec<SavedConnection> = connections
            .into_iter()
            .map(|c| SavedConnection {
                session: c.session.and_then(|id| self.registry().view(id)),
                fingerprint: self.trusted(&c.host, c.port),
                emoji: c.emoji,
                id: c.id,
                name: c.name,
                user: c.user,
                host: c.host,
                mosh: c.mosh,
                udp_port: c.udp_port,
                tmux: c.tmux,
                disconnecting: c.disconnecting,
            })
            .collect();
        home::message(&HomeInput {
            connections: &saved,
            since: &|conn| {
                agents
                    .as_ref()
                    .map(|agents| agents.since(conn))
                    .unwrap_or_default()
            },
            version: &version,
            sidebar,
            locale: &locale,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use moshpit_ui::home::Message;
    use moshpit_ui::home::model::{FailureReason, Link};

    use super::*;
    use crate::agents::AgentSettingsValue;
    use crate::session::{
        Credential, CredentialFailure, CredentialHost, PlatformSignature, SessionEvent,
        SessionEventSink, SessionMultiplexer, SessionSpec, SessionStateValue,
    };
    use crate::store::{KeychainFailure, KeychainHost, PlistValue, PreferencesHost};

    struct Nothing;

    #[async_trait::async_trait]
    impl CredentialHost for Nothing {
        // A password, so the connect reaches the port: the credential is
        // read before the dial.
        async fn credential(&self, _: String) -> Result<Credential, CredentialFailure> {
            Ok(Credential::Password {
                password: "unused".into(),
            })
        }
        async fn sign(
            &self,
            _: String,
            _: Vec<u8>,
        ) -> Result<PlatformSignature, CredentialFailure> {
            Err(CredentialFailure::Canceled)
        }
    }

    impl KeychainHost for Nothing {
        fn save(&self, _: String, _: Vec<u8>, _: bool) -> Result<(), KeychainFailure> {
            Err(KeychainFailure::NotFound)
        }
        fn load(&self, _: String, _: String) -> Result<Vec<u8>, KeychainFailure> {
            Err(KeychainFailure::NotFound)
        }
        fn delete(&self, _: String) -> Result<(), KeychainFailure> {
            Err(KeychainFailure::NotFound)
        }
        fn exists(&self, _: String) -> bool {
            false
        }
        fn accounts(&self) -> Result<Vec<String>, KeychainFailure> {
            Ok(Vec::new())
        }
        fn load_silently(&self, _: String) -> Result<Vec<u8>, KeychainFailure> {
            Err(KeychainFailure::NotFound)
        }
    }

    impl PreferencesHost for Nothing {
        fn get(&self, _: String) -> Option<PlistValue> {
            None
        }
        fn set(&self, _: String, _: Option<PlistValue>) {}
        fn protected_data_available(&self) -> bool {
            true
        }
    }

    impl SessionEventSink for Nothing {
        fn deliver(&self, _: Vec<SessionEvent>) {}
    }

    fn connection(id: &str, session: Option<u64>) -> HomeConnection {
        HomeConnection {
            id: id.into(),
            name: "studio".into(),
            user: "cluas".into(),
            host: "127.0.0.1".into(),
            port: 22,
            emoji: None,
            mosh: false,
            udp_port: 60001,
            tmux: true,
            disconnecting: false,
            session,
        }
    }

    fn cards(json: &str) -> Vec<moshpit_ui::home::model::Connection> {
        match Message::parse(json).expect("the screen takes it") {
            Message::Home(home) => home.connections,
            _ => panic!("not Home's message: {json}"),
        }
    }

    /// The registry's own view of a real session, through the entry point
    /// the host calls: a connect refused on a closed local port reads as a
    /// dead card that says why; an id the registry doesn't know (a session
    /// on the Swift path, one already gone) as a saved one.
    #[test]
    fn home_reads_each_cards_session_from_the_registry() {
        let nothing = Arc::new(Nothing);
        let sessions = CoreSessions::new(
            nothing.clone(),
            nothing.clone(),
            nothing.clone(),
            nothing,
            "test".into(),
        );
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let id = sessions.connect(SessionSpec {
            connection_id: "C0FFEE00-0000-4000-8000-000000000001".into(),
            user: "cluas".into(),
            host: "127.0.0.1".into(),
            port: closed,
            proxy_host: None,
            proxy_port: None,
            mosh: false,
            multiplexer: SessionMultiplexer::Tmux,
            tmux_path: None,
            mosh_server_path: None,
            mosh_port_low: None,
            mosh_port_high: None,
            herdr_path: None,
            locale: "en_US.UTF-8".into(),
            cols: 80,
            rows: 24,
            connected_before: false,
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        while !matches!(
            sessions.view(id).map(|v| v.state),
            Some(SessionStateValue::Failed { .. })
        ) {
            assert!(
                Instant::now() < deadline,
                "the refused connect never failed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let agents = AgentMachine::new(
            AgentSettingsValue {
                notifications: false,
                live_activity: false,
                show_detail: false,
                sound: false,
                language: "en".into(),
            },
            None,
            None,
            HashMap::new(),
            30.0,
            1_762_800_000.0,
        );
        let json = sessions.home_message(
            vec![
                connection("C0FFEE00-0000-4000-8000-000000000001", Some(id)),
                connection("C0FFEE00-0000-4000-8000-000000000002", Some(id + 1_000)),
                connection("C0FFEE00-0000-4000-8000-000000000003", None),
            ],
            Some(agents),
            "v1.0.9 (1)".into(),
            true,
            "en".into(),
        );
        let cards = cards(&json);
        assert_eq!(cards[0].link, Link::Dead);
        let failure = cards[0].failure.as_ref().expect("a failed card says why");
        assert_eq!(failure.reason, FailureReason::Unreachable, "{failure:?}");
        assert!(cards[0].lost_at.is_none(), "it never went live");
        assert!(cards[0].control.is_none());
        assert_eq!(
            cards[1].link,
            Link::Saved,
            "an id the registry doesn't know"
        );
        assert_eq!(cards[2].link, Link::Saved);
        sessions.disconnect(id);
    }
}
