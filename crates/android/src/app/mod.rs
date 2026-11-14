//! The app's controller: what the Swift host does on iOS, in Rust for
//! Android (`docs/nsh/DESIGN.md` §3.2) — the stores, the sessions, the page
//! stack, and each screen's host side (the data it posts, what it does
//! with what the screen tells it).
//!
//! On iOS each NSH screen has a Swift page that feeds it (`HomeNSHPage`,
//! `SettingsNSHPage`, …). Here the same job is one Rust controller over the
//! shared core, so the Java host stays thin (views, the Keystore, the
//! system's dialogs and intents) and everything it decides can run in
//! `cargo test` against [`crate::platform::testing::FakePlatform`].
//!
//! - [`Core`]: the controller behind one lock, the handle every entry point
//!   holds (the JNI, the screens' channel listeners, the session pump).
//! - Pages: a stack of NSH surfaces, one screen each. Pushing one opens its
//!   [`Channel`], parks what the surface will run ([`Pending`]) and asks the
//!   host for a view; the host's view takes it ([`Core::take_pending`]) and
//!   the controller posts the screen its data ([`App::surface_ready`]).
//! - A page's channel closes only when its view is gone
//!   ([`Core::surface_gone`], on the host's main thread): closing waits for
//!   a listener call in progress, and a listener holds the controller's
//!   lock, so nothing closes a channel under that lock.
//!
//! Every screen's own logic is in its module: [`home`], [`settings`],
//! [`keys`], [`shortcuts`], [`form`], [`looks`] (accent, theme), and the
//! terminal's in [`terminal`] (W11g's shared terminal screen).

pub mod debug;
pub mod form;
pub mod home;
pub mod host_setup;
pub mod install_assist;
pub mod journal;
pub mod keys;
pub mod looks;
pub mod picker;
pub mod sessions;
pub mod settings;
pub mod shortcuts;
pub mod terminal;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use moshpit_seal::foundation::uuid::Uuid;
use moshpit_store::app_themes::AppThemeStore;
use moshpit_store::connections::ConnectionStore;
use moshpit_store::settings::Settings;
use moshpit_store::shortcuts::ShortcutStore;
use moshpit_store::ssh_keys::SshKeyStore;
use moshpit_store::themes::ThemeStore;
use moshpit_store::{Defaults, Keychain, Readiness};
use moshpit_ui::channel::{Channel, SurfaceId};
use moshpit_ui::screen::{Params, Screen};
use serde_json::Value;

use crate::platform::{Ask, Platform, PlatformDefaults, PlatformKeychain};

/// What the host says about the app at start.
#[derive(Debug, Clone)]
pub struct Options {
    /// The version line under Home's last card and in Settings.
    pub version: String,
    /// What tmux's size leases call this device ("Pixel 9", "Android").
    pub device_label: String,
    /// The locale mosh-server starts the shell in (`en_US.UTF-8`).
    pub shell_locale: String,
    /// A debuggable build: the test hooks answer.
    pub debuggable: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            version: "v0.1.0 (1)".to_owned(),
            device_label: "Android".to_owned(),
            shell_locale: "en_US.UTF-8".to_owned(),
            debuggable: false,
        }
    }
}

/// What a page is, and what only it keeps.
pub enum PageKind {
    Home,
    Settings,
    /// A page Settings' hub opens: the Settings screen again, on that page.
    SettingsPart(moshpit_ui::settings::Part),
    SshKeys {
        deleting: Option<Uuid>,
        /// Why a key the person asked to delete was kept.
        notice: Option<moshpit_ui::ssh_keys::Notice>,
    },
    AddKey(Box<keys::AddKeyDraft>),
    /// Picks the key of the connection form below it.
    KeyPicker,
    Shortcuts,
    AddShortcut(Box<shortcuts::ShortcutDraft>),
    ConnectionForm(Box<form::FormDraft>),
    Accent {
        selected: String,
        deleting: Option<String>,
    },
    Theme {
        deleting: Option<String>,
    },
    /// A custom accent's editor (the shared model's).
    AccentEditor(Box<moshpit_app::looks::AccentEditor>),
    /// A custom theme's editor (the shared model's): its draft, and whether
    /// the bright rows show.
    ThemeEditor(Box<moshpit_app::looks::ThemeEditor>),
    /// Settings › Set up this host, on a live session's host.
    HostSetup(Box<host_setup::HostSetupPage>),
    /// The terminal's sessions, windows or panes (the breadcrumb's).
    MuxPicker(picker::PickerPage),
    /// The terminal's Install Assist, for what the host lacks.
    InstallAssist(Box<install_assist::InstallPage>),
    RecentLog,
    Terminal(Box<terminal::TerminalPage>),
}

