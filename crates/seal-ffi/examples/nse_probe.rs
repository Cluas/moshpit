//! E7b's memory proxy (not a test): what the notification service does for
//! pushes, in a process of its own, so its peak resident size can be read
//! (`/usr/bin/time -l`) against an empty one. `nse_probe N` sends N pushes
//! (a question, then its finish, alternating panes) through
//! [`moshpit_seal_ffi::receive_json`] over a fresh App Group directory with
//! one pairing; `nse_probe 0` does nothing, the baseline.

use moshpit_seal::foundation::json;
use moshpit_seal::{Date, PushPairing, PushPairingFile, Status, Uuid, seal};

const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const CONN: &str = "3F2504E0-4F89-11D3-9A0C-0305E82C3301";

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1);
    if n == 0 {
        return;
    }
    let dir = std::env::temp_dir().join(format!("nse-probe-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("device-only")).unwrap();
    let pairings = PushPairingFile::new(dir.join(PushPairingFile::FILE_NAME));
    pairings.write(&[PushPairing {
        connection_id: Uuid::parse(CONN).unwrap(),
        host_label: "m1-pro".into(),
        secret_hex: SECRET.into(),
        send_token: "0".repeat(64),
        relay_url: "https://push.example.org".into(),
        created_at: Date::now(),
        apns_token: None,
        apns_env: None,
        send_token_issued_at: None,
    }]);
    let now = Date::now().unix();
    for i in 0..n {
        let state = if i % 2 == 0 { "attention" } else { "done" };
        let status = Status {
            conn: CONN.into(),
            host: "m1-pro".into(),
            sess: Some("work".into()),
            pane: format!("%{}", i / 2 % 8),
            agent: Some("claude".into()),
            state: state.into(),
            title: Some("Bash: npm test -- --watch=false".into()),
            ts: now as i64 + i as i64,
            dur: (state == "done").then_some(240),
        };
        let plain = json::encode(&status);
        let envelope = seal(plain.as_bytes(), SECRET, None).unwrap();
        let request = serde_json::json!({
            "userInfo": {"mp": {"v": envelope.v, "iv": envelope.iv, "ct": envelope.ct, "mac": envelope.mac}},
            "pairings": dir.join(PushPairingFile::FILE_NAME).display().to_string(),
            "diagnostics": dir.join("push-diagnostics.json").display().to_string(),
            "standing": dir.join("device-only/standing-attention.json").display().to_string(),
            "finishes": dir.join("device-only/rung-finishes.json").display().to_string(),
            "now": now + i as f64,
            "sound": true,
            "showDetail": true,
            "language": "zh-Hans",
        });
        let answer = moshpit_seal_ffi::receive_json(&request.to_string());
        if i == 0 {
            eprintln!("{answer}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
