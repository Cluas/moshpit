//! What an agent's hook says — the plaintext inside an envelope.

use serde::{Deserialize, Serialize};

use crate::foundation::json;

/// One agent status (`PushSealedBox.Status`, Go's `sealbox.Status`).
///
/// `conn` is the phone's own connection id, handed to the host at pairing
/// and echoed back, so a notification arrives knowing which saved connection
/// it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub conn: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sess: Option<String>,
    pub pane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub ts: i64,
    /// Seconds the episode this status closes ran — for a `done`, the turn's
    /// length; decides whether the finish is worth a sound. Older senders
    /// omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dur: Option<i64>,
}

impl Status {
    /// The plaintext as `JSONDecoder().decode(Status.self, from:)` reads it:
    /// a JSON object (byte order mark, a trailing comma and repeated keys
    /// taken as Foundation takes them — the first of two wins), the five
    /// string fields and `ts` required, `sess`/`agent`/`title`/`dur` optional,
    /// `null` the same as absent, unknown keys ignored, an integral `1.0` or
    /// `1e3` read as an `Int`. `None` for anything else.
    ///
    /// B635: bytes that aren't UTF-8 are read as U+FFFD rather than refuse
    /// the whole status. A host cuts a title to 120 characters with `cut`,
    /// which cuts bytes (GNU's `-c` is `-b`), and drops the half character
    /// with `iconv -c` — where there is one: on a host without it (BusyBox,
    /// Alpine) every title of more than 40 Chinese characters split one,
    /// and the push read as one no key opened.
    pub fn from_json(plaintext: &[u8]) -> Option<Self> {
        json::decode(plaintext)
            .ok()
            .or_else(|| match String::from_utf8_lossy(plaintext) {
                std::borrow::Cow::Owned(text) => json::decode(text.as_bytes()).ok(),
                std::borrow::Cow::Borrowed(_) => None,
            })
    }

    /// Compact JSON as `JSONEncoder` writes it, keys in code point order
    /// (Swift's own order is a hash order that changes from run to run).
    pub fn to_json(&self) -> String {
        json::encode(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(text: &str) -> Option<Status> {
        Status::from_json(text.as_bytes())
    }

    const BASE: &str = r#""host":"h","pane":"%1","state":"done""#;

    /// What `JSONDecoder` did with each on macOS 27.
    #[test]
    fn decodes_as_jsondecoder_does() {
        let ok = |rest: &str| decode(&format!("{{{rest}}}"));
        assert_eq!(
            ok(&format!(r#""conn":"a",{BASE},"ts":1"#)).unwrap().conn,
            "a"
        );
        assert_eq!(
            ok(&format!(r#""conn":"a","conn":"b",{BASE},"ts":1"#))
                .unwrap()
                .conn,
            "a"
        );
        assert_eq!(
            ok(&format!(r#""conn":"a",{BASE},"ts":1,"dur":null"#))
                .unwrap()
                .dur,
            None
        );
        assert_eq!(
            ok(&format!(r#""conn":"a",{BASE},"ts":1,"dur":2.0"#))
                .unwrap()
                .dur,
            Some(2)
        );
        assert_eq!(
            ok(&format!(r#""conn":"a",{BASE},"ts":1,"dur":-5"#))
                .unwrap()
                .dur,
            Some(-5)
        );
        assert_eq!(
            ok(&format!(r#""conn":"a",{BASE},"ts":1,"sess":null"#))
                .unwrap()
                .sess,
            None
        );
        assert!(ok(&format!(r#""conn":"a",{BASE},"ts":1,"sess":1"#)).is_none());
        assert!(ok(&format!(r#""conn":null,{BASE},"ts":1"#)).is_none());
        assert!(ok(&format!(r#""conn":"a",{BASE},"ts":1,"#)).is_some());
        assert_eq!(
            ok(&format!(r#""conn":"a\u0000b",{BASE},"ts":1"#))
                .unwrap()
                .conn,
            "a\u{0}b"
        );
        assert_eq!(
            ok(&format!(r#""conn":"😀",{BASE},"ts":1"#)).unwrap().conn,
            "😀"
        );
        assert!(ok(&format!(r#""conn":"\ud83d",{BASE},"ts":1"#)).is_none());
        assert!(ok(&format!(r#""conn":"a",{BASE},"ts":1,"x":[1,{{"y":nan}}]"#)).is_none());
        assert!(ok(&format!(r#""conn":"a",{BASE},"ts":1,"x":01"#)).is_some());
        assert!(ok(&format!(r#""Conn":"a",{BASE},"ts":1"#)).is_none());
        assert!(ok(&format!("\"conn\":\"\u{1}\",{BASE},\"ts\":1")).is_none());
        assert!(decode(&format!(r#"[{{"conn":"a",{BASE},"ts":1}}]"#)).is_none());
        assert!(decode(&format!("\u{FEFF}{{\"conn\":\"a\",{BASE},\"ts\":1}}")).is_some());
        assert!(decode("not json").is_none());
    }

    #[test]
    fn writes_what_it_reads() {
        let status = Status {
            conn: "C".into(),
            host: "h/1".into(),
            sess: None,
            pane: "%3".into(),
            agent: Some("claude".into()),
            state: "done".into(),
            title: Some("a \"b\"".into()),
            ts: 7,
            dur: Some(12),
        };
        assert_eq!(
            status.to_json(),
            r#"{"agent":"claude","conn":"C","dur":12,"host":"h\/1","pane":"%3","state":"done","title":"a \"b\"","ts":7}"#
        );
        assert_eq!(decode(&status.to_json()), Some(status));
    }
}
