//! The Rust transport against the Swift app's recorded answers, without
//! swiftc: `tests/fixtures/swift/cases.swift.txt` is what
//! `harness/swift/main.swift` printed for `cases.txt` over the app's own
//! SSHKeyFactory, HostKeyValidator.fingerprint(of:) and ExecUploadCommands,
//! Citadel's key detection and the swift-nio-ssh the app pins
//! (`harness/equivalence.py update-goldens`). Rust must answer the same —
//! and every fingerprint must also be `ssh-keygen -l`'s — except on the
//! documented divergences (`harness/divergences.json`), where it must answer
//! what `divergences.rust.txt` holds and never what Swift did.
//!
//! The live key exchanges (NIOSSH and russh against real sshds) need the
//! machine: `harness/equivalence.py run`.

use std::collections::BTreeMap;
use std::path::Path;

use moshpit_ssh::harness;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/swift")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn records(name: &str) -> Vec<Vec<String>> {
    fixture(name)
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

fn json_string(text: &str) -> String {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

struct Case {
    id: String,
    command: String,
    args: Vec<String>,
    keygen: Option<String>,
}

fn cases() -> Vec<Case> {
    records("cases.txt")
        .into_iter()
        .map(|fields| Case {
            id: fields[0].clone(),
            command: fields[1].clone(),
            args: serde_json::from_str(&fields[2]).unwrap(),
            keygen: (fields[3] != "-").then(|| fields[3].clone()),
        })
        .collect()
}

fn rust_answer(case: &Case) -> String {
    let args: Vec<String> = match case.command.as_str() {
        "keytype" | "import" => {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join(&case.args[0]);
            vec![std::fs::read_to_string(path).unwrap()]
        }
        _ => case.args.clone(),
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    harness::answer(&case.command, &args).unwrap_or_else(|| panic!("{}: no answer", case.id))
}

#[test]
fn rust_answers_what_the_app_answered() {
    let swift: BTreeMap<String, String> = records("cases.swift.txt")
        .into_iter()
        .map(|f| (f[0].clone(), json_string(&f[1])))
        .collect();
    let divergent: BTreeMap<String, (String, String)> = records("divergences.rust.txt")
        .into_iter()
        .map(|f| (f[0].clone(), (f[1].clone(), json_string(&f[2]))))
        .collect();
    let cases = cases();
    assert!(cases.len() >= 70, "{} cases", cases.len());
    let mut equal = 0;
    for case in &cases {
        let rust = rust_answer(case);
        let recorded = &swift[&case.id];
        match divergent.get(&case.id) {
            Some((rule, expected)) => {
                assert_eq!(&rust, expected, "{} ({rule})", case.id);
                assert_ne!(&rust, recorded, "{} ({rule}) no longer diverges", case.id);
            }
            None => {
                assert_eq!(&rust, recorded, "{}", case.id);
                equal += 1;
            }
        }
        if let Some(keygen) = &case.keygen {
            let kind = case.args[0].split(' ').next().unwrap();
            assert_eq!(
                rust,
                format!("{kind} {keygen}"),
                "{} vs ssh-keygen",
                case.id
            );
        }
    }
    assert_eq!(equal + divergent.len(), cases.len());
    // 59 of 78 since the app refuses P-384/P-521 and a key asked without
    // its passphrase at import, and reads PKCS#8 (harness/divergences.json).
    assert!(equal >= 55, "{equal} equal answers");
}