impl PageKind {
    /// The shared screen it runs.
    pub fn screen(&self) -> Option<Screen> {
        Some(match self {
            PageKind::Home => Screen::Home,
            PageKind::Settings | PageKind::SettingsPart(_) => Screen::Settings,
            PageKind::SshKeys { .. } => Screen::SshKeys,
            PageKind::AddKey(_) => Screen::AddKey,
            PageKind::KeyPicker => Screen::KeyPicker,
            PageKind::Shortcuts => Screen::Shortcuts,
            PageKind::AddShortcut(_) => Screen::AddShortcut,
            PageKind::ConnectionForm(_) => Screen::ConnectionForm,
            PageKind::Accent { .. } => Screen::Accent,
            PageKind::Theme { .. } => Screen::Theme,
            PageKind::AccentEditor(_) => Screen::AccentEditor,
            PageKind::ThemeEditor(_) => Screen::ThemeEditor,
            PageKind::HostSetup(_) => Screen::HostSetup,
            PageKind::MuxPicker(_) => Screen::MuxPicker,
            PageKind::InstallAssist(_) => Screen::InstallAssist,
            PageKind::RecentLog => Screen::RecentLog,
            PageKind::Terminal(_) => Screen::Terminal,
        })
    }

    fn name(&self) -> &'static str {
        self.screen().map_or("none", Screen::name)
    }
}

/// One page of the stack.
pub struct Page {
    pub surface: SurfaceId,
    pub kind: PageKind,
    channel: Arc<Channel>,
}

/// What a surface runs, waiting for its view.
pub enum Pending {
    /// A shared screen on its channel.
    Screen {
        screen: Screen,
        params: Params,
        channel: Arc<Channel>,
    },
}

/// A file asked of the system's document picker, and where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Add Key's private key (`public`: its public key line), on page
    /// `surface`.
    KeyFile { surface: SurfaceId, public: bool },
}

/// What a picked file may weigh: a key or a theme is a few kilobytes.
pub const PICKED_FILE_LIMIT: usize = 1 << 20;

/// A question asked through the host's dialog, and what its answer does.
pub enum Question {
    DeleteConnection(Uuid),
    NewSession(Uuid),
    RenameSession(Uuid, String),
    KillSession(Uuid, String),
    RenameWindow(Uuid, String),
    KillWindow(Uuid, String),
    KillPane(Uuid, String),
    /// A host key the session asks about (the prompt's number).
    TrustHostKey(Uuid, u64),
    /// Theme JSON pasted into the import field.
    ImportThemes,
    /// A screen's question with a text field (`{"ev": "prompt"}`, its id):
    /// the answer goes back to page `surface` as `{"op": "prompted"}`.
    ScreenPrompt {
        surface: SurfaceId,
        id: u64,
    },
}

