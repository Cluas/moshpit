//! Install Assist, its host side (`InstallAssistNSHPage.swift`'s
//! `InstallAssistPlan`): W11h's screen (`ui/src/terminal/install.rs`) for a
//! session whose host lacks tmux, mosh or herdr — the banner's Install and
//! the empty state's. It shows the command for the host's package manager
//! (from the capabilities the session probed and kept) and installs nothing
//! itself: Run types the command into the live shell, Copy copies it,
//! Re-check probes the host again over the session's SSH connection, and
//! once everything is there Reconnect dials a new session that uses it.

use moshpit_hostcare::{AssistPlan, Recheck};
use moshpit_install::{HostCapabilities, PackageManager};
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::connections::Multiplexer as SavedMux;
use moshpit_store::session_memory;
use moshpit_ui::terminal::install::Message;
use serde_json::Value;

use crate::app::{App, Core, PageKind};

/// What an Install Assist page keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPage {
    pub connection: Uuid,
    /// What the notice asked for: `tmux`, `mosh` or `herdr`.
    pub packages: Vec<String>,
    pub rechecking: bool,
    pub result: Option<Recheck>,
    pub copied: bool,
}

impl InstallPage {
    pub fn new(connection: Uuid, packages: Vec<String>) -> Self {
        InstallPage {
            connection,
            packages,
            rechecking: false,
            result: None,
            copied: false,
        }
    }
}

/// The plan for `page` (`moshpit-hostcare`'s, which iOS runs too): what
/// to install comes from the connection, the package manager from the
/// capabilities the session's last probe found.
fn plan(app: &App, page: &InstallPage) -> AssistPlan {
    let tmux = app
        .connections
        .connections()
        .iter()
        .find(|c| c.id == page.connection)
        .is_some_and(|c| c.multiplexer() == SavedMux::Tmux);
    AssistPlan::new(&page.packages, tmux, manager(app, page.connection))
}

/// The host's package manager, as the session's last probe found it.
fn manager(app: &App, connection: Uuid) -> Option<PackageManager> {
    let caps: Option<HostCapabilities> = session_memory::read(
        app.defaults.as_ref(),
        &session_memory::host_capabilities_key(&connection.to_string()),
    );
    caps.and_then(|c| c.package_manager)
}

/// `{"op": "install", …}` for `page`.
pub fn message(app: &App, page: &InstallPage) -> String {
    let json = moshpit_ui::host_care::install_message(
        &plan(app, page),
        page.rechecking,
        page.result,
        page.copied,
    );
    debug_assert!(Message::parse(&json).is_ok(), "{json}");
    json
}

impl App {
    /// Opens Install Assist for `packages` on `connection`'s session.
    pub(crate) fn open_install_assist(
        &mut self,
        core: &Core,
        connection: Uuid,
        packages: Vec<String>,
    ) {
        self.open(
            core,
            PageKind::InstallAssist(Box::new(InstallPage::new(connection, packages))),
            true,
        );
    }

    /// `connection`'s session goes and a new one dials (the same transport):
    /// what was installed is used from now on.
    pub(crate) fn redial(&mut self, core: &Core, connection: Uuid) {
        let weak = core.me.clone();
        self.sessions.disconnect(connection, move || {
            let Some(core) = weak.upgrade() else { return };
            let mut app = core.lock();
            app.forget_session(connection);
            app.connect(connection);
            let on_screen = matches!(
                app.pages.last().map(|p| &p.kind),
                Some(PageKind::Terminal(t)) if t.connection == connection
            );
            if on_screen {
                app.sessions.set_visible(Some(connection));
            }
            app.refresh_home();
            app.refresh_terminals(connection);
        });
    }

    fn recheck(&mut self, core: &Core, surface: u64) {
        let Some(PageKind::InstallAssist(page)) = self.page_mut(surface).map(|p| &mut p.kind)
        else {
            return;
        };
        if page.rechecking {
            return;
        }
        page.rechecking = true;
        page.result = None;
        let connection = page.connection;
        let plan = {
            let page = (**page).clone();
            plan(self, &page)
        };
        let borrowed = self
            .sessions
            .conn(connection)
            .map(|c| c.id)
            .and_then(|id| self.sessions.sessions().map(|s| s.borrow_ssh(id)));
        let weak = core.me.clone();
        moshpit_runtime::handle().spawn(async move {
            let caps = match borrowed {
                Some(borrowed) => match borrowed.await {
                    // The probe's `command -v` fails when a tool is missing:
                    // its output is what counts, not its status.
                    Ok(Ok(ssh)) => ssh
                        .run(&moshpit_install::capabilities::probe_command())
                        .await
                        .ok()
                        .map(|o| HostCapabilities::parse(&String::from_utf8_lossy(&o.stdout()))),
                    _ => None,
                },
                None => None,
            };
            let result = plan.recheck(caps.as_ref());
            let Some(core) = weak.upgrade() else { return };
            let mut app = core.lock();
            if let Some(caps) = caps {
                session_memory::write(
                    app.defaults.as_ref(),
                    &moshpit_store::Readiness::System,
                    &session_memory::host_capabilities_key(&connection.to_string()),
                    &caps,
                );
            }
            if let Some(PageKind::InstallAssist(page)) = app.page_mut(surface).map(|p| &mut p.kind)
            {
                page.rechecking = false;
                page.result = Some(result);
            }
            app.refresh_install_assist(surface);
        });
        self.refresh_install_assist(surface);
    }

    pub(crate) fn refresh_install_assist(&mut self, surface: u64) {
        let Some(PageKind::InstallAssist(page)) = self
            .pages
            .iter()
            .find(|p| p.surface == surface)
            .map(|p| &p.kind)
        else {
            return;
        };
        let json = message(self, page);
        self.post_changed(surface, "install", json);
    }
}

/// What Install Assist told the host.
pub fn on_event(app: &mut App, core: &Core, surface: u64, ev: &str, _event: &Value) {
    let Some(PageKind::InstallAssist(page)) = app.page_mut(surface).map(|p| &p.kind) else {
        return;
    };
    let page = (**page).clone();
    match ev {
        "done" | "back" => app.close(surface),
        "run" => {
            // Into the live shell with the newline that runs it: sudo's
            // prompt and the output stay in view.
            if let Some(command) = plan(app, &page).command()
                && let Some(stream) = app.sessions.stream(page.connection)
            {
                moshpit_stream::view::write(stream, format!("{command}\r").as_bytes());
            }
            app.close(surface);
        }
        "copy" => {
            if let Some(command) = plan(app, &page).command() {
                app.platform.copy_text(&command);
                if let Some(PageKind::InstallAssist(page)) =
                    app.page_mut(surface).map(|p| &mut p.kind)
                {
                    page.copied = true;
                }
                app.refresh_install_assist(surface);
            }
        }
        "recheck" => app.recheck(core, surface),
        "reconnect" => {
            app.close(surface);
            app.redial(core, page.connection);
        }
        _ => {}
    }
}
