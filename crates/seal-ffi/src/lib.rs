//! The notification service extension's one call into `moshpit-seal` (E7b).
//!
//! iOS runs the extension for every push carrying `mutable-content: 1`, the
//! app often not running and the phone often locked; it is its own binary,
//! linking this static library and nothing else of the app's Rust. One call
//! does the whole of [`moshpit_seal::receive`]: open the envelope with this
//! device's pairings, keep the standing set, the rung finishes and the
//! closed turns in the App Group, leave the diagnostic lines, and say what
//! to show. Swift maps the answer onto `UNMutableNotificationContent`, and
//! nothing more: the rules live once, here, shared with the app.
//!
//! The request is a JSON object:
//!
//! | key | |
//! | --- | --- |
//! | `userInfo` | the push's `userInfo` (an object carrying `mp`) |
//! | `pairings`, `diagnostics` | their files, at the App Group container's root |
//! | `standing`, `finishes` | their files, in its device-only folder (B785: kept out of backups; the closed turns go beside the standing set) |
//! | `now` | Unix seconds |
//! | `sound`, `showDetail` | the user's two switches (the App Group's `UserDefaults`) |
//! | `language` | the language Moshpit shows (a BCP 47 tag) |
//!
//! The answer, a JSON object: `delivery` (`asReceived`, `fallback` or
//! `card`), `secretsTried` with a fallback, `card` with a card (`title`,
//! `body`, `sound`, `level`: `passive`, `active` or `timeSensitive`, and
//! `userInfo`), `soundAllowed`, `waiting` (B669: `{"redraw": {"id",
//! "card"}}` or `{"withdraw": {"id"}}`, or absent) and the `diagnostics`
//! lines, already written to the ring. `{"note", "diagnostics"}` only
//! writes `note` to the ring (the extension stopped by iOS).

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;

use moshpit_seal::diagnostics::DiagnosticsFile;
use moshpit_seal::foundation::json::{self, Dialect};
use moshpit_seal::notification::{AGENT_KEY, CONNECTION_KEY, DETAIL_KEY, PANE_KEY, STATE_KEY};
use moshpit_seal::receive::Waiting;
use moshpit_seal::{
    Card, Date, Delivery, InterruptionLevel, Language, Prefs, PushFinishesFile, PushPairingFile,
    PushStandingFile, receive,
};
use serde_json::{Value, json};

/// What the extension tells this call.
struct Request {
    user_info: Value,
    pairings: PathBuf,
    diagnostics: PathBuf,
    standing: PathBuf,
    finishes: PathBuf,
    now: f64,
    prefs: Prefs,
    language: Language,
}

impl Request {
    fn parse(text: &str) -> Option<Request> {
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let path = |key: &str| object.get(key).and_then(Value::as_str).map(PathBuf::from);
        Some(Request {
            user_info: object.get("userInfo").cloned().unwrap_or(Value::Null),
            pairings: path("pairings")?,
            diagnostics: path("diagnostics")?,
            standing: path("standing")?,
            finishes: path("finishes")?,
            now: object.get("now")?.as_f64()?,
            prefs: Prefs {
                sound: object.get("sound").and_then(Value::as_bool).unwrap_or(true),
                show_detail: object.get("showDetail").and_then(Value::as_bool).unwrap_or(true),
            },
            language: Language::from_tag(object.get("language").and_then(Value::as_str).unwrap_or("en")),
        })
    }
}

fn level(level: InterruptionLevel) -> &'static str {
    match level {
        InterruptionLevel::Passive => "passive",
        InterruptionLevel::Active => "active",
        InterruptionLevel::TimeSensitive => "timeSensitive",
    }
}

fn card(card: &Card) -> Value {
    json!({
        "title": card.title,
        "subtitle": card.subtitle,
        "body": card.body,
        "sound": card.sound,
        "level": level(card.level),
        "userInfo": {
            CONNECTION_KEY: card.connection_id,
            PANE_KEY: card.pane_id,
            AGENT_KEY: card.agent,
            STATE_KEY: card.state,
            DETAIL_KEY: card.detail,
        },
    })
}