/// The controller.
pub struct App {
    pub(crate) platform: Arc<dyn Platform>,
    pub(crate) options: Options,
    pub(crate) defaults: Arc<dyn Defaults>,
    pub(crate) keychain: Arc<dyn Keychain>,
    pub(crate) connections: ConnectionStore,
    pub(crate) keys: SshKeyStore,
    pub(crate) shortcuts: ShortcutStore,
    pub(crate) settings: Settings,
    pub(crate) themes: ThemeStore,
    pub(crate) accents: AppThemeStore,
    pub(crate) sessions: sessions::Live,
    pub(crate) pages: Vec<Page>,
    pending: HashMap<SurfaceId, Pending>,
    /// Home's trees' disclosure, as the page last told it.
    pub(crate) trees: Vec<moshpit_ui::home::TreeState>,
    questions: HashMap<u64, Question>,
    picks: HashMap<u64, Pick>,
    next_token: u64,
    /// What each page was last posted, by surface and op: a session event
    /// that changes nothing a page shows posts nothing.
    posted: HashMap<(SurfaceId, &'static str), String>,
    /// The person's own words, for a share that arrived (the share queue).
    pub(crate) shares: Vec<String>,
    /// Host care (`moshpit-hostcare`): the Host Setup pages' machine,
    /// started with the first page.
    pub(crate) care: Option<moshpit_hostcare::Driver>,
}

/// The controller behind its lock: what every entry point holds.
pub struct Core {
    app: Mutex<App>,
    pub(crate) me: Weak<Core>,
}

impl Core {
    /// The controller on `platform`, with nothing on screen yet.
    pub fn new(platform: Arc<dyn Platform>, options: Options) -> Arc<Core> {
        let defaults: Arc<dyn Defaults> = Arc::new(PlatformDefaults(platform.clone()));
        let keychain: Arc<dyn Keychain> = Arc::new(PlatformKeychain(platform.clone()));
        let readiness = Readiness::System;
        let mut connections = ConnectionStore::new(defaults.clone(), readiness.clone());
        connections.load();
        let mut keys = SshKeyStore::new(defaults.clone(), readiness.clone());
        keys.load();
        let mut shortcuts = ShortcutStore::new(defaults.clone(), readiness.clone());
        shortcuts.load();
        let mut settings = Settings::new(defaults.clone(), None, readiness.clone());
        settings.set_debug_build(options.debuggable);
        let mut themes = ThemeStore::new(defaults.clone(), readiness.clone());
        themes.load();
        let accents = AppThemeStore::new(defaults.clone(), readiness);
        Arc::new_cyclic(|me: &Weak<Core>| Core {
            me: me.clone(),
            app: Mutex::new(App {
                platform,
                options,
                defaults,
                keychain,
                connections,
                keys,
                shortcuts,
                settings,
                themes,
                accents,
                sessions: sessions::Live::default(),
                pages: Vec::new(),
                pending: HashMap::new(),
                trees: Vec::new(),
                questions: HashMap::new(),
                picks: HashMap::new(),
                next_token: 1,
                posted: HashMap::new(),
                shares: Vec::new(),
                care: None,
            }),
        })
    }

