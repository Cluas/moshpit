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
        json::decode(plaintext).ok().or_else(|| {
            std::str::from_utf8(plaintext)
                .is_err()
                .then(|| json::decode(repaired(plaintext).as_bytes()).ok())
                .flatten()
        })
    }

    /// Compact JSON as `JSONEncoder` writes it, keys in code point order
    /// (Swift's own order is a hash order that changes from run to run).
    pub fn to_json(&self) -> String {
        json::encode(self)
    }
}

/// `bytes` as text (B635): a sequence a cut left short just before a
/// string's closing quote — the end of a title `cut` ended mid-character —
/// is dropped; any other byte that isn't UTF-8 reads as U+FFFD.
fn repaired(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                return text;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                let bad = error.error_len().unwrap_or(after.len());
                let at_a_string_end = after.get(bad) == Some(&b'"');
                if !(at_a_string_end && cut_short(&after[..bad])) {
                    text.push('\u{FFFD}');
                }
                rest = &after[bad..];
            }
        }
    }
}

/// Whether `sequence` is the start of a character with bytes missing: a
/// lead byte and fewer continuation bytes than it calls for.
fn cut_short(sequence: &[u8]) -> bool {
    let wanted = match sequence.first() {
        Some(0xC2..=0xDF) => 2,
        Some(0xE0..=0xEF) => 3,
        Some(0xF0..=0xF4) => 4,
        _ => return false,
    };
    sequence.len() < wanted && sequence[1..].iter().all(|byte| byte & 0xC0 == 0x80)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(title: &[u8]) -> Vec<u8> {
        let mut plaintext = br#"{"conn":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","host":"m1-pro","pane":"%3","state":"done","ts":1755900000,"title":"Bash: "#.to_vec();
        plaintext.extend_from_slice(title);
        plaintext.extend_from_slice(br#""}"#);
        plaintext
    }

    /// B635: a character a cut left short at the very end of the title is
    /// dropped — `cut -c` without `iconv` — not shown as a trailing "\u{FFFD}".
    #[test]
    fn a_character_cut_short_at_the_end_of_a_title_is_dropped() {
        for cut in [
            &"构建".as_bytes()[..4],
            &"构建".as_bytes()[..5],
            &"é".as_bytes()[..1],
        ] {
            let status = Status::from_json(&titled(cut)).unwrap();
            let title = status.title.unwrap();
            assert!(!title.contains('\u{FFFD}'), "{title:?}");
            assert!(title == "Bash: 构" || title == "Bash: ", "{title:?}");
        }
    }

    /// B635: any other byte that isn't UTF-8 reads as U+FFFD, the status
    /// still read.
    #[test]
    fn other_bytes_that_arent_utf8_read_as_the_replacement_character() {
        for (bytes, shown) in [
            (b"a\xFFb".as_slice(), "Bash: a\u{FFFD}b"),
            (b"\xE6\x9E ok".as_slice(), "Bash: \u{FFFD} ok"),
            (b"\x80".as_slice(), "Bash: \u{FFFD}"),
        ] {
            let status = Status::from_json(&titled(bytes)).unwrap();
            assert_eq!(status.title.as_deref(), Some(shown), "{bytes:?}");
        }
    }

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