/// One push, `request` and the answer as JSON (the module's table). A
/// request that can't be read is shown as received, as a push that isn't
/// ours is: never a notification lost.
pub fn receive_json(request: &str) -> String {
    // A line for the ring alone (`{"note", "diagnostics"}`): what the
    // extension says when iOS stops it.
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(request)
        && let (Some(note), Some(path)) = (
            object.get("note").and_then(Value::as_str),
            object.get("diagnostics").and_then(Value::as_str),
        )
    {
        DiagnosticsFile::new(path).record(note, Date::now());
        return json!({"delivery": "asReceived", "soundAllowed": true, "diagnostics": [note]}).to_string();
    }
    let Some(request) = Request::parse(request) else {
        return json!({"delivery": "asReceived", "soundAllowed": true, "diagnostics": []}).to_string();
    };
    let payload = json::parse(request.user_info.to_string().as_bytes(), Dialect::Decoder)
        .unwrap_or(json::Value::Null);
    let now = Date::from_unix(request.now);
    let received = receive(
        &payload,
        &PushPairingFile::new(&request.pairings).read(),
        &PushStandingFile::new(&request.standing),
        &PushFinishesFile::new(&request.finishes),
        request.prefs,
        request.language,
        now,
    );
    // Where the app can read them: this process's own log is invisible to
    // the app's diagnostics screen.
    let ring = DiagnosticsFile::new(&request.diagnostics);
    for line in &received.diagnostics {
        ring.record(line, now);
    }
    let mut answer = json!({
        "soundAllowed": received.sound_allowed,
        "diagnostics": received.diagnostics,
    });
    match &received.delivery {
        Delivery::AsReceived => answer["delivery"] = json!("asReceived"),
        Delivery::Fallback { secrets_tried } => {
            answer["delivery"] = json!("fallback");
            answer["secretsTried"] = json!(secrets_tried);
        }
        Delivery::Card(shown) => {
            answer["delivery"] = json!("card");
            answer["card"] = card(shown);
        }
    }
    match &received.waiting {
        Some(Waiting::Redraw { id, card: redrawn }) => {
            answer["waiting"] = json!({"redraw": {"id": id, "card": card(redrawn)}});
        }
        Some(Waiting::Withdraw { id }) => answer["waiting"] = json!({"withdraw": {"id": id}}),
        None => {}
    }
    answer.to_string()
}

/// The C entry point: [`receive_json`] over a NUL-terminated UTF-8 request.
/// The answer is the caller's, freed with [`moshpit_push_free`]; null only
/// when the request isn't a C string.
///
/// # Safety
///
/// `request` is null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moshpit_push_receive(request: *const c_char) -> *mut c_char {
    if request.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller hands a NUL-terminated string.
    let Ok(request) = unsafe { CStr::from_ptr(request) }.to_str() else {
        return std::ptr::null_mut();
    };
    CString::new(receive_json(request)).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Frees an answer [`moshpit_push_receive`] returned.