    /// The controller, locked.
    pub fn lock(&self) -> MutexGuard<'_, App> {
        self.app.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts the sessions (on the core's runtime) and shows Home.
    pub fn start(self: &Arc<Self>) {
        let events = {
            let mut app = self.lock();
            let (platform, defaults, keychain, options) = (
                app.platform.clone(),
                app.defaults.clone(),
                app.keychain.clone(),
                app.options.clone(),
            );
            let events = app
                .sessions
                .start(&platform, &defaults, &keychain, &options);
            app.open(self, PageKind::Home, false);
            events
        };
        if let Some(mut events) = events {
            let core = Arc::downgrade(self);
            moshpit_runtime::handle().spawn(async move {
                while let Some(event) = events.recv().await {
                    let Some(core) = core.upgrade() else { return };
                    core.lock().on_session_event(&core, event);
                }
            });
        }
    }

    /// The activity was made anew: its views are new, so every page goes
    /// and Home is pushed again (the sessions stay).
    pub fn restart(self: &Arc<Self>) {
        let channels: Vec<Arc<Channel>> = {
            let mut app = self.lock();
            app.pending.clear();
            app.posted.clear();
            app.pages.drain(..).map(|p| p.channel).collect()
        };
        for channel in channels {
            channel.close();
        }
        self.lock().open(self, PageKind::Home, false);
    }

    /// What surface `surface` runs, once (its view takes it).
    pub fn take_pending(&self, surface: SurfaceId) -> Option<Pending> {
        self.lock().pending.remove(&surface)
    }

    /// Surface `surface`'s view runs its screen: post it its data.
    pub fn surface_ready(self: &Arc<Self>, surface: SurfaceId) {
        self.lock().surface_ready(self, surface);
    }

    /// Surface `surface`'s view is gone (destroyed by the host): its
    /// channel closes. Never under the controller's lock (the module doc).
    pub fn surface_gone(&self, surface: SurfaceId) {
        let channel = {
            let mut app = self.lock();
            app.pending.remove(&surface);
            app.posted.retain(|(s, _), _| *s != surface);
            let index = app.pages.iter().position(|p| p.surface == surface);
            index.map(|i| app.pages.remove(i).channel)
        };
        if let Some(channel) = channel.or_else(|| Channel::find(surface)) {
            channel.close();
        }
    }

    /// The system's back on the top page: `true` when the app took it.
    pub fn back(self: &Arc<Self>) -> bool {
        self.lock().back(self)
    }

    /// A dialog's answer: `ok`, and the text of a prompt.
    pub fn answer(self: &Arc<Self>, token: u64, ok: bool, text: Option<String>) {
        self.lock().answer(self, token, ok, text);
    }

    /// The file the document picker asked for under `token` (`None`: none
    /// picked, or it could not be read).
    pub fn picked(self: &Arc<Self>, token: u64, bytes: Option<Vec<u8>>) {
        self.lock().picked(token, bytes);
    }

    /// What screen `surface` told the host.
    pub fn on_event(self: &Arc<Self>, surface: SurfaceId, json: &str) {
        let value: Value = match serde_json::from_str(json) {
            Ok(value) => value,
            Err(error) => {
                log::warn!(target: "moshpit", "surface {surface}: not JSON: {error}");
                return;
            }
        };
        self.lock().on_event(self, surface, &value);
    }
}

impl App {
    /// Pushes a page running `kind`; its surface id.
    pub(crate) fn open(&mut self, core: &Core, kind: PageKind, animated: bool) -> SurfaceId {
        let channel = Channel::open();
        let surface = channel.id();
        let weak = core.me.clone();
        channel.set_listener(Some(Box::new(move |json| {
            if let Some(core) = weak.upgrade() {
                core.on_event(surface, json);
            }
        })));
        let mut params = self.params();
        if let PageKind::SettingsPart(part) = &kind {
            params.page = Some(part.id().to_owned());
        }
        let screen = kind.screen().expect("every page runs a shared screen");
        let pending = Pending::Screen {
            screen,
            params,
            channel: channel.clone(),
        };
        log::info!(target: "moshpit", "page {surface}: {}", kind.name());
        // A page over the terminal (a picker, Settings, Set up this host)
        // takes the screen: the terminal's keyboard goes down under it.
        if matches!(
            self.pages.last().map(|p| &p.kind),
            Some(PageKind::Terminal(_))
        ) && !matches!(kind, PageKind::Terminal(_))
        {
            self.platform.keyboard(false);
        }
        if let PageKind::ConnectionForm(draft) = &kind {
            // The platform's secret fields start from this form's draft,
            // not the last form's.
            self.platform
                .fill_key_field("connection-password", &draft.password);
            self.platform.fill_key_field("connection-pem", &draft.pem);
        }
        self.pending.insert(surface, pending);
        self.pages.push(Page {
            surface,
            kind,
            channel,
        });
        self.platform.push(surface, animated);
        surface
    }

    /// Takes page `surface` off the stack.
    pub(crate) fn close(&mut self, surface: SurfaceId) {
        if let Some(index) = self.pages.iter().position(|p| p.surface == surface) {
            // The record goes now (what the page still says is not heard);
            // its channel when the host destroyed its view.
            let page = self.pages.remove(index);
            log::info!(target: "moshpit", "page {surface} closed: {}", page.kind.name());
            if matches!(page.kind, PageKind::Terminal(_)) {
                // The terminal's keyboard does not stay up over Home.
                self.platform.keyboard(false);
            } else if let Some(Page {
                kind: PageKind::Terminal(terminal),
                ..
            }) = self.pages.last()
                && !terminal.keyboard_down
            {
                // Back on the terminal: its keyboard as the person left it.
                self.platform.keyboard(true);
            }
            self.posted.retain(|(s, _), _| *s != surface);
            self.platform.pop(surface, true);
        }
    }

    /// The top page, if it is one of `kind`'s.
    pub(crate) fn page_mut(&mut self, surface: SurfaceId) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.surface == surface)
    }

