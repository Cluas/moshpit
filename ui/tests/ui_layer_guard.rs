//! Pages are built from the app's `ui` layer (`docs/nsh/UI-DESIGN.md` §4,
//! R17): they take no control from `nsh_base` directly, the alert
//! included (`ui::alert`). The scan reads the code (not the comments) of
//! every file under `ui/src` but the layer itself (`ui/`) and the theme
//! (`theme.rs`, which builds the NSH theme from `nsh_base::Theme`).

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`.
fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

/// `line` without its comment: what follows a `//` outside a string.
fn code(line: &str) -> &str {
    let bytes = line.as_bytes();
    let (mut quoted, mut escaped) = (false, false);
    for (i, &b) in bytes.iter().enumerate() {
        if quoted {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => quoted = false,
                _ => {}
            }
        } else if b == b'"' {
            quoted = true;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return &line[..i];
        }
    }
    line
}

/// The lines of `source` whose code names `nsh_base`, numbered from 1.
fn uses(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| code(line).contains("nsh_base"))
        .map(|(n, line)| (n + 1, line.trim().to_owned()))
        .collect()
}

#[test]
fn no_page_takes_a_control_from_nsh_base() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    assert!(files.len() > 30, "the walk finds the pages");
    let mut report = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&src).unwrap();
        if rel.starts_with("ui") || rel == Path::new("theme.rs") {
            continue;
        }
        let rel = rel.display();
        for (n, line) in uses(&std::fs::read_to_string(&file).unwrap()) {
            report.push(format!(
                "{rel}:{n}: {line} (take it from the ui layer: crate::ui::…)"
            ));
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}

/// What the scan reads as using `nsh_base`, and what it does not.
#[test]
fn the_scan_reads_code_not_comments() {
    for used in [
        "use nsh_base::alert_dialog;",
        "    nsh_base::button(\"Save\")",
        "let url = \"https://example.com\"; nsh_base::input()",
        "let s = \"a \\\" quote\"; nsh_base::switch()",
    ] {
        assert_eq!(uses(used).len(), 1, "{used}");
    }
    for fine in [
        "// nsh_base::button()",
        "/// `nsh_base`'s button says only selected",
        "//! not from nsh_base directly",
        "let x = 1; // nsh_base::input()",
        "use crate::ui::alert::alert_dialog;",
    ] {
        assert!(uses(fine).is_empty(), "{fine}");
    }
}