///
/// # Safety
///
/// `answer` is null or a pointer `moshpit_push_receive` returned, freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn moshpit_push_free(answer: *mut c_char) {
    if !answer.is_null() {
        // SAFETY: it came from `CString::into_raw` above.
        drop(unsafe { CString::from_raw(answer) });
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use moshpit_seal::{PushPairing, Status, Uuid, seal};

    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

    /// An App Group as the extension lays it out: one pairing at the root,
    /// the device-only folder beside it.
    struct Group(PathBuf);

    impl Group {
        fn new(name: &str) -> Group {
            let dir = std::env::temp_dir().join(format!("seal-ffi-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("device-only")).unwrap();
            PushPairingFile::new(dir.join(PushPairingFile::FILE_NAME)).write(&[PushPairing {
                connection_id: Uuid::parse(CONN).unwrap(),
                host_label: "Mac".into(),
                secret_hex: SECRET.into(),
                send_token: "0".repeat(64),
                relay_url: "https://push.example.org".into(),
                created_at: Date::now(),
                apns_token: None,
                apns_env: None,
                send_token_issued_at: None,
            }]);
            Group(dir)
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).display().to_string()
        }

        fn push(&self, pane: &str, state: &str, ts: i64, sound: bool) -> Value {
            let status = Status {
                conn: CONN.into(),
                host: "m1-pro".into(),
                sess: Some("work".into()),
                pane: pane.into(),
                agent: Some("claude".into()),
                state: state.into(),
                title: Some("Bash: npm test".into()),
                ts,
                dur: (state == "done").then_some(240),
            };
            let sealed = seal(status.to_json().as_bytes(), SECRET, None).unwrap();
            let request = json!({
                "userInfo": {"aps": {"mutable-content": 1},
                             "mp": {"v": sealed.v, "iv": sealed.iv, "ct": sealed.ct, "mac": sealed.mac}},
                "pairings": self.path(PushPairingFile::FILE_NAME),
                "diagnostics": self.path("push-diagnostics.json"),
                "standing": self.path("device-only/standing-attention.json"),
                "finishes": self.path("device-only/rung-finishes.json"),
                "now": Date::now().unix(),
                "sound": sound,
                "showDetail": true,
                "language": "en",
            });
            serde_json::from_str(&receive_json(&request.to_string())).unwrap()
        }

        fn has(&self, name: &str) -> bool {
            Path::new(&self.path(name)).exists()
        }
    }

    impl Drop for Group {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// E7b: the extension's one call does the whole receive. A question:
    /// the card it shows (words, interruption, the keys a tap routes by),
    /// its standing entry in the device-only folder (B785) and the
    /// diagnostic lines in the ring at the root; its finish: the waiting
    /// card withdrawn (B669), the closed turn kept beside the standing set
    /// (B636).
    #[test]
    fn one_call_opens_the_push_keeps_the_files_and_says_what_to_show() {
        let group = Group::new("whole");
        let asked = group.push("%3", "attention", 1_790_000_500, true);
        assert_eq!(asked["delivery"], "card");
        assert_eq!(asked["card"]["title"], "claude needs you");
        assert_eq!(asked["card"]["subtitle"], "Mac · work");
        assert_eq!(asked["card"]["body"], "Bash: npm test");
        assert_eq!((asked["card"]["sound"].as_bool(), asked["card"]["level"].as_str()), (Some(true), Some("timeSensitive")));
        assert_eq!(asked["card"]["userInfo"]["connectionId"], CONN);
        assert_eq!(asked["card"]["userInfo"]["paneId"], "%3");
        assert!(asked.get("waiting").is_none());
        assert!(group.has("device-only/standing-attention.json"));
        assert!(!group.has("standing-attention.json"), "never at the root (B785)");
        let ring = std::fs::read_to_string(group.path("push-diagnostics.json")).unwrap();
        assert!(ring.contains("opened a attention push from m1-pro"), "{ring}");

        let finished = group.push("%3", "done", 1_790_000_600, true);
        assert_eq!(finished["card"]["title"], "✓ claude finished · 4 min");
        assert_eq!(finished["waiting"]["withdraw"]["id"], format!("moshpit.attention.{CONN}"));
        assert!(group.has("device-only/closed-turns.json"), "beside the standing set");
    }

    /// Every path keeps the user's sound switch; a push that isn't ours, or
    /// a request that can't be read, is shown as it came; a note reaches the
    /// ring alone.
    #[test]
    fn the_switch_holds_and_what_isnt_ours_passes_through() {
        let group = Group::new("switch");
        let quiet = group.push("%3", "attention", 1_790_000_500, false);
        assert_eq!((quiet["soundAllowed"].as_bool(), quiet["card"]["sound"].as_bool()), (Some(false), Some(false)));
        let plain: Value = serde_json::from_str(&receive_json(
            &json!({"userInfo": {"aps": {}}, "pairings": "/x", "diagnostics": group.path("d.json"),
                    "standing": "/x", "finishes": "/x", "now": 1.0})
            .to_string(),
        ))
        .unwrap();
        assert_eq!(plain["delivery"], "asReceived");
        let garbled: Value = serde_json::from_str(&receive_json("not json")).unwrap();
        assert_eq!(garbled["delivery"], "asReceived");
        let noted: Value = serde_json::from_str(&receive_json(
            &json!({"note": "ran out of time — delivered whatever was ready",
                    "diagnostics": group.path("push-diagnostics.json")})
            .to_string(),
        ))
        .unwrap();
        assert_eq!(noted["delivery"], "asReceived");
        let ring = std::fs::read_to_string(group.path("push-diagnostics.json")).unwrap();
        assert!(ring.contains("ran out of time"), "{ring}");
    }

    /// The C entry point round-trips, and frees what it returns.
    #[test]
    fn the_c_entry_point_answers_and_frees() {
        let request = CString::new(json!({"note": "x", "diagnostics": "/nonexistent/d.json"}).to_string()).unwrap();
        // SAFETY: a valid C string in, the answer freed once.
        unsafe {
            let answer = moshpit_push_receive(request.as_ptr());
            assert!(!answer.is_null());
            assert!(CStr::from_ptr(answer).to_str().unwrap().contains("asReceived"));
            moshpit_push_free(answer);
            assert!(moshpit_push_receive(std::ptr::null()).is_null());
            moshpit_push_free(std::ptr::null_mut());
        }
    }
}