    /// The pages showing `screen`.
    fn surfaces_of(&self, screen: Screen) -> Vec<SurfaceId> {
        self.pages
            .iter()
            .filter(|p| p.kind.screen() == Some(screen))
            .map(|p| p.surface)
            .collect()
    }

    /// Posts `json` to page `surface` unless it is what `op` last posted
    /// there.
    pub(crate) fn post_changed(&mut self, surface: SurfaceId, op: &'static str, json: String) {
        if self.posted.get(&(surface, op)) == Some(&json) {
            return;
        }
        if self.post(surface, json.clone()) {
            self.posted.insert((surface, op), json);
        }
    }

    /// Posts `json` to page `surface`, if it is there and its view runs
    /// (until then, the view's first post carries what it shows); whether
    /// it was.
    pub(crate) fn post(&self, surface: SurfaceId, json: String) -> bool {
        if self.pending.contains_key(&surface) {
            return false;
        }
        let Some(page) = self.pages.iter().find(|p| p.surface == surface) else {
            return false;
        };
        match page.channel.post(&json) {
            Ok(()) => true,
            Err(error) => {
                log::warn!(target: "moshpit", "post to {}: {error}", page.kind.name());
                false
            }
        }
    }

    /// What every screen runs with: the app's theme, and its appearance,
    /// the one chosen in Settings › Appearance (R18; the system's unless
    /// light or dark was chosen). A page takes them when it opens, as it
    /// takes the accent. (The locale is the system's, which the host gives
    /// NSH.)
    pub(crate) fn params(&self) -> Params {
        Params {
            theme: looks::app_theme(&self.accents.theme(&self.settings.app_theme_id())),
            appearance: Some(settings::appearance_choice(self.settings.appearance())),
            ..Params::default()
        }
    }

    /// Posts each page what it shows now (after anything that may have
    /// changed what several pages show).
    pub(crate) fn refresh(&mut self) {
        for surface in self.pages.iter().map(|p| p.surface).collect::<Vec<_>>() {
            self.refresh_page(surface);
        }
    }

    fn refresh_page(&mut self, surface: SurfaceId) {
        let Some(page) = self.pages.iter().find(|p| p.surface == surface) else {
            return;
        };
        let json = match &page.kind {
            PageKind::Home => Some(home::message(self)),
            PageKind::Settings | PageKind::SettingsPart(_) => Some(settings::message(self)),
            PageKind::SshKeys { deleting, notice } => {
                Some(keys::keys_message(self, *deleting, notice.clone()))
            }
            PageKind::AddKey(draft) => Some(keys::add_key_message(self, draft)),
            PageKind::KeyPicker => Some(keys::picker_message(self)),
            PageKind::Shortcuts => Some(shortcuts::message(self)),
            PageKind::AddShortcut(_) => None,
            PageKind::ConnectionForm(_) => None,
            PageKind::Accent { selected, deleting } => {
                Some(looks::accents_message(self, selected, deleting.as_deref()))
            }
            PageKind::Theme { deleting } => Some(looks::themes_message(self, deleting.as_deref())),
            // An editor keeps its own copy: posted once, as its view starts.
            PageKind::AccentEditor(_) | PageKind::ThemeEditor(_) => None,
            PageKind::HostSetup(page) => {
                // Which host first: the page names it.
                if let Some(host) = host_setup::identity(self, page) {
                    self.post(surface, host);
                }
                host_setup::message(self, page)
            }
            PageKind::MuxPicker(page) => Some(picker::message(self, page)),
            PageKind::InstallAssist(page) => Some(install_assist::message(self, page)),
            PageKind::RecentLog => None,
            PageKind::Terminal(page) => {
                let page = (**page).clone();
                self.refresh_terminal(surface, &page);
                None
            }
        };
        if let Some(json) = json {
            self.post(surface, json);
        }
    }

    /// The pages showing Home, refreshed.
    pub(crate) fn refresh_home(&mut self) {
        for surface in self.surfaces_of(Screen::Home) {
            self.refresh_page(surface);
        }
    }

