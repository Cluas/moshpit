//! The equivalence harness's Rust side: the case format both front ends run
//! (`harness/swift/main.swift` over the app's unchanged model sources,
//! `examples/store_harness.rs` over this crate), the 1.0.8 view of what this
//! crate stores (`harness/swift108/main.swift` prints the same for 1.0.8's
//! sources), the seeded case generator and the canonical form results are
//! compared in. `harness/equivalence.py` drives them.
//!
//! A case is one line: `STORE PHONE SNAPSHOT OP…`.
//!
//! - `STORE`: `conn`, `keys`, `short`, `theme`, `apptheme`, `settings`,
//!   `hosts` (a store over preferences or the keychain, built first), or
//!   `ready`, `bytes`, `hex`, `export` (functions).
//! - `PHONE`: `U` (protected data available) or `L` (a launch before the
//!   first unlock); `unlock` / `lock` ops flip it.
//! - `SNAPSHOT`: `~`, or the hex of a JSON object
//!   `{"d": [[key, VALUE]…], "k": [[account, hex]…]}`: the preferences and
//!   keychain items before the store is built. A VALUE is `["b", true]`,
//!   `["i", "12"]`, `["f", BITS]`, `["s", HEX]`, `["x", HEX]` (data),
//!   `["t", BITS]` (a date), `["a", [VALUE…]]`, `["o", [[HEXKEY, VALUE]…]]`.
//! - `OP`: `name` or `name:ARG:ARG…`, each ARG `~` (empty), `-` (none), a
//!   decimal, or the hex of a string (JSON text for models).
//!
//! The result line holds one segment per step, ` | `-separated: `init`, then
//! each op as `name=RESULT`, each followed by the store's state and the
//! preferences (`snap{…}`) / keychain (`kc{…}`). Strings are hex (`~`
//! empty, `-` none), dates and doubles the hex of their bits. Ids a step
//! minted (a seeded built-in's UUID, a theme's `custom-…`) are printed `#N`
//! in order of appearance, since both sides mint at random.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use moshpit_seal::foundation::date::Date;
use moshpit_seal::foundation::json::{self, Dialect, Value};
use moshpit_seal::foundation::uuid::Uuid;
use moshpit_seal::foundation::{double, hex};

use crate::app_themes::{AppTheme, AppThemeStore, catalog};
use crate::connections::{ConnectionStore, ServerConnection};
use crate::host::{Defaults, Keychain, KeychainError, MemoryDefaults};
use crate::known_hosts::{self, KeychainKnownHosts, KnownHostsStore, StoreError, Table};
use crate::plist::Plist;
use crate::readiness::Readiness;
use crate::settings::{self, Kind, SettingValue, Settings};
use crate::shortcuts::{ShortcutStore, TerminalShortcut};
use crate::ssh_keys::{SshKeyAlgorithm, SshKeyRecord, SshKeyStore};
use crate::themes::{self, AnsiSlot, TerminalTheme, ThemeStore};

// MARK: - Tokens

/// A string argument: `~` is empty, otherwise the hex of its UTF-8.
pub fn arg(token: &str) -> String {
    if token == "~" {
        return String::new();
    }
    String::from_utf8(hex::decode(token).expect("a hex token")).expect("UTF-8")
}

/// The hex of a string, `~` when empty.
pub fn out(text: &str) -> String {
    if text.is_empty() {
        "~".to_owned()
    } else {
        hex::encode(text.as_bytes())
    }
}

fn bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn opt_flag(value: Option<bool>) -> &'static str {
    value.map_or("-", flag)
}

// MARK: - Minted ids

/// Ids the case did not bring, printed `#1`, `#2`, … in order of appearance.
struct Minted {
    known: HashSet<String>,
    seen: Vec<String>,
}

impl Minted {
    fn new(inputs: &[String]) -> Self {
        let mut known = HashSet::new();
        for input in inputs {
            for id in ids_in(input) {
                known.insert(id.to_uppercase());
            }
        }
        Self {
            known,
            seen: Vec::new(),
        }
    }

    fn mask(&mut self, text: &str) -> String {
        let mut out = text.to_owned();
        for id in ids_in(text) {
            if self.known.contains(&id.to_uppercase()) {
                continue;
            }
            let index = match self.seen.iter().position(|s| *s == id) {
                Some(index) => index,
                None => {
                    self.seen.push(id.clone());
                    self.seen.len() - 1
                }
            };
            out = out.replace(&id, &format!("#{}", index + 1));
        }
        out
    }

    /// A string as a result token.
    fn out(&mut self, text: &str) -> String {
        out(&self.mask(text))
    }

    fn opt(&mut self, text: Option<&str>) -> String {
        text.map_or_else(|| "-".to_owned(), |t| self.out(t))
    }
}

/// Every id-shaped run in `text`: UUIDs (8-4-4-4-12 hex digits, either
/// case) and the minted `custom-xxxxxxxx` / `app-xxxxxxxx` theme ids.
fn ids_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if let Some(len) = uuid_at(bytes, at).or_else(|| minted_theme_at(bytes, at)) {
            found.push(text[at..at + len].to_owned());
            at += len;
        } else {
            at += 1;
        }
    }
    found
}

fn uuid_at(bytes: &[u8], at: usize) -> Option<usize> {
    let slice = bytes.get(at..at + 36)?;
    let ok = slice.iter().enumerate().all(|(i, &b)| {
        if matches!(i, 8 | 13 | 18 | 23) {
            b == b'-'
        } else {
            b.is_ascii_hexdigit()
        }
    });
    ok.then_some(36)
}

fn minted_theme_at(bytes: &[u8], at: usize) -> Option<usize> {
    for prefix in [&b"custom-"[..], &b"app-"[..]] {
        if bytes[at..].starts_with(prefix) {
            let digits = bytes.get(at + prefix.len()..at + prefix.len() + 8)?;
            if digits
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            {
                return Some(prefix.len() + 8);
            }
        }
    }
    None
}

// MARK: - Snapshots

/// The value a snapshot names: `["b", true]`, `["i", "12"]`, ….
pub fn plist_from_json(value: &Value) -> Plist {
    let Some([tag, payload]) = value
        .as_array()
        .and_then(|a| <&[Value; 2]>::try_from(a).ok())
    else {
        panic!("a snapshot value is a pair")
    };
    let text = || payload.as_str().expect("a string payload");
    match tag.as_str().expect("a tag") {
        "b" => Plist::Bool(matches!(payload, Value::Bool(true))),
        "i" => Plist::Int(text().parse().expect("an integer")),
        "f" => Plist::Double(f64::from_bits(
            u64::from_str_radix(text(), 16).expect("bits"),
        )),
        "t" => Plist::Date(f64::from_bits(
            u64::from_str_radix(text(), 16).expect("bits"),
        )),
        "s" => Plist::String(arg(text())),
        "x" => Plist::Data(if text() == "~" {
            Vec::new()
        } else {
            hex::decode(text()).expect("hex")
        }),
        "a" => Plist::Array(
            payload
                .as_array()
                .expect("items")
                .iter()
                .map(plist_from_json)
                .collect(),
        ),
        "o" => Plist::Dictionary(
            payload
                .as_array()
                .expect("members")
                .iter()
                .map(|member| {
                    let pair = member.as_array().expect("a member");
                    (
                        arg(pair[0].as_str().expect("a key")),
                        plist_from_json(&pair[1]),
                    )
                })
                .collect(),
        ),
        other => panic!("no snapshot value {other:?}"),
    }
}

