//! Settings › Set up this host, its host side: the core's host care
//! (`moshpit-hostcare`, the machine iOS runs too) driven over the live
//! session's own SSH connection (`Sessions::borrow_ssh`: no second
//! sign-in), what the page shows posted whenever the machine says it
//! changed, and the person's asks handed to the machine.
//!
//! This build has no push service (FCM is not linked, `android/PUSH.md`):
//! the machine runs with `PushSupport::Unavailable`, so the push section
//! says why and pairs nothing. Care on connect isn't run here: with nothing
//! to pair, its one job would be the consent question, which Android
//! doesn't ask yet.

use std::sync::{Arc, Weak};

use moshpit_hostcare::driver::{Driver, Platform};
use moshpit_hostcare::{Change, Clock, Config, Conn, HostCare, HttpReply, PushSupport, Settings};
use moshpit_install::HostChannel;
use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::uuid::Uuid;
use serde_json::Value;

use crate::app::{App, Core, PageKind};

/// The language the core's texts are said in here: Android's host gives
/// the system locale to NSH, not to the core, so these stay English (as
/// this page always was).
const LOCALE: &str = "en";

/// What the Set up this host page keeps: the machine's page.
#[derive(Debug, Clone)]
pub struct HostSetupPage {
    /// The connection whose live session the installer rides.
    pub connection: Uuid,
    /// The machine's id for the page.
    pub page: u64,
}

/// `{"op": "host", …}`: which connection `page` sets up, by its name and
/// where it reaches (the page names it, however it was opened).
pub fn identity(app: &App, page: &HostSetupPage) -> Option<String> {
    let connection = app
        .connections
        .connections()
        .iter()
        .find(|c| c.id == page.connection)?;
    Some(
        moshpit_ui::host_setup::Message::Host(moshpit_ui::host_setup::Subject {
            name: connection.display_name(),
            user: connection.username.clone(),
            host: connection.host.clone(),
            port: connection.port,
        })
        .to_json(),
    )
}

/// `{"op": "host-setup", …}` for `page`.
pub fn message(app: &App, page: &HostSetupPage) -> Option<String> {
    let care = app.care.as_ref()?;
    let view = care.read(|m| m.setup_view(page.page))?;
    Some(moshpit_ui::host_care::host_setup_message(&view, LOCALE))
}

/// The installer's channel: one exec on the session's SSH connection,
/// failing unless the command finished with status 0 (what the app's
/// `executeCommand` does since B136).
#[derive(Clone)]
pub struct SshHost(pub moshpit_ssh::Ssh);

impl HostChannel for SshHost {
    async fn run(
        &self,
        command: &str,
    ) -> Result<Vec<u8>, moshpit_install::installer::ChannelError> {
        use moshpit_install::installer::ChannelError;
        let output = self
            .0
            .run(command)
            .await
            .map_err(|e| ChannelError(e.to_string()))?;
        match (output.closed, output.status()) {
            (true, Some(0)) => Ok(output.stdout()),
            (true, Some(code)) => {
                let stderr = String::from_utf8_lossy(&output.stderr()).trim().to_owned();
                Err(ChannelError(if stderr.is_empty() {
                    format!("the command exited with status {code}")
                } else {
                    stderr
                }))
            }
            _ => Err(ChannelError(
                "the connection dropped mid-command".to_owned(),
            )),
        }
    }
}

/// What the machine asks of Android.
struct AndroidCare {
    core: Weak<Core>,
}

#[async_trait::async_trait]
impl Platform for AndroidCare {
    async fn exec(&self, connection: Uuid, command: String) -> Result<Vec<u8>, String> {
        let not_connected = || "The session isn't connected.".to_owned();
        let borrowed = {
            let core = self.core.upgrade().ok_or_else(not_connected)?;
            let app = core.lock();
            app.sessions
                .conn(connection)
                .map(|c| c.id)
                .and_then(|id| app.sessions.sessions().map(|s| s.borrow_ssh(id)))
        };
        let ssh = borrowed
            .ok_or_else(not_connected)?
            .await
            .map_err(|_| not_connected())?
            .map_err(|failure| failure.log)?;
        SshHost(ssh).run(&command).await.map_err(|e| e.0)
    }

    async fn post(&self, _url: String, _body: String, _timeout: u32) -> Result<HttpReply, String> {
        Err(moshpit_hostcare::note::PUSH_UNAVAILABLE.to_owned())
    }

    fn register_for_push(&self) {}

    fn remember_declined(&self, _connection: Uuid) {}

    fn remember_unpaired(&self, _connection: Uuid, _unpaired: bool) {}

    fn changed(&self, change: Change) {
        let Change::Page(page) = change else {
            return;
        };
        // Called with the app locked: post from the runtime instead.
        let core = self.core.clone();
        moshpit_runtime::handle().spawn(async move {
            if let Some(core) = core.upgrade() {
                core.lock().refresh_host_setup_page(page);
            }
        });
    }
}