    /// The terminal pages of `connection`, refreshed.
    pub(crate) fn refresh_terminals(&mut self, connection: Uuid) {
        let surfaces: Vec<_> = self
            .pages
            .iter()
            .filter(|p| matches!(&p.kind, PageKind::Terminal(t) if t.connection == connection))
            .map(|p| p.surface)
            .collect();
        for surface in surfaces {
            self.refresh_page(surface);
        }
    }

    /// Terminal page `surface`'s chrome and bar, posted when they changed,
    /// and its slot's stream told to the host when it changed.
    fn refresh_terminal(&mut self, surface: SurfaceId, page: &terminal::TerminalPage) {
        let chrome = terminal::json("chrome", &terminal::chrome(self, page));
        let bar = terminal::json("bar", &terminal::bar(self, page));
        self.post_changed(surface, "chrome", chrome);
        self.post_changed(surface, "bar", bar);
        let theme = self.themes.theme(&self.settings.theme_id());
        let colors = looks::terminal_colors(&theme);
        let size = self.settings.font_size();
        let look = format!("{colors:?} {size}");
        if self.posted.get(&(0, "look")) != Some(&look) {
            self.platform.terminal_look(&colors, size);
            self.posted.insert((0, "look"), look);
        }
        // Text shared while no terminal was live goes to the first one that is.
        if !self.shares.is_empty()
            && self
                .sessions
                .conn(page.connection)
                .is_some_and(|c| c.state.is_live())
        {
            for text in std::mem::take(&mut self.shares) {
                let _ = self.sessions.deliver(page.connection, &text);
            }
            self.platform.toast("Shared text sent to the terminal");
        }
        let stream = self.sessions.stream(page.connection).unwrap_or(0);
        if stream != page.shown_stream {
            if let Some(Page {
                kind: PageKind::Terminal(page),
                ..
            }) = self.page_mut(surface)
            {
                page.shown_stream = stream;
            }
            self.platform.show_stream(surface, stream);
        }
    }

    fn surface_ready(&mut self, core: &Core, surface: SurfaceId) {
        let Some(page) = self.pages.iter().find(|p| p.surface == surface) else {
            return;
        };
        let first = match &page.kind {
            PageKind::Home => {
                let trees = moshpit_ui::home::Message::Trees(self.trees.clone()).to_json();
                Some(vec![trees, home::message(self)])
            }
            PageKind::AddShortcut(draft) => Some(vec![shortcuts::editor_message(self, draft)]),
            PageKind::ConnectionForm(draft) => Some(vec![form::message(self, draft)]),
            PageKind::RecentLog => Some(vec![journal::message()]),
            PageKind::AccentEditor(editor) => Some(vec![editor.message()]),
            PageKind::ThemeEditor(editor) => Some(vec![editor.message()]),
            _ => None,
        };
        match first {
            Some(messages) => {
                for json in messages {
                    self.post(surface, json);
                }
            }
            None => self.refresh_page(surface),
        }
        let _ = core;
    }

    fn back(&mut self, core: &Core) -> bool {
        let Some(top) = self.pages.last() else {
            return false;
        };
        if self.pages.len() == 1 {
            // Home: leave the app (the sessions stay up while it is away,
            // as on iOS, until the system takes the process).
            self.platform.finish();
            return true;
        }
        let surface = top.surface;
        // The system's back leaves a page as its own back button does: the
        // accent picked applies as the Accent page goes (it was dropped).
        if matches!(top.kind, PageKind::Accent { .. }) {
            looks::on_event(self, core, surface, "back", &Value::Null);
        } else {
            self.close(surface);
        }
        if matches!(self.pages.last().map(|p| &p.kind), Some(PageKind::Home)) {
            self.sessions.set_visible(None);
        }
        true
    }