/// The JSON a snapshot names `value` with (the inverse of [`plist_from_json`]).
pub fn plist_to_json(value: &Plist) -> String {
    match value {
        Plist::Bool(flag) => format!("[\"b\",{flag}]"),
        Plist::Int(number) => format!("[\"i\",\"{number}\"]"),
        Plist::Double(number) => format!("[\"f\",\"{}\"]", bits(*number)),
        Plist::Date(seconds) => format!("[\"t\",\"{}\"]", bits(*seconds)),
        Plist::String(text) => format!("[\"s\",\"{}\"]", out(text)),
        Plist::Data(bytes) => format!(
            "[\"x\",\"{}\"]",
            if bytes.is_empty() {
                "~".to_owned()
            } else {
                hex::encode(bytes)
            }
        ),
        Plist::Array(items) => format!(
            "[\"a\",[{}]]",
            items
                .iter()
                .map(plist_to_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Plist::Dictionary(members) => format!(
            "[\"o\",[{}]]",
            members
                .iter()
                .map(|(k, v)| format!("[\"{}\",{}]", out(k), plist_to_json(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// A snapshot token for these preferences and keychain items.
pub fn snapshot_token(
    defaults: &BTreeMap<String, Plist>,
    keychain: &BTreeMap<String, Vec<u8>>,
) -> String {
    if defaults.is_empty() && keychain.is_empty() {
        return "~".to_owned();
    }
    let d: Vec<String> = defaults
        .iter()
        .map(|(k, v)| format!("[\"{}\",{}]", json_escape(k), plist_to_json(v)))
        .collect();
    let k: Vec<String> = keychain
        .iter()
        .map(|(a, b)| {
            format!(
                "[\"{}\",\"{}\"]",
                json_escape(a),
                if b.is_empty() {
                    "~".to_owned()
                } else {
                    hex::encode(b)
                }
            )
        })
        .collect();
    hex::encode(format!("{{\"d\":[{}],\"k\":[{}]}}", d.join(","), k.join(",")).as_bytes())
}

fn json_escape(text: &str) -> String {
    json::write_encoder(&Value::String(text.to_owned()), json::Style::COMPACT)
        .trim_matches('"')
        .to_owned()
}

struct Snapshot {
    defaults: BTreeMap<String, Plist>,
    keychain: BTreeMap<String, Vec<u8>>,
}

fn parse_snapshot(token: &str) -> Snapshot {
    let mut snapshot = Snapshot {
        defaults: BTreeMap::new(),
        keychain: BTreeMap::new(),
    };
    if token == "~" {
        return snapshot;
    }
    let text = arg(token);
    let value = json::parse(text.as_bytes(), Dialect::Decoder).expect("snapshot JSON");
    let object = value.as_object().expect("a snapshot object");
    for pair in object.get("d").and_then(Value::as_array).unwrap_or(&[]) {
        let pair = pair.as_array().expect("a pair");
        snapshot.defaults.insert(
            pair[0].as_str().expect("a key").to_owned(),
            plist_from_json(&pair[1]),
        );
    }
    for pair in object.get("k").and_then(Value::as_array).unwrap_or(&[]) {
        let pair = pair.as_array().expect("a pair");
        let bytes = pair[1].as_str().expect("hex");
        snapshot.keychain.insert(
            pair[0].as_str().expect("an account").to_owned(),
            if bytes == "~" {
                Vec::new()
            } else {
                hex::decode(bytes).expect("hex")
            },
        );
    }
    snapshot
}

/// Every string a snapshot holds, for the ids it brings.
fn snapshot_texts(snapshot: &Snapshot) -> Vec<String> {
    fn walk(value: &Plist, into: &mut Vec<String>) {
        match value {
            Plist::String(text) => into.push(text.clone()),
            Plist::Data(bytes) => into.push(String::from_utf8_lossy(bytes).into_owned()),
            Plist::Array(items) => items.iter().for_each(|i| walk(i, into)),
            Plist::Dictionary(members) => members.iter().for_each(|(k, v)| {
                into.push(k.clone());
                walk(v, into);
            }),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for value in snapshot.defaults.values() {
        walk(value, &mut texts);
    }
    for bytes in snapshot.keychain.values() {
        texts.push(String::from_utf8_lossy(bytes).into_owned());
    }
    texts
}

/// A plist value in a snapshot. A number prints by its value (`n…`; a
/// double with a fraction by its bits): `UserDefaults` may keep an equal
/// value of another kind (a stored 0 over a set `false`), depending on its
/// caching, so the kind of equal numbers is not something to compare.
fn plist_out(value: &Plist, minted: &mut Minted) -> String {
    match value {
        Plist::Bool(flag) => format!("n{}", self::flag(*flag)),
        Plist::Int(number) => format!("n{number}"),
        Plist::Double(number)
            if number.fract() == 0.0 && number.abs() < 9_223_372_036_854_775_808.0 =>
        {
            format!("n{}", *number as i64)
        }
        Plist::Double(number) => format!("f{}", bits(*number)),
        Plist::Date(seconds) => format!("t{}", bits(*seconds)),
        Plist::String(text) => format!("s{}", minted.out(text)),
        Plist::Data(bytes) => format!("x{}", data_out(bytes, minted)),
        Plist::Array(items) => format!(
            "a[{}]",
            items
                .iter()
                .map(|i| plist_out(i, minted))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Plist::Dictionary(members) => {
            let mut members: Vec<&(String, Plist)> = members.iter().collect();
            members.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            format!(
                "o{{{}}}",
                members
                    .iter()
                    .map(|(k, v)| format!("{}={}", out(k), plist_out(v, minted)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

/// Data as hex, minted ids masked where it is UTF-8.
fn data_out(bytes: &[u8], minted: &mut Minted) -> String {
    if bytes.is_empty() {
        return "~".to_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => hex::encode(minted.mask(text).as_bytes()),
        Err(_) => hex::encode(bytes),
    }
}

fn snap_out(defaults: &MemoryDefaults, minted: &mut Minted) -> String {
    let snapshot = defaults.snapshot();
    let mut keys: Vec<&String> = snapshot.keys().collect();
    keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let members: Vec<String> = keys
        .iter()
        .map(|k| format!("{}={}", out(k), plist_out(&snapshot[*k], minted)))
        .collect();
    format!("snap{{{}}}", members.join(","))
}

// MARK: - The keychain

/// A keychain in memory that can be locked.
#[derive(Default)]
struct TestKeychain {
    items: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
    locked: AtomicBool,
}

impl Keychain for TestKeychain {
    fn save(&self, account: &str, secret: &[u8], _: bool) -> Result<(), KeychainError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(KeychainError::InteractionNotAllowed);
        }
        crate::host::lock(&self.items).insert(account.to_owned(), secret.to_vec());
        Ok(())
    }
    fn load(&self, account: &str, _: &str) -> Result<Vec<u8>, KeychainError> {
        if self.locked.load(Ordering::SeqCst) {
            return Err(KeychainError::InteractionNotAllowed);
        }
        crate::host::lock(&self.items)
            .get(account)
            .cloned()
            .ok_or(KeychainError::NotFound)
    }
    fn delete(&self, account: &str) -> Result<(), KeychainError> {
        crate::host::lock(&self.items).remove(account);
        Ok(())
    }
    fn exists(&self, account: &str) -> bool {
        crate::host::lock(&self.items).contains_key(account)
    }
}

fn kc_out(keychain: &TestKeychain, minted: &mut Minted) -> String {
    let items = crate::host::lock(&keychain.items).clone();
    format!(
        "kc{{{}}}",
        items
            .iter()
            .map(|(a, b)| format!("{}=x{}", out(a), data_out(b, minted)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

// MARK: - Dumps

fn date_out(date: Date) -> String {
    bits(date.since_reference())
}

fn conn_out(c: &ServerConnection, m: &mut Minted) -> String {
    let id = m.mask(&c.id.to_string());
    let ssh_key = c
        .ssh_key_id
        .map_or_else(|| "-".to_owned(), |id| m.mask(&id.to_string()));
    [
        id,
        m.out(&c.name),
        m.out(&c.host),
        c.port.to_string(),
        m.out(&c.username),
        out(match c.auth_method {
            crate::connections::AuthMethod::Password => "password",
            crate::connections::AuthMethod::Key => "key",
        }),
        out(match c.connection_protocol {
            crate::connections::ConnectionProtocol::Ssh => "ssh",
            crate::connections::ConnectionProtocol::Mosh => "mosh",
        }),
        c.ssh_port.to_string(),
        c.mosh_port_range_start.to_string(),
        c.mosh_port_range_end.to_string(),
        m.opt(c.mosh_server_path.as_deref()),
        m.opt(c.keychain_ref.as_deref()),
        date_out(c.created_at),
        c.last_connected_at.map_or_else(|| "-".to_owned(), date_out),
        flag(c.use_tmux).to_owned(),
        m.opt(c.predict_mode_raw.as_deref()),
        opt_flag(c.roam_on_cellular).to_owned(),
        m.opt(c.tmux_path.as_deref()),
        m.opt(c.herdr_path.as_deref()),
        m.opt(c.multiplexer_raw.as_deref()),
        ssh_key,
        m.opt(c.ssh_key_algorithm_raw.as_deref()),
        opt_flag(c.use_socks_proxy).to_owned(),
        m.opt(c.socks_proxy_host.as_deref()),
        c.socks_proxy_port
            .map_or_else(|| "-".to_owned(), |p| p.to_string()),
        out(c.multiplexer().raw_value()),
        out(c.predict_mode().raw_value()),
        m.out(&c.display_name()),
    ]
    .join("/")
}

fn conn_state(store: &ConnectionStore, m: &mut Minted) -> String {
    let conns: Vec<String> = store.connections().iter().map(|c| conn_out(c, m)).collect();
    let saved = match store.saved_ids() {
        None => "-".to_owned(),
        Some(ids) if ids.is_empty() => "~".to_owned(),
        Some(ids) => {
            let mut ids: Vec<String> = ids.iter().map(|id| m.mask(&id.to_string())).collect();
            ids.sort();
            ids.join(",")
        }
    };
    format!(
        "auth={} conns=[{}] saved={saved}",
        flag(store.is_authoritative()),
        conns.join(";")
    )
}

fn key_out(k: &SshKeyRecord, m: &mut Minted) -> String {
    let hosts: Vec<String> = k.bound_hosts.iter().map(|h| m.out(h)).collect();
    [
        m.mask(&k.id.to_string()),
        m.out(&k.name),
        out(k.algorithm.raw_value()),
        k.bits.map_or_else(|| "-".to_owned(), |b| b.to_string()),
        m.out(&k.comment),
        m.out(&k.fingerprint),
        m.out(&k.public_key),
        out(k.source.raw_value()),
        if hosts.is_empty() {
            "~".to_owned()
        } else {
            hosts.join(",")
        },
        m.opt(k.keychain_ref.as_deref()),
        flag(k.require_biometry).to_owned(),
        date_out(k.created_at),
        k.last_used_at.map_or_else(|| "-".to_owned(), date_out),
        m.out(&k.badge_text()),
    ]
    .join("/")
}

fn keys_state(store: &SshKeyStore, m: &mut Minted) -> String {
    let keys: Vec<String> = store.keys().iter().map(|k| key_out(k, m)).collect();
    format!(
        "auth={} keys=[{}] dev={} imp={}",
        flag(store.is_authoritative()),
        keys.join(";"),
        store.device_keys().len(),
        store.imported_keys().len()
    )
}

fn modifiers_out(s: &TerminalShortcut) -> String {
    if s.modifiers.is_empty() {
        return "~".to_owned();
    }
    s.modifiers
        .iter()
        .map(|m| match m {
            crate::shortcuts::ShortcutModifier::Ctrl => "ctrl",
            crate::shortcuts::ShortcutModifier::Alt => "alt",
            crate::shortcuts::ShortcutModifier::Shift => "shift",
            crate::shortcuts::ShortcutModifier::Cmd => "cmd",
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn sorted_hex_set(strings: &[String], m: &mut Minted) -> String {
    let mut hosts: Vec<&String> = strings.iter().collect();
    hosts.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    if hosts.is_empty() {
        return "~".to_owned();
    }
    hosts.iter().map(|h| m.out(h)).collect::<Vec<_>>().join(",")
}

fn shortcut_out(s: &TerminalShortcut, m: &mut Minted) -> String {
    [
        m.mask(&s.id.to_string()),
        m.out(&s.summary),
        flag(s.is_builtin).to_owned(),
        s.kind.raw_value().to_owned(),
        modifiers_out(s),
        m.out(&s.key),
        m.out(&s.payload),
        flag(s.append_return).to_owned(),
        flag(s.repeat_on_hold).to_owned(),
        m.out(&s.chip_label),
        m.out(&s.color_id),
        sorted_hex_set(&s.scope_hosts, m),
        flag(s.only_in_tmux).to_owned(),
        flag(s.in_toolbar).to_owned(),
    ]
    .join("/")
}

fn short_state(store: &ShortcutStore, m: &mut Minted) -> String {
    let all: Vec<String> = store
        .shortcuts()
        .iter()
        .map(|s| shortcut_out(s, m))
        .collect();
    let bar: Vec<String> = store
        .toolbar()
        .iter()
        .map(|s| m.mask(&s.id.to_string()))
        .collect();
    format!(
        "auth={} sc=[{}] bar={} custom={} avail={}",
        flag(store.is_authoritative()),
        all.join(";"),
        if bar.is_empty() {
            "~".to_owned()
        } else {
            bar.join(",")
        },
        store.custom().len(),
        store.available().len()
    )
}

fn theme_out(t: &TerminalTheme, m: &mut Minted) -> String {
    let ansi: Vec<String> = t.ansi_hex.iter().map(|h| out(h)).collect();
    let bright = t.bright_hex.as_ref().map_or_else(
        || "-".to_owned(),
        |b| b.iter().map(|h| out(h)).collect::<Vec<_>>().join(","),
    );
    [
        m.out(&t.id),
        m.out(&t.name),
        out(&t.background_hex),
        out(&t.foreground_hex),
        out(&t.cursor_hex),
        ansi.join(","),
        bright,
        flag(t.is_built_in).to_owned(),
    ]
    .join("/")
}

fn theme_state(store: &ThemeStore, m: &mut Minted) -> String {
    let themes: Vec<String> = store
        .custom_themes()
        .iter()
        .map(|t| theme_out(t, m))
        .collect();
    format!(
        "auth={} themes=[{}]",
        flag(store.is_authoritative()),
        themes.join(";")
    )
}

fn app_theme_out(t: &AppTheme, m: &mut Minted) -> String {
    [
        m.out(&t.id),
        m.out(&t.name),
        out(&t.accent_hex),
        flag(t.is_built_in).to_owned(),
    ]
    .join("/")
}

fn app_theme_state(store: &AppThemeStore, m: &mut Minted) -> String {
    let themes: Vec<String> = store
        .custom_themes()
        .iter()
        .map(|t| app_theme_out(t, m))
        .collect();
    format!(
        "auth={} themes=[{}]",
        flag(store.is_authoritative()),
        themes.join(";")
    )
}

fn setting_out(value: &SettingValue, m: &mut Minted) -> String {
    match value {
        SettingValue::Bool(flag) => format!("b{}", self::flag(*flag)),
        SettingValue::Int(number) => format!("i{number}"),
        SettingValue::Double(number) => format!("f{}", bits(*number)),
        SettingValue::String(text) => format!("s{}", m.out(text)),
        SettingValue::StringSet(strings) => format!("a{}", sorted_hex_set(strings, m)),
    }
}

fn settings_state(settings: &Settings, mirror: &MemoryDefaults, m: &mut Minted) -> String {
    let values: Vec<String> = settings::ALL
        .iter()
        .map(|spec| {
            let value = match spec.name {
                "cursorShape" => {
                    SettingValue::String(settings.cursor_shape().raw_value().to_owned())
                }
                "predictMode" => {
                    SettingValue::String(settings.predict_mode().raw_value().to_owned())
                }
                "appearance" => SettingValue::String(settings.appearance().raw_value().to_owned()),
                "voiceEngine" => {
                    SettingValue::String(settings.voice_engine().raw_value().to_owned())
                }
                "pushRelayURL" => SettingValue::String(settings.push_relay_url()),
                _ => settings.get(spec),
            };
            format!("{}={}", spec.name, setting_out(&value, m))
        })
        .collect();
    let mirrored = |key: &str| opt_flag(mirror.get(key).and_then(|v| v.as_bool()));
    format!(
        "vals=[{}] mirror={}/{}",
        values.join(";"),
        mirrored(moshpit_seal::prefs::DETAIL_KEY),
        mirrored(moshpit_seal::prefs::SOUND_KEY)
    )
}

fn table_out(table: &Table, m: &mut Minted) -> String {
    let mut entries: Vec<(&String, &String)> = table.iter().collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    entries
        .iter()
        .map(|(k, v)| format!("{}={}", m.out(k), m.out(v)))
        .collect::<Vec<_>>()
        .join(",")
}

// MARK: - Model arguments

fn json_arg(token: &str) -> Value {
    json::parse(arg(token).as_bytes(), Dialect::Decoder).unwrap_or(Value::Null)
}

/// `@n` (the n-th element, modulo the count) or an id.
fn index_arg(token: &str, count: usize) -> Option<usize> {
    let n: usize = token.strip_prefix('@')?.parse().ok()?;
    (count > 0).then(|| n % count)
}

// MARK: - Running

/// Run every case of `cases` (one per line): one result line each, and one
/// line of what the case left stored (`STORE SNAPSHOT`, unmasked: the input
/// of a case that reads it back).
pub fn run(cases: &str) -> (String, String) {
    let mut results = String::new();
    let mut finals = String::new();
    for line in cases.lines().filter(|l| !l.is_empty()) {
        let (result, last) = run_case_with_final(line);
        results.push_str(&result);
        results.push('\n');
        finals.push_str(&last);
        finals.push('\n');
    }
    (results, finals)
}

/// The phone: unlocked or not, shared with the store's readiness.
struct Phone(Arc<AtomicBool>);

impl Phone {
    fn readiness(&self) -> Readiness {
        let unlocked = self.0.clone();
        Readiness::custom(move || unlocked.load(Ordering::SeqCst))
    }
}

/// Run one case line.
pub fn run_case(line: &str) -> String {
    run_case_with_final(line).0
}

/// Run one case line; also what it left stored.
pub fn run_case_with_final(line: &str) -> (String, String) {
    let fields: Vec<&str> = line.split(' ').collect();
    let (store, phone, snapshot) = (fields[0], fields[1], fields[2]);
    let ops = &fields[3..];
    let snapshot = parse_snapshot(snapshot);
    let mut inputs = snapshot_texts(&snapshot);
    for op in ops {
        for part in op.split(':').skip(1) {
            if let Some(bytes) = hex::decode(part) {
                inputs.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
    let mut minted = Minted::new(&inputs);
    let defaults = Arc::new(MemoryDefaults::with(snapshot.defaults.clone()));
    let keychain = Arc::new(TestKeychain::default());
    *crate::host::lock(&keychain.items) = snapshot.keychain.clone();
    let phone = Phone(Arc::new(AtomicBool::new(phone == "U")));
    let result = match store {
        "conn" => run_conn(&defaults, &phone, ops, &mut minted),
        "keys" => run_keys(&defaults, &phone, ops, &mut minted),
        "short" => run_short(&defaults, &phone, ops, &mut minted),
        "theme" => run_theme(&defaults, &phone, ops, &mut minted),
        "apptheme" => run_app_theme(&defaults, &phone, ops, &mut minted),
        "settings" => run_settings(&defaults, &phone, ops, &mut minted),
        "hosts" => run_hosts(&keychain, ops, &mut minted),
        "ready" => run_ready(&defaults, &phone, ops, &mut minted),
        "bytes" => ops
            .iter()
            .map(|op| bytes_op(op))
            .collect::<Vec<_>>()
            .join(" | "),
        "hex" => ops
            .iter()
            .map(|op| hex_op(op))
            .collect::<Vec<_>>()
            .join(" | "),
        "export" => ops
            .iter()
            .map(|op| export_op(op))
            .collect::<Vec<_>>()
            .join(" | "),
        other => panic!("no store {other:?}"),
    };
    let last = format!(
        "{store} {}",
        snapshot_token(
            &defaults.snapshot(),
            &crate::host::lock(&keychain.items).clone()
        )
    );
    (result, last)
}

/// The ops every store over preferences takes; `None` when `op` is not one.
fn common_op(op: &str, defaults: &MemoryDefaults, phone: &Phone) -> Option<String> {
    let parts: Vec<&str> = op.split(':').collect();
    match parts[0] {
        "unlock" => phone.0.store(true, Ordering::SeqCst),
        "lock" => phone.0.store(false, Ordering::SeqCst),
        "set" => {
            let key = arg(parts[1]);
            let value = plist_from_json(
                &json::parse(arg(parts[2]).as_bytes(), Dialect::Decoder).expect("a value"),
            );
            defaults.set(&key, Some(value));
        }
        "del" => defaults.set(&arg(parts[1]), None),
        _ => return None,
    }
    Some(parts[0].to_owned())
}

fn segment(name: &str, result: &str, state: String, snap: String) -> String {
    format!("{name}={result} {state} {snap}")
}

fn run_conn(defaults: &Arc<MemoryDefaults>, phone: &Phone, ops: &[&str], m: &mut Minted) -> String {
    let mut store = ConnectionStore::new(defaults.clone(), phone.readiness());
    let mut segments = vec![segment(
        "init",
        "ok",
        conn_state(&store, m),
        snap_out(defaults, m),
    )];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "fresh" => {
                    store = ConnectionStore::new(defaults.clone(), phone.readiness());
                    "ok".to_owned()
                }
                "reload" => {
                    store.reload_if_needed();
                    "ok".to_owned()
                }
                "add" | "update" => {
                    match crate::codec::decode::<ServerConnection>(&json_arg(parts[1])) {
                        Ok(connection) => {
                            if parts[0] == "add" {
                                store.add(connection);
                            } else {
                                store.update(connection);
                            }
                            "ok".to_owned()
                        }
                        Err(_) => "undecodable".to_owned(),
                    }
                }
                // Make this side write the list: re-save the first connection,
                // or add one and delete it again.
                "touch" => {
                    match store.connections().first().cloned() {
                        Some(first) => store.update(first),
                        None => {
                            let temp = ServerConnection::new(Uuid::from_bytes([7; 16]), Date(0.0));
                            store.add(temp);
                            store.delete(Uuid::from_bytes([7; 16]));
                        }
                    }
                    "ok".to_owned()
                }
                "delete" => {
                    let id = match index_arg(parts[1], store.connections().len()) {
                        Some(index) => Some(store.connections()[index].id),
                        None if parts[1].starts_with('@') => None,
                        None => Uuid::parse(&arg(parts[1])),
                    };
                    match id {
                        Some(id) => {
                            store.delete(id);
                            "ok".to_owned()
                        }
                        None => "skip".to_owned(),
                    }
                }
                other => panic!("no conn op {other:?}"),
            }
        };
        segments.push(segment(
            parts[0],
            &result,
            conn_state(&store, m),
            snap_out(defaults, m),
        ));
    }
    segments.join(" | ")
}

fn run_keys(defaults: &Arc<MemoryDefaults>, phone: &Phone, ops: &[&str], m: &mut Minted) -> String {
    let mut store = SshKeyStore::new(defaults.clone(), phone.readiness());
    let mut segments = vec![segment(
        "init",
        "ok",
        keys_state(&store, m),
        snap_out(defaults, m),
    )];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "fresh" => {
                    store = SshKeyStore::new(defaults.clone(), phone.readiness());
                    "ok".to_owned()
                }
                "reload" => {
                    store.reload_if_needed();
                    "ok".to_owned()
                }
                "add" => match SshKeyRecord::from_json(&json_arg(parts[1])) {
                    Ok(record) => {
                        store.add(record);
                        "ok".to_owned()
                    }
                    Err(_) => "undecodable".to_owned(),
                },
                "touch" => {
                    let temp = SshKeyRecord::new(
                        Uuid::from_bytes([7; 16]),
                        "touch",
                        SshKeyAlgorithm::Ed25519,
                        crate::ssh_keys::SshKeySource::Generated,
                        Date(0.0),
                    );
                    store.add(temp);
                    store.remove(Uuid::from_bytes([7; 16]));
                    "ok".to_owned()
                }
                "remove" => {
                    let id = match index_arg(parts[1], store.keys().len()) {
                        Some(index) => Some(store.keys()[index].id),
                        None if parts[1].starts_with('@') => None,
                        None => Uuid::parse(&arg(parts[1])),
                    };
                    match id {
                        Some(id) => {
                            store.remove(id);
                            "ok".to_owned()
                        }
                        None => "skip".to_owned(),
                    }
                }
                other => panic!("no keys op {other:?}"),
            }
        };
        segments.push(segment(
            parts[0],
            &result,
            keys_state(&store, m),
            snap_out(defaults, m),
        ));
    }
    segments.join(" | ")
}

fn run_short(
    defaults: &Arc<MemoryDefaults>,
    phone: &Phone,
    ops: &[&str],
    m: &mut Minted,
) -> String {
    let mut store = ShortcutStore::new(defaults.clone(), phone.readiness());
    let mut segments = vec![segment(
        "init",
        "ok",
        short_state(&store, m),
        snap_out(defaults, m),
    )];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let count = store.shortcuts().len();
        let nth = |token: &str, store: &ShortcutStore| {
            index_arg(token, store.shortcuts().len()).map(|i| store.shortcuts()[i].clone())
        };
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "fresh" => {
                    store = ShortcutStore::new(defaults.clone(), phone.readiness());
                    "ok".to_owned()
                }
                "reload" => {
                    store.reload_if_needed();
                    "ok".to_owned()
                }
                "restore" => {
                    store.restore_defaults();
                    "ok".to_owned()
                }
                "touch" => {
                    match store.shortcuts().first().cloned() {
                        Some(first) => store.update(first),
                        None => store.restore_defaults(),
                    }
                    "ok".to_owned()
                }
                "add" => match TerminalShortcut::from_json(&json_arg(parts[1])) {
                    Ok(shortcut) => {
                        store.add(shortcut);
                        "ok".to_owned()
                    }
                    Err(_) => "undecodable".to_owned(),
                },
                // `update:@n:JSON`: the n-th shortcut's id and toolbar place,
                // everything else from JSON.
                "update" => match (
                    nth(parts[1], &store),
                    TerminalShortcut::from_json(&json_arg(parts[2])),
                ) {
                    (Some(target), Ok(mut shortcut)) => {
                        shortcut.id = target.id;
                        shortcut.in_toolbar = target.in_toolbar;
                        store.update(shortcut);
                        "ok".to_owned()
                    }
                    (None, _) => "skip".to_owned(),
                    (_, Err(_)) => "undecodable".to_owned(),
                },
                "remove" | "addbar" | "rmbar" => match nth(parts[1], &store) {
                    Some(target) => {
                        match parts[0] {
                            "remove" => store.remove(target.id),
                            "addbar" => {
                                store.add_to_toolbar(target.id);
                            }
                            _ => store.remove_from_toolbar(target.id),
                        }
                        "ok".to_owned()
                    }
                    None => "skip".to_owned(),
                },
                // `move:a,b,…:t`, toolbar positions modulo its length (of
                // the toolbar the move will see: read again first).
                "move" => {
                    store.reload_if_needed();
                    let bar = store.toolbar_count();
                    if bar == 0 {
                        "skip".to_owned()
                    } else {
                        let from: BTreeSet<usize> = parts[1]
                            .split(',')
                            .map(|n| n.parse::<usize>().expect("a position") % bar)
                            .collect();
                        let to = parts[2].parse::<usize>().expect("a position") % (bar + 1);
                        store.move_in_toolbar(&from, to);
                        "ok".to_owned()
                    }
                }
                // `bar:HOST|-:0|1`: the toolbar for a connection.
                "bar" => {
                    let host = (parts[1] != "-").then(|| arg(parts[1]));
                    let ids: Vec<String> = store
                        .toolbar_for(host.as_deref(), parts[2] == "1")
                        .iter()
                        .map(|s| m.mask(&s.id.to_string()))
                        .collect();
                    if ids.is_empty() {
                        "~".to_owned()
                    } else {
                        ids.join(",")
                    }
                }
                other => panic!("no short op {other:?} ({count})"),
            }
        };
        segments.push(segment(
            parts[0],
            &result,
            short_state(&store, m),
            snap_out(defaults, m),
        ));
    }
    segments.join(" | ")
}

fn run_theme(
    defaults: &Arc<MemoryDefaults>,
    phone: &Phone,
    ops: &[&str],
    m: &mut Minted,
) -> String {
    let mut store = ThemeStore::new(defaults.clone(), phone.readiness());
    let mut segments = vec![segment(
        "init",
        "ok",
        theme_state(&store, m),
        snap_out(defaults, m),
    )];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "fresh" => {
                    store = ThemeStore::new(defaults.clone(), phone.readiness());
                    "ok".to_owned()
                }
                "reload" => {
                    store.reload_if_needed();
                    "ok".to_owned()
                }
                "save" => match TerminalTheme::from_json(&json_arg(parts[1])) {
                    Ok(theme) => {
                        store.save(theme);
                        "ok".to_owned()
                    }
                    Err(_) => "undecodable".to_owned(),
                },
                "savebuiltin" => {
                    store.save(
                        TerminalTheme::built_ins()[parts[1].parse::<usize>().expect("n") % 8]
                            .clone(),
                    );
                    "ok".to_owned()
                }
                "touch" => {
                    let mut temp = TerminalTheme::fallback();
                    temp.id = "touch".to_owned();
                    temp.is_built_in = false;
                    store.save(temp);
                    store.delete("touch");
                    "ok".to_owned()
                }
                "delete" => {
                    store.delete(&arg(parts[1]));
                    "ok".to_owned()
                }
                "removeall" => {
                    store.remove_all_custom();
                    "ok".to_owned()
                }
                // `dup:@n`: duplicate the n-th of all themes, then save it.
                "dup" => {
                    let all = store.all_themes();
                    let theme = all[index_arg(parts[1], all.len()).expect("themes")].clone();
                    let copy = store.duplicate(&theme);
                    let shown = theme_out(&copy, m);
                    store.save(copy);
                    shown
                }
                "draft" => {
                    let draft = store.make_draft();
                    let shown = theme_out(&draft, m);
                    store.save(draft);
                    shown
                }
                "get" => theme_out(&store.theme(&arg(parts[1])), m),
                "import" => match store.import_themes(&arg(parts[1])) {
                    Ok(imported) => imported
                        .iter()
                        .map(|t| theme_out(t, m))
                        .collect::<Vec<_>>()
                        .join(";"),
                    Err(themes::ImportError::NotJson) => "notJSON".to_owned(),
                    Err(themes::ImportError::NoThemes) => "noThemes".to_owned(),
                },
                other => panic!("no theme op {other:?}"),
            }
        };
        segments.push(segment(
            parts[0],
            &result,
            theme_state(&store, m),
            snap_out(defaults, m),
        ));
    }
    segments.join(" | ")
}

fn run_app_theme(
    defaults: &Arc<MemoryDefaults>,
    phone: &Phone,
    ops: &[&str],
    m: &mut Minted,
) -> String {
    let mut store = AppThemeStore::new(defaults.clone(), phone.readiness());
    let mut segments = vec![segment(
        "init",
        "ok",
        app_theme_state(&store, m),
        snap_out(defaults, m),
    )];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "fresh" => {
                    store = AppThemeStore::new(defaults.clone(), phone.readiness());
                    "ok".to_owned()
                }
                "reload" => {
                    store.reload_if_needed();
                    "ok".to_owned()
                }
                "save" => {
                    store.save(&AppTheme::custom(
                        &arg(parts[1]),
                        &arg(parts[2]),
                        &arg(parts[3]),
                    ));
                    "ok".to_owned()
                }
                "savebuiltin" => {
                    store.save(&catalog::built_ins()[parts[1].parse::<usize>().expect("n") % 4]);
                    "ok".to_owned()
                }
                "touch" => {
                    store.save(&AppTheme::custom("touch", "touch", "000000"));
                    store.delete("touch");
                    "ok".to_owned()
                }
                "delete" => {
                    store.delete(&arg(parts[1]));
                    "ok".to_owned()
                }
                "removeall" => {
                    store.remove_all_custom();
                    "ok".to_owned()
                }
                "draft" => {
                    let draft = store.make_draft(&store.theme(&arg(parts[1])));
                    let shown = app_theme_out(&draft, m);
                    store.save(&draft);
                    shown
                }
                other => panic!("no apptheme op {other:?}"),
            }
        };
        segments.push(segment(
            parts[0],
            &result,
            app_theme_state(&store, m),
            snap_out(defaults, m),
        ));
    }
    segments.join(" | ")
}

fn setting_arg(kind: Kind, token: &str) -> SettingValue {
    let text = arg(token);
    match kind {
        Kind::Bool => SettingValue::Bool(text == "1"),
        Kind::Int => SettingValue::Int(text.parse().expect("an Int")),
        Kind::Double => SettingValue::Double(f64::from_bits(
            u64::from_str_radix(&text, 16).expect("bits"),
        )),
        Kind::String => SettingValue::String(text),
        Kind::StringSet => {
            let items = json_arg(token);
            SettingValue::StringSet(
                items
                    .as_array()
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            )
        }
    }
}

fn run_settings(
    defaults: &Arc<MemoryDefaults>,
    phone: &Phone,
    ops: &[&str],
    m: &mut Minted,
) -> String {
    let mirror = Arc::new(MemoryDefaults::new());
    let mut settings = Settings::new(defaults.clone(), Some(mirror.clone()), phone.readiness());
    let mut segments = vec![segment(
        "init",
        "ok",
        settings_state(&settings, &mirror, m),
        snap_out(defaults, m),
    )];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "fresh" => {
                    settings =
                        Settings::new(defaults.clone(), Some(mirror.clone()), phone.readiness());
                    "ok".to_owned()
                }
                "reload" | "touch" => {
                    settings.reload_if_needed();
                    "ok".to_owned()
                }
                // `put:NAME:VALUE`, the value as the setting's setter takes it.
                "put" => {
                    let spec = settings::spec_named(&arg(parts[1])).expect("a setting");
                    settings.set(spec, setting_arg(spec.kind, parts[2]));
                    "ok".to_owned()
                }
                other => panic!("no settings op {other:?}"),
            }
        };
        segments.push(segment(
            parts[0],
            &result,
            settings_state(&settings, &mirror, m),
            snap_out(defaults, m),
        ));
    }
    segments.join(" | ")
}

fn run_hosts(keychain: &Arc<TestKeychain>, ops: &[&str], m: &mut Minted) -> String {
    let store = KeychainKnownHosts::new(keychain.clone());
    let mut segments = vec![format!("init=ok {}", kc_out(keychain, m))];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = match parts[0] {
            "read" => match store.read() {
                Ok(table) => format!("[{}]", table_out(&table, m)),
                Err(StoreError::Locked) => "locked".to_owned(),
                Err(StoreError::Unavailable(_)) => "unavailable".to_owned(),
            },
            "write" => {
                let map = json_arg(parts[1]);
                let table: Table = map
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| Some((k.to_owned(), v.as_str()?.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default();
                match store.write(&table) {
                    Ok(()) => "ok".to_owned(),
                    Err(StoreError::Locked) => "locked".to_owned(),
                    Err(StoreError::Unavailable(_)) => "unavailable".to_owned(),
                }
            }
            // Re-save what reads.
            "touch" => match store.read() {
                Ok(table) => match store.write(&table) {
                    Ok(()) => "ok".to_owned(),
                    Err(_) => "unwritable".to_owned(),
                },
                Err(_) => "unreadable".to_owned(),
            },
            "fresh" => "ok".to_owned(),
            "kclock" => {
                keychain.locked.store(true, Ordering::SeqCst);
                "ok".to_owned()
            }
            "kcunlock" => {
                keychain.locked.store(false, Ordering::SeqCst);
                "ok".to_owned()
            }
            other => panic!("no hosts op {other:?}"),
        };
        segments.push(format!("{}={result} {}", parts[0], kc_out(keychain, m)));
    }
    segments.join(" | ")
}

fn run_ready(
    defaults: &Arc<MemoryDefaults>,
    phone: &Phone,
    ops: &[&str],
    m: &mut Minted,
) -> String {
    let readiness = phone.readiness();
    let mut segments = vec![format!("init=ok {}", snap_out(defaults, m))];
    for op in ops {
        let parts: Vec<&str> = op.split(':').collect();
        let result = if let Some(done) = common_op(op, defaults, phone) {
            done
        } else {
            match parts[0] {
                "check" => flag(readiness.is_readable(defaults.as_ref())).to_owned(),
                "touch" | "fresh" => "ok".to_owned(),
                "mark" => {
                    readiness.mark_readable(defaults.as_ref());
                    "ok".to_owned()
                }
                other => panic!("no ready op {other:?}"),
            }
        };
        segments.push(format!("{}={result} {}", parts[0], snap_out(defaults, m)));
    }
    segments.join(" | ")
}

/// `sc:JSON`: the writes a tap on the shortcut makes, in order (`~`: none,
/// the chips the caller resolves). Only what the app still asks for: its
/// Swift kept `encodedStages()` alone (b85f84d1; the bytes and the
/// discrete-keys flag went into the Rust store, behind the stages).
fn bytes_op(op: &str) -> String {
    let parts: Vec<&str> = op.split(':').collect();
    match TerminalShortcut::from_json(&json_arg(parts[1])) {
        Ok(shortcut) => {
            let stages: Vec<String> = shortcut
                .encoded_stages()
                .iter()
                .map(|s| hex::encode(s))
                .collect();
            if stages.is_empty() {
                "bytes=~".to_owned()
            } else {
                format!("bytes={}", stages.join(","))
            }
        }
        Err(_) => "bytes=undecodable".to_owned(),
    }
}

fn hex_op(op: &str) -> String {
    let parts: Vec<&str> = op.split(':').collect();
    let f = |token: &str| f64::from_bits(u64::from_str_radix(token, 16).expect("bits"));
    let value = match parts[0] {
        "norm" => out(&themes::normalized_hex(&arg(parts[1]))),
        "rgb" => {
            let (r, g, b) = themes::rgb_components(&arg(parts[1]));
            format!("{}/{}/{}", bits(r), bits(g), bits(b))
        }
        "blend" => out(&themes::blend(&arg(parts[1]), &arg(parts[2]), f(parts[3]))),
        "bright" => out(&TerminalTheme::derived_bright(&arg(parts[1]))),
        "hexstr" => out(&themes::hex_string(f(parts[1]), f(parts[2]), f(parts[3]))),
        "custom" => {
            let theme = AppTheme::custom("x", "x", &arg(parts[1]));
            [
                &theme.accent_hex,
                &theme.accent_pressed_hex,
                &theme.screen_bg_hex,
                &theme.terminal_bg_hex,
            ]
            .iter()
            .map(|h| out(h))
            .collect::<Vec<_>>()
            .join("/")
        }
        other => panic!("no hex op {other:?}"),
    };
    format!("{}={value}", parts[0])
}

fn export_op(op: &str) -> String {
    let parts: Vec<&str> = op.split(':').collect();
    match TerminalTheme::from_json(&json_arg(parts[1])) {
        Ok(theme) => {
            let mut m = Minted::new(&[arg(parts[1])]);
            format!("export={}", out(&m.mask(&ThemeStore::export_json(&theme))))
        }
        Err(_) => "export=undecodable".to_owned(),
    }
}

// MARK: - The 1.0.8 view

/// What 1.0.8 should see of what this crate stores under `store`'s key in
/// `snapshot` (a snapshot token, as the result of a run holds it): the
/// models as 1.0.8's one-piece decode would produce them, in the format
/// `harness/swift108/main.swift` prints. `undecodable` where this crate
/// itself carried something (1.0.8 reads such a list as nothing — its own
/// rule, which no writer can change).
pub fn view108(store: &str, snapshot: &str) -> String {
    view108_knowing(store, snapshot, snapshot)
}

/// [`view108`], with ids minted since `known` (a snapshot token: what 1.0.8
/// was given before it wrote `snapshot`) printed `#N`, as 1.0.8's own view
/// of its write prints them.
pub fn view108_knowing(store: &str, snapshot: &str, known: &str) -> String {
    let snapshot = parse_snapshot(snapshot);
    let mut inputs = snapshot_texts(&parse_snapshot(known));
    inputs.push(String::new());
    let mut m = Minted::new(&inputs);
    let defaults = Arc::new(MemoryDefaults::with(snapshot.defaults.clone()));
    match store {
        "conn" => {
            let stored = stored_count(&defaults, ConnectionStore::STORAGE_KEY);
            let store = ConnectionStore::new(defaults, Readiness::Always);
            if stored.is_some_and(|n| n != store.connections().len()) {
                return "undecodable".to_owned();
            }
            let conns: Vec<String> = store
                .connections()
                .iter()
                .map(|c| conn108_out(c, &mut m))
                .collect();
            format!("conns=[{}]", conns.join(";"))
        }
        "keys" => {
            let Some(Plist::Data(data)) = defaults.get(SshKeyStore::STORAGE_KEY) else {
                return "keys=[]".to_owned();
            };
            let Ok(Value::Array(entries)) = json::parse(&data, Dialect::Decoder) else {
                return "undecodable".to_owned();
            };
            let store = SshKeyStore::new(defaults, Readiness::Always);
            if store.keys().len() != entries.len() {
                return "undecodable".to_owned();
            }
            let keys: Vec<String> = store.keys().iter().map(|k| key108_out(k, &mut m)).collect();
            format!("keys=[{}]", keys.join(";"))
        }
        "short" => {
            let Some(Plist::Data(data)) = defaults.get(ShortcutStore::STORAGE_KEY) else {
                return "sc=[]".to_owned();
            };
            let Ok(Value::Array(entries)) = json::parse(&data, Dialect::Decoder) else {
                return "undecodable".to_owned();
            };
            let mut shortcuts = Vec::new();
            for entry in &entries {
                let Ok(shortcut) = TerminalShortcut::from_json(entry) else {
                    return "undecodable".to_owned();
                };
                let mut old = shortcut.clone();
                old.kind = shortcut.kind.older_builds_name();
                shortcuts.push(shortcut_out(&old, &mut m));
            }
            format!("sc=[{}]", shortcuts.join(";"))
        }
        "theme" => {
            let stored = stored_count(&defaults, ThemeStore::STORAGE_KEY);
            let store = ThemeStore::new(defaults, Readiness::Always);
            if stored.is_some_and(|n| n != store.custom_themes().len()) {
                return "undecodable".to_owned();
            }
            let themes: Vec<String> = store
                .custom_themes()
                .iter()
                .map(|t| theme_out(t, &mut m))
                .collect();
            format!("themes=[{}]", themes.join(";"))
        }
        "apptheme" => {
            let stored = stored_count(&defaults, AppThemeStore::STORAGE_KEY);
            let store = AppThemeStore::new(defaults, Readiness::Always);
            if stored.is_some_and(|n| n != store.custom_themes().len()) {
                return "undecodable".to_owned();
            }
            let themes: Vec<String> = store
                .custom_themes()
                .iter()
                .map(|t| app_theme_out(t, &mut m))
                .collect();
            format!("themes=[{}]", themes.join(";"))
        }
        "settings" => {
            let settings = Settings::new(defaults.clone(), None, Readiness::Always);
            let values: Vec<String> = settings::ALL
                .iter()
                .filter(|spec| !settings::SINCE_108.contains(&spec.name))
                .map(|spec| {
                    let value = match spec.name {
                        "cursorShape" => {
                            SettingValue::String(settings.cursor_shape().raw_value().to_owned())
                        }
                        "predictMode" => {
                            SettingValue::String(settings.predict_mode().raw_value().to_owned())
                        }
                        "voiceEngine" => {
                            SettingValue::String(settings.voice_engine().raw_value().to_owned())
                        }
                        "pushRelayURL" => SettingValue::String(settings.push_relay_url()),
                        // 1.0.8 reads the declined hosts with
                        // `stringArray(forKey:)`: an array holding anything
                        // but strings is none at all there (B159, fixed since).
                        "hostSetupDeclined" => SettingValue::StringSet(crate::swift::dedup(
                            crate::plist::string_array(defaults.get(spec.key).as_ref())
                                .unwrap_or_default(),
                        )),
                        // 1.0.8 derived an unset icon from the accent theme
                        // (`AppIconCatalog.migratedID`; B13).
                        "appIconId" => match defaults
                            .get(spec.key)
                            .and_then(|v| v.as_str().map(str::to_owned))
                        {
                            Some(stored) => SettingValue::String(stored),
                            None => SettingValue::String(
                                match settings.app_theme_id().as_str() {
                                    "moshpit-classic" => "teal",
                                    "terminal-green" => "green",
                                    "amber-console" => "amber",
                                    _ => "default",
                                }
                                .to_owned(),
                            ),
                        },
                        _ => settings.get(spec),
                    };
                    format!("{}={}", spec.name, setting_out(&value, &mut m))
                })
                .collect();
            format!("vals=[{}]", values.join(";"))
        }
        "hosts" => match snapshot.keychain.get(known_hosts::ACCOUNT) {
            None => "map=[]".to_owned(),
            Some(data) => match known_hosts::decode(data) {
                Some(table) => format!("map=[{}]", table_out(&table, &mut m)),
                None => "undecodable".to_owned(),
            },
        },
        other => panic!("no 1.0.8 view of {other:?}"),
    }
}

/// How many entries the list under `key` holds; `Some(usize::MAX)` when
/// something is stored that is no list.
fn stored_count(defaults: &MemoryDefaults, key: &str) -> Option<usize> {
    let stored = defaults.get(key)?;
    match stored
        .as_data()
        .map(|data| json::parse(data, Dialect::Decoder))
    {
        Some(Ok(Value::Array(entries))) => Some(entries.len()),
        _ => Some(usize::MAX),
    }
}

/// A key record as 1.0.8 declares it: `algorithm` only, no `bits`.
fn key108_out(k: &SshKeyRecord, m: &mut Minted) -> String {
    let hosts: Vec<String> = k.bound_hosts.iter().map(|h| m.out(h)).collect();
    [
        m.mask(&k.id.to_string()),
        m.out(&k.name),
        out(k.algorithm.older_builds_name().raw_value()),
        m.out(&k.comment),
        m.out(&k.fingerprint),
        m.out(&k.public_key),
        out(k.source.raw_value()),
        if hosts.is_empty() {
            "~".to_owned()
        } else {
            hosts.join(",")
        },
        m.opt(k.keychain_ref.as_deref()),
        flag(k.require_biometry).to_owned(),
        date_out(k.created_at),
        k.last_used_at.map_or_else(|| "-".to_owned(), date_out),
    ]
    .join("/")
}

/// A connection as 1.0.8 sees it: the fields both have, then the ones only
/// 1.0.8 has (J1's), which a save fills in with their defaults.
fn conn108_out(c: &ServerConnection, m: &mut Minted) -> String {
    let common = conn_out(c, m);
    let common: Vec<&str> = common.split('/').collect();
    format!("{}/-/0/0/60/-/0/1/1", common[..common.len() - 3].join("/"))
}

// MARK: - Push pairings

/// Pairing files as `moshpit-seal`'s `PushPairingFile` writes them (hex of
/// the bytes): pairings made, minted, re-paired and unpaired at random.
pub fn pairing_files(seed: u64, count: usize) -> Vec<String> {
    use moshpit_seal::pairing::PushPairing;
    use moshpit_seal::pairing_file::PushPairingFile;
    let mut rng = Rng::new(seed);
    let dir = std::env::temp_dir().join(format!("store-pairings-{}-{seed}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let mut out = Vec::new();
    for n in 0..count {
        let file = PushPairingFile::new(dir.join(format!("{n}.json")));
        let mut ids = Vec::new();
        for _ in 0..1 + rng.below(4) {
            let id = if !ids.is_empty() && rng.chance(20) {
                *rng.pick(&ids)
            } else {
                rng.uuid()
            };
            ids.push(id);
            let relay = rng
                .pick(&[
                    "https://push.moshpit.cluas.eu.org",
                    "https://r.example.org/",
                    " http://127.0.0.1:8080 ",
                ])
                .to_string();
            let mut pairing = PushPairing::make(
                id,
                &gen_text(&mut rng),
                &relay,
                Date(rng.below(1_000_000_000) as f64 + 0.25),
            );
            if rng.chance(60) {
                pairing = pairing.minted(
                    &hex::encode(&rng.next_u64().to_le_bytes()),
                    Date(784_000_000.5),
                    &"ab".repeat(32),
                    rng.pick(&["production", "sandbox"]),
                );
            }
            assert!(file.upsert(&pairing));
        }
        if rng.chance(30) {
            assert!(file.remove(*rng.pick(&ids)));
        }
        out.push(hex::encode(&std::fs::read(file.path()).unwrap_or_default()));
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// A pairing file's pairings as `JSONDecoder` decodes the whole array (what
/// 1.0.8's `PushPairingStore.read` does), one line: `~` for none,
/// `undecodable` when that decode fails.
pub fn pairing_view(file_hex: &str) -> String {
    use moshpit_seal::pairing::PushPairing;
    let bytes = hex::decode(file_hex).unwrap_or_default();
    let Ok(pairings) = json::decode::<Vec<PushPairing>>(&bytes) else {
        return "undecodable".to_owned();
    };
    if pairings.is_empty() {
        return "~".to_owned();
    }
    let opt = |text: &Option<String>| text.as_deref().map_or_else(|| "-".to_owned(), out);
    pairings
        .iter()
        .map(|p| {
            [
                p.connection_id.to_string(),
                out(&p.host_label),
                out(&p.secret_hex),
                out(&p.send_token),
                out(&p.relay_url),
                date_out(p.created_at),
                opt(&p.apns_token),
                opt(&p.apns_env),
                p.send_token_issued_at
                    .map_or_else(|| "-".to_owned(), date_out),
            ]
            .join("/")
        })
        .collect::<Vec<_>>()
        .join(";")
}

// MARK: - Comparison

/// `line` with every data value that is JSON in canonical form, so results
/// from both sides compare: members sorted by key, the arrays that hold a
/// Swift `Set` (`modifiers`, `scopeHosts`) sorted, every scalar as written;
/// and the shortcut fields of Fix (files-downgrade) folded, `shortcutKind`
/// into `kind`, a built-in files chip's kind read as the key combo the
/// current Swift sees.
pub fn canonical(line: &str) -> String {
    canonical_with(line, true)
}

/// [`canonical`], without the files-chip folding: for the divergence cases,
/// where the difference is the point.
pub fn canonical_strict(line: &str) -> String {
    canonical_with(line, false)
}

fn canonical_with(line: &str, fold_files: bool) -> String {
    let line = &mask_backup_stamps(line);
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        // A data value: `x` after `=`, `[` or `,`, then hex.
        if bytes[at] == b'x'
            && at > 0
            && matches!(bytes[at - 1], b'=' | b'[' | b',')
            && bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
        {
            let end = at
                + 1
                + bytes[at + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_hexdigit())
                    .count();
            let data = hex::decode(&line[at + 1..end]).unwrap_or_default();
            match canonical_json_with(&data, fold_files) {
                Some(text) => {
                    out.push('j');
                    out.push_str(&hex::encode(text.as_bytes()));
                }
                None => out.push_str(&line[at..end]),
            }
            at = end;
        } else {
            out.push(bytes[at] as char);
            at += 1;
        }
    }
    if !fold_files {
        return out;
    }
    // Fix (files-downgrade), in the state: the current Swift reads the files
    // chip Rust writes as the inert key combo 1.0.8 reads.
    out.replace("/0/files/", "/0/keyCombo/")
        .replace("/1/files/", "/1/keyCombo/")
}

/// The time in a stamped backup account (`….backup-<unix seconds>`, Fix
/// (known-hosts-backup)), hex-encoded in a keychain dump, as `N`.
fn mask_backup_stamps(line: &str) -> String {
    let marker = hex::encode(b".backup-");
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(&marker) {
        out.push_str(&rest[..at + marker.len()]);
        rest = &rest[at + marker.len()..];
        let digits = rest
            .as_bytes()
            .chunks(2)
            .take_while(|pair| pair.len() == 2 && pair[0] == b'3' && pair[1].is_ascii_digit())
            .count();
        if digits > 0 {
            out.push_str(&hex::encode(b"N"));
            rest = &rest[digits * 2..];
        }
    }
    out.push_str(rest);
    out
}

/// JSON text in canonical form, or `None` when it isn't JSON.
pub fn canonical_json(data: &[u8]) -> Option<String> {
    canonical_json_with(data, true)
}

fn canonical_json_with(data: &[u8], fold_files: bool) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    let mut parser = Raw { text, at: 0 };
    parser.space();
    let value = parser.value()?;
    parser.space();
    (parser.at == text.len()).then(|| value.write(fold_files))
}

/// A JSON value keeping every scalar's text as written.
enum RawValue {
    Scalar(String),
    Array(Vec<RawValue>),
    /// Members as (decoded key, raw key, value).
    Object(Vec<(String, String, RawValue)>),
}

impl RawValue {
    fn write(&self, fold: bool) -> String {
        match self {
            Self::Scalar(text) => text.clone(),
            Self::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|i| i.write(fold))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Object(members) => {
                let mut members: Vec<(String, String, String)> = members
                    .iter()
                    .map(|(key, raw, value)| {
                        let written = match (key.as_str(), value) {
                            ("modifiers" | "scopeHosts", Self::Array(items)) => {
                                let mut items: Vec<String> =
                                    items.iter().map(|i| i.write(fold)).collect();
                                items.sort();
                                format!("[{}]", items.join(","))
                            }
                            _ => value.write(fold),
                        };
                        (key.clone(), raw.clone(), written)
                    })
                    .collect();
                // Fix (files-downgrade): the real kind in `kind`.
                if !fold {
                    // As written.
                } else if let Some(index) = members.iter().position(|(k, _, _)| k == "shortcutKind")
                {
                    let (_, _, real) = members.remove(index);
                    if let Some(kind) = members.iter_mut().find(|(k, _, _)| k == "kind") {
                        kind.2 = real;
                    }
                }
                // Fix (files-downgrade): a files chip as the current Swift
                // reads Rust's form of it.
                if let Some(kind) = members
                    .iter_mut()
                    .find(|(k, _, v)| fold && k == "kind" && v == "\"files\"")
                {
                    kind.2 = "\"keyCombo\"".to_owned();
                }
                members.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                format!(
                    "{{{}}}",
                    members
                        .iter()
                        .map(|(_, raw, v)| format!("{raw}:{v}"))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
    }
}

struct Raw<'a> {
    text: &'a str,
    at: usize,
}

impl Raw<'_> {
    fn space(&mut self) {
        while self.text[self.at..].starts_with([' ', '\t', '\n', '\r']) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Option<RawValue> {
        let rest = &self.text[self.at..];
        match rest.chars().next()? {
            '{' => {
                self.at += 1;
                let mut members = Vec::new();
                self.space();
                if self.text[self.at..].starts_with('}') {
                    self.at += 1;
                    return Some(RawValue::Object(members));
                }
                loop {
                    self.space();
                    let raw = self.string()?;
                    let key = match json::parse(raw.as_bytes(), Dialect::Decoder).ok()? {
                        Value::String(key) => key,
                        _ => return None,
                    };
                    self.space();
                    if !self.text[self.at..].starts_with(':') {
                        return None;
                    }
                    self.at += 1;
                    self.space();
                    let value = self.value()?;
                    members.push((key, raw, value));
                    self.space();
                    if self.text[self.at..].starts_with(',') {
                        self.at += 1;
                    } else if self.text[self.at..].starts_with('}') {
                        self.at += 1;
                        return Some(RawValue::Object(members));
                    } else {
                        return None;
                    }
                }
            }
            '[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.text[self.at..].starts_with(']') {
                    self.at += 1;
                    return Some(RawValue::Array(items));
                }
                loop {
                    self.space();
                    items.push(self.value()?);
                    self.space();
                    if self.text[self.at..].starts_with(',') {
                        self.at += 1;
                    } else if self.text[self.at..].starts_with(']') {
                        self.at += 1;
                        return Some(RawValue::Array(items));
                    } else {
                        return None;
                    }
                }
            }
            '"' => self.string().map(RawValue::Scalar),
            _ => {
                let len = rest
                    .find([',', ']', '}', ' ', '\t', '\n', '\r'])
                    .unwrap_or(rest.len());
                if len == 0 {
                    return None;
                }
                self.at += len;
                Some(RawValue::Scalar(rest[..len].to_owned()))
            }
        }
    }

    /// A string token, raw.
    fn string(&mut self) -> Option<String> {
        let rest = &self.text[self.at..];
        if !rest.starts_with('"') {
            return None;
        }
        let mut escaped = false;
        for (index, c) in rest.char_indices().skip(1) {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                self.at += index + 1;
                return Some(rest[..=index].to_owned());
            }
        }
        None
    }
}

// MARK: - Generation

/// SplitMix64: seeded, the same on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn uuid(&mut self) -> Uuid {
        let mut bytes = [0u8; 16];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_le_bytes()[..chunk.len()]);
        }
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    }
}

/// Text from every corner a user's strings reach: words, CJK, emoji (ZWJ
/// and flags), combining marks, slashes, quotes, controls, separators —
/// no U+FEFF, which trimming can make leading (Fix (bom-strip) has its own
/// cases).
fn gen_text(rng: &mut Rng) -> String {
    const PIECES: &[&str] = &[
        "dev",
        "box",
        "m1-pro",
        "studio",
        "a",
        "Z",
        "home",
        "deploy",
        " ",
        "  ",
        "/",
        "\\",
        "\"",
        "'",
        "é",
        "e\u{301}",
        "ñ",
        "Å",
        "日本",
        "服务器",
        "🚀",
        "👨‍👩‍👧",
        "🇨🇳",
        "\u{200B}",
        "\u{2028}",
        "\t",
        "\n",
        "\r\n",
        "\u{1}",
        "\u{7f}",
        "\u{a0}",
        "#",
        "0x",
        "@",
        ":",
        "-",
        ".",
        "_",
        "%",
        "&",
        "<",
        ">",
        "ΑΣ",
        "ß",
        "İ",
        "１２",
        "ＡＢ",
        "\u{301}",
        "x\u{20DD}",
        "~",
        "*",
        "😀",
    ];
    let len = rng.below(5);
    (0..len).map(|_| *rng.pick(PIECES)).collect()
}

fn json_string(text: &str) -> String {
    json::write_encoder(&Value::String(text.to_owned()), json::Style::COMPACT)
}

/// A date as `JSONEncoder`'s default strategy writes it.
fn gen_reference_date(rng: &mut Rng) -> String {
    let seconds = match rng.below(6) {
        0 => rng.below(1_000_000_000) as f64,
        1 => rng.below(1_000_000_000) as f64 + (rng.below(1_000_000) as f64) / 1_000_000.0,
        2 => -(rng.below(1_000_000_000) as f64) - 0.5,
        3 => 784_000_000.123_456,
        4 => (rng.next_u64() % 10_000_000_000) as f64 / 7.0,
        _ => 0.0,
    };
    double::encoder(seconds)
}

/// An ISO 8601 date as the connections store reads them.
fn gen_iso_date(rng: &mut Rng) -> String {
    let base = Date(rng.below(1_000_000_000) as f64);
    match rng.below(8) {
        0 => "1992-8-9T5:29:13Z".to_owned(),
        1 => base.iso8601().replace('Z', ".5Z"),
        2 => base.iso8601().replace('Z', "+08:00"),
        3 => "2026-02-30T05:29:13Z".to_owned(),
        _ => base.iso8601(),
    }
}

fn gen_int(rng: &mut Rng) -> String {
    match rng.below(16) {
        0 => "22.0".to_owned(),
        1 => "1e3".to_owned(),
        2 => "-1".to_owned(),
        3 => "9223372036854775807".to_owned(),
        4 => (rng.below(70000)).to_string(),
        5 => rng
            .pick(&[
                "9223372036854775808",
                "-9223372036854775809",
                "1.5",
                "2.2e1",
                "-0",
                "1E2",
                "1.000000000000000000001",
            ])
            .to_string(),
        _ => [22, 2222, 60001, 60999, 1080, 0][rng.below(6)].to_string(),
    }
}

/// One stored connection, mostly decodable.
fn gen_connection(rng: &mut Rng, id: Uuid, broken: bool) -> String {
    let mut fields: Vec<(String, String)> = vec![
        ("id".into(), json_string(&id.to_string())),
        ("name".into(), json_string(&gen_text(rng))),
        ("host".into(), json_string(&gen_text(rng))),
        ("port".into(), gen_int(rng)),
        ("username".into(), json_string(&gen_text(rng))),
        (
            "authMethod".into(),
            json_string(rng.pick(&["password", "key"])),
        ),
        (
            "connectionProtocol".into(),
            json_string(rng.pick(&["ssh", "mosh"])),
        ),
        ("sshPort".into(), gen_int(rng)),
        ("moshPortRangeStart".into(), gen_int(rng)),
        ("moshPortRangeEnd".into(), gen_int(rng)),
        ("createdAt".into(), json_string(&gen_iso_date(rng))),
        ("useTmux".into(), rng.pick(&["true", "false"]).to_string()),
    ];
    type Make = fn(&mut Rng) -> String;
    let optional: &[(&str, Make)] = &[
        ("moshServerPath", |r| json_string(&gen_text(r))),
        ("keychainRef", |r| {
            json_string(&format!(
                "moshpit.secret.{}",
                r.uuid().to_string().to_lowercase()
            ))
        }),
        ("lastConnectedAt", |r| json_string(&gen_iso_date(r))),
        ("predictModeRaw", |r| {
            json_string(r.pick(&["adaptive", "always", "never", "experimental", "bogus"]))
        }),
        ("roamOnCellular", |r| r.pick(&["true", "false"]).to_string()),
        ("tmuxPath", |r| json_string(&gen_text(r))),
        ("herdrPath", |r| json_string(&gen_text(r))),
        ("multiplexerRaw", |r| {
            json_string(r.pick(&["none", "tmux", "herdr", "zellij"]))
        }),
        ("sshKeyId", |r| json_string(&r.uuid().to_string())),
        ("sshKeyAlgorithmRaw", |r| {
            json_string(r.pick(&["ED25519", "ECDSA", "SE · P256", "RSA-4096"]))
        }),
        ("useSOCKSProxy", |r| r.pick(&["true", "false"]).to_string()),
        ("socksProxyHost", |r| json_string(&gen_text(r))),
        ("socksProxyPort", |r| gen_int(r)),
        // J1's dead fields and others no build reads.
        ("forwardAgent", |r| r.pick(&["true", "false"]).to_string()),
        ("compression", |r| r.pick(&["true", "false"]).to_string()),
        ("keepAliveInterval", |r| gen_int(r)),
        ("jumpHost", |r| json_string(&gen_text(r))),
        ("startupCommand", |r| json_string(&gen_text(r))),
        ("useTailscale", |r| r.pick(&["true", "false"]).to_string()),
        ("tmuxAutoAttach", |r| r.pick(&["true", "false"]).to_string()),
        ("tmuxCreateNew", |r| r.pick(&["true", "false"]).to_string()),
        ("certificateRef", |r| json_string(&gen_text(r))),
    ];
    for (key, make) in optional {
        match rng.below(10) {
            0..=3 => fields.push(((*key).into(), make(rng))),
            4 => fields.push(((*key).into(), "null".into())),
            _ => {}
        }
    }
    if broken {
        let index = rng.below(fields.len());
        match rng.below(4) {
            0 => {
                fields.remove(index);
            }
            1 => fields[index].1 = "\"oops\"".into(),
            2 => fields[index].1 = "[1]".into(),
            _ => fields.push(("authMethod".into(), "\"certificate\"".into())),
        }
        if rng.chance(30) {
            // An authMethod no build has shipped: carried.
            fields.retain(|(k, _)| k != "authMethod");
            fields.push(("authMethod".into(), "\"certificate\"".into()));
        }
    }
    shuffle(rng, &mut fields);
    if rng.chance(5) {
        // A key twice: the first one counts, for both parsers.
        let (key, _) = fields[rng.below(fields.len())].clone();
        fields.push((key, json_string(&gen_text(rng))));
    }
    format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(k, v)| format!("{}:{v}", json_string(k)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn shuffle<T>(rng: &mut Rng, items: &mut [T]) {
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
}

const ALGORITHMS: &[&str] = &["ED25519", "ECDSA-sk", "RSA-4096", "SE · P256", "ECDSA"];
const RSA_LINE: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDkYs+QICsPo/VgCPbUXwgMestI7VPDYen1WcQ6K8o6e++LVX8B/HqOMs7XL0DWIrU7J4vVuPdL7Pyvhc1Gc946Z8woOaE/BpUnydTWZCTORrjUqWAkZ5NnlWpTPf92RVXbp2qpjXXKTiDmP0COkQfbyfubOpN3O0bcy0LZQplOt4DIn5iCEuV01qU4d2OJIrZhZwxwIegMfLYSid7sYa0EoJhW2JWMoi4LuYRDhFLusotZaT/Qahxxd587CQz+Tch5hLUeeL1Cei8EI78o2ZOhguryU8zA+r3ncyrjH5hls9p91MWE8UDf7f2mK8Ecfux1GOlFhgfqNpqwEYpUQDsN fixture-rsa-openssh-none";

fn gen_public_key(rng: &mut Rng) -> String {
    match rng.below(9) {
        0 => RSA_LINE.to_owned(),
        1 => "ssh-rsa AAAAB3NzaC1yc2E= server".to_owned(),
        2 => "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY= work".to_owned(),
        3 => "ecdsa-sha2-\u{301}nistp256 AAAA".to_owned(),
        4 => "sk-ecdsa-sha2-nistp256@openssh.com AAAA yubikey".to_owned(),
        5 => "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA== laptop".to_owned(),
        6 => "ssh-rsa  !!!".to_owned(),
        7 => String::new(),
        _ => gen_text(rng),
    }
}

/// One stored key record: today's shape, 1.0.8's, or broken.
fn gen_key(rng: &mut Rng, id: Uuid, broken: bool) -> String {
    let real = *rng.pick(ALGORITHMS);
    let older = if real == "ECDSA" && rng.chance(80) {
        "ECDSA-sk"
    } else {
        real
    };
    let mut fields: Vec<(String, String)> = vec![
        ("id".into(), json_string(&id.to_string())),
        ("name".into(), json_string(&gen_text(rng))),
        ("algorithm".into(), json_string(older)),
        ("comment".into(), json_string(&gen_text(rng))),
        (
            "fingerprint".into(),
            json_string(&format!("SHA256:{}", gen_text(rng))),
        ),
        ("publicKey".into(), json_string(&gen_public_key(rng))),
        (
            "source".into(),
            json_string(rng.pick(&["secureEnclave", "generated", "imported", "hardware"])),
        ),
        (
            "boundHosts".into(),
            format!(
                "[{}]",
                (0..rng.below(3))
                    .map(|_| json_string(&gen_text(rng)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ),
        (
            "requireBiometry".into(),
            rng.pick(&["true", "false"]).to_string(),
        ),
        ("createdAt".into(), gen_reference_date(rng)),
    ];
    if rng.chance(60) {
        fields.push((
            "keyAlgorithm".into(),
            json_string(if rng.chance(90) { real } else { "ED448" }),
        ));
    }
    if rng.chance(40) {
        fields.push((
            "bits".into(),
            rng.pick(&["2048", "3072", "4096", "\"2048\"", "1.5", "null"])
                .to_string(),
        ));
    }
    if rng.chance(50) {
        fields.push((
            "keychainRef".into(),
            json_string(&format!(
                "moshpit.secret.{}",
                rng.uuid().to_string().to_lowercase()
            )),
        ));
    }
    if rng.chance(30) {
        fields.push(("lastUsedAt".into(), gen_reference_date(rng)));
    }
    if broken {
        let index = rng.below(fields.len());
        match rng.below(3) {
            0 => {
                fields.remove(index);
            }
            1 => fields[index].1 = "{}".into(),
            _ => fields[2].1 = json_string("ED448-sk"),
        }
    }
    shuffle(rng, &mut fields);
    format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(k, v)| format!("{}:{v}", json_string(k)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

const KINDS: &[&str] = &[
    "keyCombo", "text", "command", "dpad", "arrows", "paste", "scroll", "ctrl", "mic", "image",
    "files",
];

fn gen_shortcut_fields(rng: &mut Rng, id: Uuid) -> Vec<(String, String)> {
    let mods: Vec<String> = ["ctrl", "alt", "shift", "cmd"]
        .iter()
        .filter(|_| rng.chance(25))
        .map(|m| json_string(m))
        .collect();
    let key = match rng.below(8) {
        0 => "esc".to_owned(),
        1 => rng
            .pick(&[
                "UP", "Down", "left", "RIGHT", "Home", "end", "PgUp", "pagedown", "Tab", "Return",
                "space", "enter", "Escape",
            ])
            .to_string(),
        2 => rng
            .pick(&[
                "c", "l", "u", "k", "A", "z", "1", "é", "ß", "ΑΣ", "İ", "😀", "",
            ])
            .to_string(),
        _ => gen_text(rng),
    };
    let payload = match rng.below(6) {
        0 => rng
            .pick(&[
                "\\t\\r",
                "\\e\\r",
                "\\r",
                "a\\tb\\r\\e",
                "\\r\u{301}",
                "\\\\r",
                "deploy\\r",
                "\\n\\n",
            ])
            .to_string(),
        1 | 2 => String::new(),
        _ => gen_text(rng),
    };
    let hosts: Vec<String> = (0..rng.below(3))
        .map(|_| json_string(&gen_text(rng)))
        .collect();
    vec![
        ("id".into(), json_string(&id.to_string())),
        ("kind".into(), json_string(rng.pick(KINDS))),
        ("modifiers".into(), format!("[{}]", mods.join(","))),
        ("key".into(), json_string(&key)),
        ("payload".into(), json_string(&payload)),
        (
            "appendReturn".into(),
            rng.pick(&["true", "false"]).to_string(),
        ),
        (
            "repeatOnHold".into(),
            rng.pick(&["true", "false"]).to_string(),
        ),
        ("chipLabel".into(), json_string(&gen_text(rng))),
        ("summary".into(), json_string(&gen_text(rng))),
        (
            "colorId".into(),
            json_string(rng.pick(&["gray", "accent", "mosh", "amber"])),
        ),
        ("scopeHosts".into(), format!("[{}]", hosts.join(","))),
        (
            "onlyInTmux".into(),
            rng.pick(&["true", "false"]).to_string(),
        ),
        ("isBuiltin".into(), "false".to_string()),
        ("inToolbar".into(), "false".to_string()),
    ]
}

fn object(fields: &[(String, String)]) -> String {
    format!(
        "{{{}}}",
        fields
            .iter()
            .map(|(k, v)| format!("{}:{v}", json_string(k)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// A stored shortcut set: the built-ins (some pruned, some in the 1.0.8
/// or this crate's form), custom chips, chips this build can't read; at
/// most twelve marked for the toolbar.
fn gen_shortcut_set(rng: &mut Rng) -> String {
    let mut entries = Vec::new();
    let mut in_bar = 0;
    for builtin in ShortcutStore::builtins() {
        if rng.chance(20) {
            continue;
        }
        let mut sc = builtin.clone();
        sc.id = rng.uuid();
        sc.in_toolbar = sc.in_toolbar && in_bar < 12;
        in_bar += usize::from(sc.in_toolbar);
        let text = match rng.below(10) {
            // This crate's form.
            0 => sc.to_json(),
            // As 1.0.8 or the app wrote it: the real kind in `kind`.
            _ => {
                let value = json::parse(sc.to_json().as_bytes(), Dialect::Decoder).expect("JSON");
                let mut object = value.as_object().expect("an object").clone();
                object.remove("shortcutKind");
                object.insert("kind", Value::String(sc.kind.raw_value().to_owned()));
                crate::codec::write(&Value::Object(object))
            }
        };
        entries.push(text);
    }
    for _ in 0..rng.below(4) {
        let mut fields = new_shortcut_fields(rng);
        if in_bar < 12 && rng.chance(50) {
            fields.retain(|(k, _)| k != "inToolbar");
            fields.push(("inToolbar".into(), "true".into()));
            in_bar += 1;
        }
        if rng.chance(15) {
            fields.retain(|(k, _)| k != "kind");
            fields.push(("kind".into(), json_string("macro")));
        }
        if rng.chance(10) {
            // A built-in this build can't read.
            fields.retain(|(k, _)| k != "kind" && k != "isBuiltin" && k != "summary");
            fields.push(("kind".into(), json_string("pasteV2")));
            fields.push(("isBuiltin".into(), rng.pick(&["true", "1"]).to_string()));
            fields.push(("summary".into(), json_string("Paste clipboard")));
        }
        shuffle(rng, &mut fields);
        let at = rng.below(entries.len() + 1);
        entries.insert(at, object(&fields));
    }
    format!("[{}]", entries.join(","))
}

fn gen_hex_colour(rng: &mut Rng) -> String {
    let digits = "0123456789abcdefABCDEF";
    let random = |rng: &mut Rng, n: usize| -> String {
        (0..n)
            .map(|_| digits.as_bytes()[rng.below(digits.len())] as char)
            .collect()
    };
    match rng.below(14) {
        0 => format!("#{}", random(rng, 3)),
        1 => format!("0x{}", random(rng, 6)),
        2 => format!(" #{} ", random(rng, 6)),
        3 => format!("#{}", random(rng, 8)),
        4 => "#\u{FF21}\u{FF22}\u{FF23}".to_owned(),
        5 => format!("AB\u{FF23}{}", random(rng, 3)),
        6 => "#\u{301}abc".to_owned(),
        7 => gen_text(rng),
        8 => String::new(),
        9 => "0X0Xabcdef".to_owned(),
        10 => "ﬀ".to_owned(),
        _ => format!("#{}", random(rng, 6)),
    }
}

/// A stored or pasted theme: any of the names the decoder takes.
fn gen_theme(rng: &mut Rng, with_id: bool) -> String {
    let mut fields: Vec<(String, String)> = Vec::new();
    if with_id {
        fields.push((
            "id".into(),
            json_string(&format!(
                "custom-{}",
                rng.uuid().to_string()[..8].to_lowercase()
            )),
        ));
    }
    if rng.chance(85) {
        fields.push(("name".into(), json_string(&gen_text(rng))));
    }
    let colour = |rng: &mut Rng| -> String {
        if rng.chance(10) {
            rng.pick(&["5", "null", "[]", "\"\""]).to_string()
        } else {
            json_string(&gen_hex_colour(rng))
        }
    };
    for names in [
        &["background", "bg"][..],
        &["foreground", "fg", "text"],
        &["cursor", "cursorColor", "caret"],
    ] {
        if rng.chance(80) {
            let name = *rng.pick(names);
            let value = colour(rng);
            fields.push((name.into(), value));
        }
    }
    for slot in AnsiSlot::ALL {
        if rng.chance(85) {
            let name = if rng.chance(80) {
                slot.key().to_owned()
            } else {
                format!("ansi{}", slot.index())
            };
            let value = colour(rng);
            fields.push((name, value));
        }
        if rng.chance(25) {
            let name = if rng.chance(70) {
                slot.bright_key()
            } else {
                format!("ansi{}", slot.index() + 8)
            };
            let value = colour(rng);
            fields.push((name, value));
        }
    }
    shuffle(rng, &mut fields);
    object(&fields)
}

fn new_shortcut_fields(rng: &mut Rng) -> Vec<(String, String)> {
    let id = rng.uuid();
    gen_shortcut_fields(rng, id)
}

fn new_connection(rng: &mut Rng, id: Option<Uuid>, broken_percent: usize) -> String {
    let id = id.unwrap_or_else(|| rng.uuid());
    let broken = rng.chance(broken_percent);
    gen_connection(rng, id, broken)
}

fn new_key(rng: &mut Rng, id: Option<Uuid>, broken_percent: usize) -> String {
    let id = id.unwrap_or_else(|| rng.uuid());
    let broken = rng.chance(broken_percent);
    gen_key(rng, id, broken)
}

fn new_theme(rng: &mut Rng, id_percent: usize) -> String {
    let with_id = rng.chance(id_percent);
    gen_theme(rng, with_id)
}

fn data_value(text: &str) -> Plist {
    Plist::Data(text.as_bytes().to_vec())
}

/// A key's stored list, or something that isn't one.
fn gen_list_value(rng: &mut Rng, entries: Vec<String>) -> Plist {
    match rng.below(48) {
        0 => data_value("not valid json {{{"),
        1 => data_value("{\"version\":2,\"items\":[]}"),
        2 => Plist::String("a list as a string".into()),
        3 => data_value(""),
        4 => Plist::Array(vec![Plist::String("a".into())]),
        5 => data_value(&format!("[{},]", entries.join(","))),
        6 => data_value(&format!("\u{FEFF}[{}]", entries.join(","))),
        7 => data_value(&format!(" \n[{}]\t ", entries.join(","))),
        8 => Plist::Data(vec![0xFF, 0xFE, b'[', b']']),
        9 => data_value(&format!("[{}] x", entries.join(","))),
        10 => Plist::Int(7),
        _ => data_value(&format!("[{}]", entries.join(","))),
    }
}

/// Sometimes a backup is there already: copies of earlier values, up to the
/// limit, or something that is no array.
fn gen_backup(rng: &mut Rng, key: &str, defaults: &mut BTreeMap<String, Plist>) {
    if !rng.chance(12) {
        return;
    }
    let value = match rng.below(4) {
        0 => Plist::String("not an array".into()),
        1 => Plist::Array(
            (0..5)
                .map(|n| data_value(&format!("garbage {n}")))
                .collect(),
        ),
        2 => Plist::Array(vec![data_value("not valid json {{{"), Plist::Int(1)]),
        _ => Plist::Array(vec![
            defaults.get(key).cloned().unwrap_or(Plist::Bool(true)),
        ]),
    };
    defaults.insert(crate::stored_list::backup_key(key), value);
}

fn phone_and_sentinel(rng: &mut Rng, defaults: &mut BTreeMap<String, Plist>) -> &'static str {
    if rng.chance(25) {
        if rng.chance(40) {
            defaults.insert(
                crate::readiness::SENTINEL_KEY.into(),
                rng.pick(&[
                    Plist::Bool(true),
                    Plist::Int(1),
                    Plist::String("YES".into()),
                    Plist::Bool(false),
                ])
                .clone(),
            );
        }
        "L"
    } else {
        "U"
    }
}

fn hex_arg(text: &str) -> String {
    out(text)
}

/// Generate `count` cases from `seed`.
pub fn generate(seed: u64, count: usize) -> Vec<String> {
    let mut rng = Rng::new(seed);
    (0..count).map(|_| gen_case(&mut rng)).collect()
}

fn lifecycle_op(rng: &mut Rng) -> String {
    rng.pick(&["fresh", "reload", "unlock", "lock", "unlock"])
        .to_string()
}

fn gen_case(rng: &mut Rng) -> String {
    let mut defaults = BTreeMap::new();
    let mut keychain = BTreeMap::new();
    let mut ops: Vec<String> = Vec::new();
    let store = *rng.pick(&[
        "conn", "conn", "conn", "keys", "keys", "short", "short", "short", "theme", "theme",
        "apptheme", "settings", "settings", "hosts", "ready", "bytes", "hex", "export",
    ]);
    let mut phone = "U";
    match store {
        "conn" => {
            let ids: Vec<Uuid> = (0..rng.below(5)).map(|_| rng.uuid()).collect();
            if !ids.is_empty() || rng.chance(50) {
                let entries = ids
                    .iter()
                    .map(|id| new_connection(rng, Some(*id), 15))
                    .collect();
                defaults.insert(
                    ConnectionStore::STORAGE_KEY.into(),
                    gen_list_value(rng, entries),
                );
            }
            gen_backup(rng, ConnectionStore::STORAGE_KEY, &mut defaults);
            phone = phone_and_sentinel(rng, &mut defaults);
            for _ in 0..1 + rng.below(5) {
                ops.push(match rng.below(7) {
                    0 if !ids.is_empty() && rng.chance(20) => {
                        let id = *rng.pick(&ids);
                        format!("add:{}", hex_arg(&new_connection(rng, Some(id), 0)))
                    }
                    0 | 1 => format!("add:{}", hex_arg(&new_connection(rng, None, 10))),
                    2 => {
                        let id = if !ids.is_empty() && rng.chance(80) {
                            *rng.pick(&ids)
                        } else {
                            rng.uuid()
                        };
                        format!("update:{}", hex_arg(&gen_connection(rng, id, false)))
                    }
                    3 => format!("delete:@{}", rng.below(6)),
                    _ => lifecycle_op(rng),
                });
            }
        }
        "keys" => {
            let ids: Vec<Uuid> = (0..rng.below(5)).map(|_| rng.uuid()).collect();
            if !ids.is_empty() || rng.chance(50) {
                let entries = ids.iter().map(|id| new_key(rng, Some(*id), 15)).collect();
                defaults.insert(
                    SshKeyStore::STORAGE_KEY.into(),
                    gen_list_value(rng, entries),
                );
            }
            gen_backup(rng, SshKeyStore::STORAGE_KEY, &mut defaults);
            phone = phone_and_sentinel(rng, &mut defaults);
            for _ in 0..1 + rng.below(4) {
                ops.push(match rng.below(5) {
                    0 | 1 => format!("add:{}", hex_arg(&new_key(rng, None, 10))),
                    2 => format!("remove:@{}", rng.below(6)),
                    _ => lifecycle_op(rng),
                });
            }
        }
        "short" => {
            if rng.chance(70) {
                let set = gen_shortcut_set(rng);
                defaults.insert(
                    ShortcutStore::STORAGE_KEY.into(),
                    gen_list_value(rng, vec![set[1..set.len() - 1].to_owned()]),
                );
            }
            gen_backup(rng, ShortcutStore::STORAGE_KEY, &mut defaults);
            phone = phone_and_sentinel(rng, &mut defaults);
            for _ in 0..1 + rng.below(5) {
                ops.push(match rng.below(12) {
                    0 | 1 => format!("add:{}", hex_arg(&object(&new_shortcut_fields(rng)))),
                    2 => format!(
                        "update:@{}:{}",
                        rng.below(40),
                        hex_arg(&object(&new_shortcut_fields(rng)))
                    ),
                    3 => format!("remove:@{}", rng.below(40)),
                    4 => format!("addbar:@{}", rng.below(40)),
                    5 => format!("rmbar:@{}", rng.below(40)),
                    6 => {
                        let from: Vec<String> = (0..1 + rng.below(3))
                            .map(|_| rng.below(12).to_string())
                            .collect();
                        format!("move:{}:{}", from.join(","), rng.below(13))
                    }
                    7 => "restore".to_owned(),
                    8 => format!(
                        "bar:{}:{}",
                        if rng.chance(30) {
                            "-".to_owned()
                        } else {
                            hex_arg(&gen_text(rng))
                        },
                        rng.below(2)
                    ),
                    _ => lifecycle_op(rng),
                });
            }
        }
        "theme" => {
            if rng.chance(70) {
                let entries = (0..rng.below(4))
                    .map(|_| {
                        if rng.chance(10) {
                            json_string("a theme stored as a string")
                        } else if rng.chance(8) {
                            "{\"name\":5}".to_owned()
                        } else {
                            gen_theme(rng, true)
                        }
                    })
                    .collect();
                defaults.insert(ThemeStore::STORAGE_KEY.into(), gen_list_value(rng, entries));
            }
            gen_backup(rng, ThemeStore::STORAGE_KEY, &mut defaults);
            phone = phone_and_sentinel(rng, &mut defaults);
            for _ in 0..1 + rng.below(4) {
                ops.push(match rng.below(11) {
                    0 | 1 => format!("save:{}", hex_arg(&gen_theme(rng, true))),
                    2 => format!("savebuiltin:{}", rng.below(8)),
                    3 => format!(
                        "delete:{}",
                        hex_arg(&format!(
                            "custom-{}",
                            rng.uuid().to_string()[..8].to_lowercase()
                        ))
                    ),
                    4 => format!("dup:@{}", rng.below(12)),
                    5 => "draft".to_owned(),
                    6 => format!(
                        "import:{}",
                        hex_arg(&match rng.below(5) {
                            0 => format!("[{},{}]", new_theme(rng, 50), gen_theme(rng, false)),
                            1 => rng
                                .pick(&["[]", "  ", "[1]", "not json", "{\"name\":5}"])
                                .to_string(),
                            _ => new_theme(rng, 50),
                        })
                    ),
                    7 => format!(
                        "get:{}",
                        hex_arg(rng.pick(&["github-dark", "nord", "nope", ""]))
                    ),
                    8 => "removeall".to_owned(),
                    _ => lifecycle_op(rng),
                });
            }
        }
        "apptheme" => {
            if rng.chance(70) {
                let entries = (0..rng.below(4))
                    .map(|_| {
                        if rng.chance(10) {
                            "{\"id\":\"app-x\"}".to_owned()
                        } else {
                            object(&[
                                (
                                    "id".into(),
                                    json_string(&format!(
                                        "app-{}",
                                        rng.uuid().to_string()[..8].to_lowercase()
                                    )),
                                ),
                                ("name".into(), json_string(&gen_text(rng))),
                                ("accent".into(), json_string(&gen_hex_colour(rng))),
                            ])
                        }
                    })
                    .collect();
                defaults.insert(
                    AppThemeStore::STORAGE_KEY.into(),
                    gen_list_value(rng, entries),
                );
            }
            gen_backup(rng, AppThemeStore::STORAGE_KEY, &mut defaults);
            phone = phone_and_sentinel(rng, &mut defaults);
            for _ in 0..1 + rng.below(4) {
                ops.push(match rng.below(8) {
                    0 | 1 => format!(
                        "save:{}:{}:{}",
                        hex_arg(&format!(
                            "app-{}",
                            rng.uuid().to_string()[..8].to_lowercase()
                        )),
                        hex_arg(rng.pick(&["Signal Room", "Mine", " ", "Teal Line", "Mine 2"])),
                        hex_arg(&gen_hex_colour(rng))
                    ),
                    2 => format!("savebuiltin:{}", rng.below(4)),
                    3 => format!("delete:{}", hex_arg("app-1")),
                    4 => format!(
                        "draft:{}",
                        hex_arg(rng.pick(&["signal-room", "amber-console", "nope"]))
                    ),
                    5 => "removeall".to_owned(),
                    _ => lifecycle_op(rng),
                });
            }
        }
        "settings" => {
            for spec in settings::ALL.iter() {
                if !rng.chance(30) {
                    continue;
                }
                let value = match rng.below(10) {
                    8 => Plist::Date(784_000_000.5),
                    9 => Plist::Dictionary(vec![("a".into(), Plist::Int(1))]),
                    0 => Plist::Bool(rng.chance(50)),
                    1 => Plist::Int(*rng.pick(&[0, 1, 2, -1, 60001, 7])),
                    2 => Plist::Double(*rng.pick(&[0.0, 1.0, 0.5, 12.0, 18.5, 1e300])),
                    3 => Plist::String(
                        rng.pick(&[
                            "block", "bar", "BLOCK", "never", "whisper", "nord", "", "12", "1",
                        ])
                        .to_string(),
                    ),
                    4 => Plist::String(gen_text(rng)),
                    5 => Plist::Array(vec![
                        Plist::String(gen_text(rng)),
                        Plist::String(gen_text(rng)),
                    ]),
                    6 => Plist::Array(vec![Plist::String("a".into()), Plist::Int(1)]),
                    _ => Plist::Data(b"x".to_vec()),
                };
                defaults.insert(spec.key.into(), value);
            }
            phone = phone_and_sentinel(rng, &mut defaults);
            for _ in 0..1 + rng.below(5) {
                if rng.chance(25) {
                    ops.push(lifecycle_op(rng));
                    continue;
                }
                let spec = rng.pick(&settings::ALL);
                let value = match spec.kind {
                    Kind::Bool => rng.pick(&["0", "1"]).to_string(),
                    Kind::Int => rng
                        .pick(&["0", "7", "60000", "-5", "9223372036854775807"])
                        .to_string(),
                    Kind::Double => bits(*rng.pick(&[9.0, 18.0, 0.1, -0.0, 1e-300])),
                    Kind::String => match spec.name {
                        "cursorShape" => {
                            rng.pick(&["block", "bar", "underline", "Bar"]).to_string()
                        }
                        "predictMode" => rng
                            .pick(&["adaptive", "always", "never", "experimental", "x"])
                            .to_string(),
                        "voiceEngine" => rng.pick(&["apple", "whisper", "Apple"]).to_string(),
                        "appearance" => rng
                            .pick(&["system", "light", "dark", "Dark", "auto"])
                            .to_string(),
                        _ => gen_text(rng),
                    },
                    Kind::StringSet => format!(
                        "[{}]",
                        (0..rng.below(4))
                            .map(|_| json_string(&gen_text(rng)))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                };
                ops.push(format!("put:{}:{}", hex_arg(spec.name), hex_arg(&value)));
            }
        }
        "hosts" => {
            let fingerprint =
                |rng: &mut Rng| format!("SHA256:{}", hex::encode(&rng.next_u64().to_le_bytes()));
            if rng.chance(80) {
                let map = match rng.below(8) {
                    0 => "garbage".to_owned(),
                    1 => "[1]".to_owned(),
                    2 => format!("{{\"a:22\":\"{}\",\"b:22\":{{\"v\":2}}}}", fingerprint(rng)),
                    3 => format!("{{\"a:22\":\"{}\",\"b:22\":5}}", fingerprint(rng)),
                    _ => {
                        let mut seen = HashSet::new();
                        let mut entries = Vec::new();
                        for _ in 0..rng.below(4) {
                            let host = format!("{}:{}", gen_text(rng), rng.below(70000));
                            if seen.insert(crate::swift::canonical(&host)) {
                                entries.push(format!(
                                    "{}:{}",
                                    json_string(&host),
                                    json_string(&fingerprint(rng))
                                ));
                            }
                        }
                        format!("{{{}}}", entries.join(","))
                    }
                };
                keychain.insert(known_hosts::ACCOUNT.to_owned(), map.into_bytes());
            }
            // An earlier copy only beside a map that reads (Fix
            // (known-hosts-backup) has its own cases).
            let decodable = keychain
                .get(known_hosts::ACCOUNT)
                .is_none_or(|map| known_hosts::decode(map).is_some());
            if decodable && rng.chance(15) {
                keychain.insert(
                    format!("{}.backup", known_hosts::ACCOUNT),
                    b"garbage".to_vec(),
                );
            }
            for _ in 0..1 + rng.below(4) {
                ops.push(match rng.below(6) {
                    0 | 1 => "read".to_owned(),
                    2 | 3 => {
                        let mut seen = HashSet::new();
                        let mut entries = Vec::new();
                        for _ in 0..rng.below(3) {
                            let host = format!("{}:22", gen_text(rng));
                            if seen.insert(crate::swift::canonical(&host)) {
                                entries.push(format!(
                                    "{}:{}",
                                    json_string(&host),
                                    json_string(&fingerprint(rng))
                                ));
                            }
                        }
                        format!("write:{}", hex_arg(&format!("{{{}}}", entries.join(","))))
                    }
                    4 => "kclock".to_owned(),
                    _ => "kcunlock".to_owned(),
                });
            }
        }
        "ready" => {
            if rng.chance(70) {
                defaults.insert(
                    crate::readiness::SENTINEL_KEY.into(),
                    rng.pick(&[
                        Plist::Bool(true),
                        Plist::Bool(false),
                        Plist::Int(1),
                        Plist::Int(2),
                        Plist::Double(0.5),
                        Plist::String("YES".into()),
                        Plist::String("yes".into()),
                        Plist::String("true".into()),
                        Plist::String("1".into()),
                        Plist::String("9".into()),
                        Plist::Data(b"1".to_vec()),
                    ])
                    .clone(),
                );
            }
            phone = if rng.chance(50) { "L" } else { "U" };
            for _ in 0..1 + rng.below(4) {
                ops.push(
                    rng.pick(&["check", "mark", "unlock", "lock", "check"])
                        .to_string(),
                );
            }
        }
        "bytes" => {
            for _ in 0..1 + rng.below(4) {
                let mut fields = new_shortcut_fields(rng);
                fields.retain(|(k, _)| k != "kind");
                fields.push((
                    "kind".into(),
                    json_string(
                        rng.pick(&["keyCombo", "keyCombo", "text", "text", "command", "files"]),
                    ),
                ));
                ops.push(format!("sc:{}", hex_arg(&object(&fields))));
            }
        }
        "hex" => {
            for _ in 0..1 + rng.below(5) {
                let f = |rng: &mut Rng| {
                    bits(*rng.pick(&[
                        0.0,
                        1.0,
                        0.5,
                        -3.0,
                        5.0,
                        0.18,
                        0.955,
                        0.982,
                        0.32,
                        0.0019607843,
                    ]))
                };
                ops.push(match rng.below(6) {
                    0 => format!("norm:{}", hex_arg(&gen_hex_colour(rng))),
                    1 => format!("rgb:{}", hex_arg(&gen_hex_colour(rng))),
                    2 => format!(
                        "blend:{}:{}:{}",
                        hex_arg(&gen_hex_colour(rng)),
                        hex_arg(&gen_hex_colour(rng)),
                        f(rng)
                    ),
                    3 => format!("bright:{}", hex_arg(&gen_hex_colour(rng))),
                    4 => format!("hexstr:{}:{}:{}", f(rng), f(rng), f(rng)),
                    _ => format!("custom:{}", hex_arg(&gen_hex_colour(rng))),
                });
            }
        }
        "export" => {
            for _ in 0..1 + rng.below(3) {
                ops.push(format!("theme:{}", hex_arg(&gen_theme(rng, true))));
            }
        }
        _ => unreachable!(),
    }
    format!(
        "{store} {phone} {} {}",
        snapshot_token(&defaults, &keychain),
        ops.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cases_run() {
        for case in generate(7, 300) {
            let result = run_case(&case);
            assert!(!result.is_empty());
            let _ = canonical(&result);
        }
    }

    #[test]
    fn canonical_json_sorts_members_and_sets_and_keeps_scalars() {
        assert_eq!(
            canonical_json(br#"{"b":1.0,"a":"x\/y","modifiers":["shift","ctrl"],"n":[2,1]}"#)
                .unwrap(),
            r#"{"a":"x\/y","b":1.0,"modifiers":["ctrl","shift"],"n":[2,1]}"#
        );
        assert_eq!(
            canonical_json(br#"{"kind":"keyCombo","shortcutKind":"image"}"#).unwrap(),
            r#"{"kind":"image"}"#
        );
        assert!(canonical_json(b"not json").is_none());
    }

    #[test]
    fn minted_ids_are_numbered_in_order() {
        let mut m = Minted::new(&["3F2504E0-4F89-11D3-9A0C-0305E82C3301".to_owned()]);
        assert_eq!(
            m.mask("3F2504E0-4F89-11D3-9A0C-0305E82C3301 0F1E2D3C-4B5A-4978-8695-A4B3C2D1E0F1 custom-0a1b2c3d"),
            "3F2504E0-4F89-11D3-9A0C-0305E82C3301 #1 #2"
        );
        assert_eq!(m.mask("custom-0a1b2c3d"), "#2");
    }
}