impl App {
    /// The machine, started the first time a page needs it.
    pub(crate) fn care(&mut self, core: &Core) -> Driver {
        if let Some(care) = &self.care {
            return care.clone();
        }
        let clock = Clock {
            ms: 0,
            date: Date::now(),
        };
        let machine = HostCare::new(
            Config {
                push: PushSupport::Unavailable,
                autocare_off: true,
                budgets: Default::default(),
            },
            Box::new(moshpit_hostcare::MemoryStore::default()),
            clock,
        );
        let platform: Arc<dyn Platform> = Arc::new(AndroidCare {
            core: core.me.clone(),
        });
        let driver = Driver::new(machine, platform, moshpit_runtime::handle());
        let relay = self.settings.push_relay_url();
        driver.with(|m, k| {
            m.set_settings(
                Settings {
                    relay_url: relay,
                    ..Settings::default()
                },
                k,
            )
        });
        self.care = Some(driver.clone());
        driver
    }

    /// Opens Set up this host on the session of the connection on screen (the
    /// terminal on top, else any live one); with none live, says why.
    pub(crate) fn open_host_setup(&mut self, core: &Core) {
        let on_screen = self.pages.iter().rev().find_map(|p| match &p.kind {
            PageKind::Terminal(t) => Some(t.connection),
            _ => None,
        });
        let live = on_screen
            .filter(|c| self.sessions.conn(*c).is_some_and(|c| c.state.is_live()))
            .or_else(|| {
                self.sessions
                    .by_connection
                    .iter()
                    .find(|(_, c)| c.state.is_live())
                    .map(|(id, _)| *id)
            });
        let Some(connection) = live else {
            self.platform
                .toast("Connect to a host first: setup inspects the one you're connected to");
            return;
        };
        let label = self
            .connections
            .connections()
            .iter()
            .find(|c| c.id == connection)
            .map(|c| c.display_name())
            .unwrap_or_default();
        // The pane the stamp is fired into: the session's tmux pane.
        let pane = self
            .sessions
            .conn(connection)
            .and_then(|c| c.tmux.as_ref())
            .and_then(|t| t.snapshot.active_pane_id.clone());
        let care = self.care(core);
        let page = care.with(|m, k| {
            m.open_setup(
                Conn {
                    id: connection,
                    label,
                },
                pane,
                k,
            )
        });
        self.open(
            core,
            PageKind::HostSetup(Box::new(HostSetupPage { connection, page })),
            true,
        );
    }

    /// The page showing the machine's page `page`, posted what it shows.
    pub(crate) fn refresh_host_setup_page(&mut self, page: u64) {
        let surfaces: Vec<u64> = self
            .pages
            .iter()
            .filter(|p| matches!(&p.kind, PageKind::HostSetup(h) if h.page == page))
            .map(|p| p.surface)
            .collect();
        for surface in surfaces {
            self.refresh_host_setup(surface);
        }
    }

    /// Page `surface` shows what its model says now.
    pub(crate) fn refresh_host_setup(&mut self, surface: u64) {
        let Some(PageKind::HostSetup(page)) = self
            .pages
            .iter()
            .find(|p| p.surface == surface)
            .map(|p| &p.kind)
        else {
            return;
        };
        let (host, setup) = (identity(self, page), message(self, page));
        if let Some(json) = host {
            self.post_changed(surface, "host", json);
        }
        if let Some(json) = setup {
            self.post_changed(surface, "host-setup", json);
        }
    }
}

/// What the Set up this host page told the host.
pub fn on_event(app: &mut App, _core: &Core, surface: u64, ev: &str, event: &Value) {
    let Some(PageKind::HostSetup(page)) = app.page_mut(surface).map(|p| &p.kind) else {
        return;
    };
    let page = page.page;
    let Some(care) = app.care.clone() else {
        return;
    };
    match ev {
        "done" | "back" => {
            app.close(surface);
            care.with(|m, k| m.close_setup(page, k));
        }
        "agent" => {
            if let Some(id) = event["id"].as_str() {
                care.with(|m, k| m.setup_select(page, id, k));
            }
        }
        "inspect" => care.with(|m, k| m.setup_inspect(page, k)),
        "install-hooks" => care.with(|m, k| m.setup_install_hooks(page, k)),
        "install-hooks-anyway" => care.with(|m, k| m.setup_install_hooks_anyway(page, k)),
        "dismiss" => care.with(|m, k| m.setup_dismiss(page, k)),
        "remove-hooks" => care.with(|m, k| m.setup_remove_hooks(page, k)),
        "prove-hooks" => care.with(|m, k| m.setup_prove_hooks(page, k)),
        "pair" => care.with(|m, k| m.setup_pair(page, k)),
        "unpair" => care.with(|m, k| m.setup_unpair(page, k)),
        "remove-everything" => care.with(|m, k| m.setup_remove_everything(page, k)),
        "prove-push" => care.with(|m, k| m.setup_prove_push(page, k)),
        _ => {}
    }
    app.refresh_host_setup(surface);
}