    fn on_event(&mut self, core: &Core, surface: SurfaceId, event: &Value) {
        let Some(page) = self.pages.iter().find(|p| p.surface == surface) else {
            // A page already closed: what it still says is not heard.
            return;
        };
        let ev = event["ev"].as_str().unwrap_or_default().to_owned();
        let kind = page.kind.name();
        log::info!(target: "moshpit", "{kind}: {ev}");
        match kind {
            "home" => home::on_event(self, core, surface, &ev, event),
            "settings" => settings::on_event(self, core, surface, &ev, event),
            "ssh-keys" => keys::on_keys_event(self, core, surface, &ev, event),
            "add-key" => keys::on_add_key_event(self, core, surface, &ev, event),
            "key-picker" => keys::on_picker_event(self, core, surface, &ev, event),
            "shortcuts" => shortcuts::on_event(self, core, surface, &ev, event),
            "add-shortcut" => shortcuts::on_editor_event(self, core, surface, &ev, event),
            "connection-form" => form::on_event(self, core, surface, &ev, event),
            "accent" | "theme" => looks::on_event(self, core, surface, &ev, event),
            "accent-editor" | "theme-editor" => {
                looks::on_editor_event(self, core, surface, &ev, event)
            }
            "host-setup" => host_setup::on_event(self, core, surface, &ev, event),
            "mux-picker" => picker::on_event(self, core, surface, &ev, event),
            "install-assist" => install_assist::on_event(self, core, surface, &ev, event),
            "terminal" => terminal::on_event(self, core, surface, &ev, event),
            "recent-log" if ev == "back" => self.close(surface),
            _ => {}
        }
    }

    /// Asks the person `question` with a system dialog.
    pub(crate) fn ask(&mut self, question: Question, build: impl FnOnce(u64) -> Ask) {
        let token = self.next_token;
        self.next_token += 1;
        self.questions.insert(token, question);
        self.platform.ask(&build(token));
    }

    fn answer(&mut self, core: &Core, token: u64, ok: bool, text: Option<String>) {
        let Some(question) = self.questions.remove(&token) else {
            return;
        };
        if !ok {
            match question {
                Question::TrustHostKey(connection, prompt) => {
                    self.answer_host_key(connection, prompt, false)
                }
                Question::ScreenPrompt { surface, id } => picker::prompted(self, surface, id, None),
                _ => {}
            }
            return;
        }
        match question {
            Question::ImportThemes => looks::import_themes(self, text.as_deref().unwrap_or("")),
            Question::ScreenPrompt { surface, id } => {
                picker::prompted(self, surface, id, Some(text.unwrap_or_default()))
            }
            question => home::answer(self, core, question, text),
        }
    }

    /// Asks the system's document picker for a file, for `pick`.
    pub(crate) fn pick_file(&mut self, pick: Pick) {
        let token = self.next_token;
        self.next_token += 1;
        self.picks.insert(token, pick);
        self.platform.pick_file(token);
    }

    fn picked(&mut self, token: u64, bytes: Option<Vec<u8>>) {
        let Some(pick) = self.picks.remove(&token) else {
            return;
        };
        let Some(bytes) = bytes else {
            // Cancelled: nothing changes.
            return;
        };
        if bytes.len() > PICKED_FILE_LIMIT {
            self.platform.toast("That file is too large");
            return;
        }
        match pick {
            Pick::KeyFile { surface, public } => self.key_file_picked(surface, public, bytes),
        }
    }

    /// A share from another app: queued, and typed into the terminal on
    /// screen when there is one (iOS: the share queue the extension fills).
    pub fn share(&mut self, text: String) {
        let live = self.pages.iter().rev().find_map(|p| match &p.kind {
            PageKind::Terminal(t) => Some(t.connection),
            _ => None,
        });
        match live.and_then(|c| self.sessions.deliver(c, &text)) {
            Some(()) => self.platform.toast("Sent to the terminal"),
            None => {
                self.shares.push(text);
                self.platform
                    .toast("Shared text is waiting: open a terminal to send it");
            }
        }
    }

    /// The pages of the stack, by name, bottom first (the test hooks).
    pub fn stack(&self) -> Vec<(SurfaceId, &'static str)> {
        self.pages
            .iter()
            .map(|p| (p.surface, p.kind.name()))
            .collect()
    }

    /// The platform (the JNI host's entry points use it).
    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.platform
    }
}

/// A text field of a JSON event.
pub(crate) fn text(event: &Value, key: &str) -> Option<String> {
    event[key].as_str().map(str::to_owned)
}

/// A UUID field of a JSON event.
pub(crate) fn uuid(event: &Value, key: &str) -> Option<Uuid> {
    event[key].as_str().and_then(Uuid::parse)
}

#[cfg(test)]
mod tests;
