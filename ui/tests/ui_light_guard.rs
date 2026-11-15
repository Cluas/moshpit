//! R18's guard (UI-DESIGN §10.6, `docs/nsh/R18-AUDIT.md`): a screen follows
//! the system's appearance (`Screen::follows_system`) only once nothing it
//! draws with is of the old dark palette, so no page ships half light.
//!
//! What a screen draws with:
//!
//! - its own files ([`draws_with`]), whole;
//! - the shared parts every page draws through ([`SHARED`]: the `ui`
//!   layer, the navigation bar, the large title's bar, the type faces),
//!   whole;
//! - what it uses of any other module (another page's helper), item by
//!   item: each item's body, and the module's own items that one calls in
//!   turn. A type's methods are not followed: a page that draws with
//!   another page's type lists that page's file as its own.
//!
//! The old dark palette is a colour literal (`rgb(0x…)`, `hex(0x…)`,
//! `Rgba { r: … }`) and the old helpers that make one (`white(…)`,
//! `faded(…)`): R17 deleted the modules of named dark colours, and one
//! made again would be literals in its own file, which the guard reads. A
//! colour the page shows for what it is (a swatch's white, words on a
//! picture) is listed in [`CONTENT`]. The tokens (`ui/tokens.rs`) and the
//! theme (`theme.rs`) are where colours are defined, so they are not read.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use moshpit_ui::screen::Screen;

/// A screen's own files under `ui/src`: read whole. Every screen is listed
/// (Settings was the last onto the light appearance): a new one is not
/// built until it says what it draws with.
fn draws_with(screen: Screen) -> &'static [&'static str] {
    match screen {
        Screen::UiGallery => &["ui/gallery.rs"],
        Screen::Home => &[
            "home/mod.rs",
            "home/card.rs",
            "home/rows.rs",
            "home/swipe.rs",
            "home/model.rs",
            "home/words.rs",
        ],
        Screen::ConnectionForm => &["connection_form.rs"],
        Screen::Theme => &["theme_gallery.rs"],
        // The editor draws the gallery's scheme previews.
        Screen::ThemeEditor => &["theme_editor.rs", "theme_gallery.rs"],
        Screen::Accent => &["accent.rs"],
        Screen::AccentEditor => &["accent_editor.rs"],
        Screen::AppIcon => &["app_icon.rs"],
        Screen::RemoteFolder => &["terminal/folder.rs", "terminal/prompt.rs"],
        // The picker opens the folder's sheet and asks the host's prompt.
        Screen::MuxPicker => &[
            "terminal/picker.rs",
            "terminal/folder.rs",
            "terminal/model.rs",
            "terminal/prompt.rs",
        ],
        Screen::SshKeys => &["ssh_keys.rs"],
        Screen::AddKey => &["add_key.rs"],
        Screen::KeyPicker => &["key_picker.rs"],
        Screen::Shortcuts => &["shortcuts.rs"],
        Screen::AddShortcut => &["add_shortcut.rs"],
        // The terminal's chrome (R12): the bar, the covers and banners, the
        // shortcut bar, the panels, mosh's counters. Its appearance is its
        // terminal theme's, not the system's (`Screen::appearance`).
        Screen::Terminal => &[
            "terminal/mod.rs",
            "terminal/bar.rs",
            "terminal/chrome.rs",
            "terminal/cover.rs",
            "terminal/keys.rs",
            "terminal/panels.rs",
            "terminal/diagnostics.rs",
            "terminal/model.rs",
            "terminal/shortcuts.rs",
        ],
        Screen::InstallAssist => &["terminal/install.rs"],
        Screen::Sessions => &["sessions/mod.rs", "sessions/model.rs"],
        // R15: the voice pages; the language page names the engine's.
        Screen::VoiceEngine => &["voice_engine.rs"],
        Screen::VoiceLanguage => &["voice_language.rs", "voice_engine.rs"],
        Screen::WhisperModel => &["whisper_model.rs"],
        // R16: feedback and the recent log.
        Screen::Feedback => &["feedback.rs"],
        Screen::RecentLog => &["recent_log.rs"],
        // R16: Set up a host.
        Screen::HostSetup => &["host_setup.rs"],
        // Settings: the hub and its five pages, one file.
        Screen::Settings => &["settings.rs"],
        // E6c: the sheets that were SwiftUI's.
        Screen::HostKey => &["host_key.rs"],
        Screen::AgentTask => &["agent_task.rs"],
        Screen::ThemeImport => &["theme_import.rs"],
    }
}

/// What every page draws through: read whole for each screen.
const SHARED: &[&str] = &["nav_bar.rs", "face.rs", "home/bar.rs"];

/// The `ui` layer's modules that are not shared drawing: the tokens (where
/// the colours are defined) and the gallery (a screen of its own).
const UI_NOT_SHARED: &[&str] = &["ui::tokens", "ui::gallery"];

/// Modules that draw nothing, or define the colours: never read.
const INERT: &[&str] = &[
    "lib",
    "theme",
    "ui::tokens",
    "screen",
    "strings",
    "channel",
    "os",
];

/// Colours a page shows for what they are, by file and the words on their
/// line: (file, line, why).
const CONTENT: &[(&str, &str, &str)] = &[
    (
        "ui/swatch.rs",
        "pub const WHITE: Rgba",
        "a white swatch's colour",
    ),
    (
        "terminal/folder.rs",
        "const ON_MEDIA: Rgba",
        "words on a picture's dark scrim, light in both appearances",
    ),
    (
        "app_icon.rs",
        "color: nsh_core::Color::from(nsh_core::color::Rgba {",
        "an icon's drop shadow, black in both appearances as the system's",
    ),
];

// --- Reading the source ------------------------------------------------------

/// `ui/src`.
fn src() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every source file's module (`home::bar`) and path under `ui/src`.
fn modules() -> BTreeMap<String, String> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let stem = rel.trim_end_matches(".rs");
                let module = stem.strip_suffix("/mod").unwrap_or(stem).replace('/', "::");
                out.insert(module, rel);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&src(), &src(), &mut out);
    out
}

/// The module of `rel`, a path under `ui/src`.
fn module_of(rel: &str) -> String {
    let stem = rel.trim_end_matches(".rs");
    stem.strip_suffix("/mod").unwrap_or(stem).replace('/', "::")
}

/// `line` without its `//` comment (one outside a string).
fn uncommented(line: &str) -> &str {
    let mut quoted = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'\\' if quoted => i += 1,
            b'"' => quoted = !quoted,
            b'/' if !quoted && bytes[i + 1] == b'/' => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// The code of `text` (numbered from 1), its comments out, up to its tests.
fn code(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .take_while(|(_, line)| !line.starts_with("#[cfg(test)]"))
        .map(|(n, line)| (n + 1, uncommented(line).to_owned()))
        .collect()
}

thread_local! {
    /// Files a test of the guard itself stands in for `ui/src`'s, by path:
    /// its own thread's only, so the guard proper never reads them.
    static FIXTURES: RefCell<BTreeMap<String, String>> = RefCell::default();
}

/// The code of `rel`, a file under `ui/src` (or a test's stand-in).
fn code_of(rel: &str) -> Vec<(usize, String)> {
    if let Some(text) = FIXTURES.with(|f| f.borrow().get(rel).cloned()) {
        return code(&text);
    }
    let text = std::fs::read_to_string(src().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    code(&text)
}

/// Whether `text[at..]` starts a word (the byte before it is not part of
/// an identifier).
fn word_starts(text: &str, at: usize) -> bool {
    at == 0 || {
        let c = text.as_bytes()[at - 1];
        !(c.is_ascii_alphanumeric() || c == b'_')
    }
}

/// The old dark palette's marks, each at the start of a word.
const MARKS: &[&str] = &["rgb(0x", "hex(0x", "white(", "faded("];

/// The lines of `lines` that use the old dark palette.
fn palette(lines: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (i, (n, line)) in lines.iter().enumerate() {
        let marked = MARKS.iter().any(|mark| {
            line.match_indices(mark)
                .any(|(at, _)| word_starts(line, at))
        });
        // `Rgba { r: …` (its fields on the next line or this one); not
        // `Rgba { a: 1.0, ..colour }`, a colour the page was given.
        let literal = line.match_indices("Rgba").any(|(at, _)| {
            let rest = line[at + 4..].trim_start();
            let Some(after) = rest.strip_prefix('{') else {
                return false;
            };
            let after = after.trim_start();
            let next = if after.is_empty() {
                lines.get(i + 1).map_or("", |(_, l)| l.trim_start())
            } else {
                after
            };
            next.starts_with("r:") || next.starts_with("r :")
        });
        if marked || literal {
            found.push((*n, line.trim().to_owned()));
        }
    }
    found
}

/// `tree`, the text of a `use` after `use`, as full paths
/// (`crate::{a, b::{c, d}}`: `crate::a`, `crate::b::c`, `crate::b::d`).
fn expand(tree: &str) -> Vec<String> {
    let tree: String = tree.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(open) = tree.find('{') else {
        return vec![tree];
    };
    let prefix = &tree[..open];
    let inner = &tree[open + 1..tree.rfind('}').unwrap_or(tree.len())];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&inner[start..]);
    parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .flat_map(expand)
        .map(|p| {
            if p == "self" {
                prefix.trim_end_matches(':').to_owned()
            } else {
                format!("{prefix}{p}")
            }
        })
        .collect()
}

/// The paths `lines` name the crate's modules by: its `use`s, and the
/// `crate::…` and `super::…` paths in its code.
fn paths(lines: &[(usize, String)]) -> Vec<String> {
    let text: String = lines
        .iter()
        .map(|(_, l)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    for (at, _) in text.match_indices("use ") {
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let before = text[line_start..at].trim();
        if !(before.is_empty() || before == "pub" || before.starts_with("pub(")) {
            continue;
        }
        if let Some(end) = text[at..].find(';') {
            found.extend(expand(&text[at + 4..at + end]));
        }
    }
    for head in ["crate::", "super::"] {
        for (at, _) in text.match_indices(head) {
            if !word_starts(&text, at) || text[..at].ends_with("::") {
                continue;
            }
            let path: String = text[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            found.push(path.trim_end_matches(':').to_owned());
        }
    }
    found
}

/// The module `path` names, written in module `here`, and what follows it
/// in the path (`crate::nav_bar::text_size` in `voice_engine`: `nav_bar`,
/// `["text_size"]`).
/// `None` for a path that is not the crate's.
fn resolve(
    path: &str,
    here: &str,
    modules: &BTreeMap<String, String>,
) -> Option<(String, Vec<String>)> {
    let segments: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    let here: Vec<&str> = if here == "lib" {
        Vec::new()
    } else {
        here.split("::").collect()
    };
    let (mut base, mut rest): (Vec<&str>, &[&str]) = match segments.first() {
        Some(&"crate") => (Vec::new(), &segments[1..]),
        Some(&"self") => (here.clone(), &segments[1..]),
        Some(&"super") => (
            here[..here.len().saturating_sub(1)].to_vec(),
            &segments[1..],
        ),
        _ => return None,
    };
    while rest.first() == Some(&"super") {
        base.pop();
        rest = &rest[1..];
    }
    for k in (0..=rest.len()).rev() {
        let mut parts = base.clone();
        parts.extend_from_slice(&rest[..k]);
        let module = if parts.is_empty() {
            "lib".to_owned()
        } else {
            parts.join("::")
        };
        if modules.contains_key(&module) {
            return Some((module, rest[k..].iter().map(|s| s.to_string()).collect()));
        }
    }
    None
}

/// The items of other modules `lines` (of module `here`) use, by module,
/// leaving out the modules `skip` says: through a path
/// (`crate::nav_bar::text_size`, `super::symbol`) or by a module's name
/// after importing it (`nav_bar::text_size`), but not a module of `here`'s own of that
/// name (`home`'s `card`).
fn borrowed(
    lines: &[(usize, String)],
    here: &str,
    modules: &BTreeMap<String, String>,
    skip: &dyn Fn(&str) -> bool,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut uses: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut whole = BTreeSet::new();
    for path in paths(lines) {
        let Some((module, rest)) = resolve(&path, here, modules) else {
            continue;
        };
        if skip(&module) {
            continue;
        }
        match rest.first() {
            Some(item) if item != "*" => {
                uses.entry(module).or_default().insert(item.clone());
            }
            _ => {
                whole.insert(module);
            }
        }
    }
    // A module imported by its name: its items as `name::item`.
    for module in whole {
        let leaf = module.rsplit("::").next().unwrap_or(&module).to_owned();
        let child = if here == "lib" {
            leaf.clone()
        } else {
            format!("{here}::{leaf}")
        };
        if modules.contains_key(&child) && child != module {
            continue;
        }
        let mark = format!("{leaf}::");
        for (_, line) in lines {
            for (at, _) in line.match_indices(&mark) {
                if !word_starts(line, at) || line[..at].ends_with("::") {
                    continue;
                }
                let item: String = line[at + mark.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !item.is_empty() && item != "self" {
                    uses.entry(module.clone()).or_default().insert(item);
                }
            }
        }
    }
    uses
}

/// Where item `name` starts in `text`: its `fn`, `const`, `static`,
/// `struct`, `enum` or `type`.
fn definition(text: &str, name: &str) -> Option<usize> {
    for keyword in ["fn ", "const ", "static ", "struct ", "enum ", "type "] {
        let mark = format!("{keyword}{name}");
        for (at, _) in text.match_indices(&mark) {
            let next = text[at + mark.len()..].chars().next();
            if word_starts(text, at) && !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Some(at);
            }
        }
    }
    None
}

/// The text of the item that starts at `at`: up to its block's end, or
/// its `;`.
fn item_text(text: &str, at: usize) -> &str {
    let mut depth = 0;
    for (i, c) in text[at..].char_indices() {
        match c {
            ';' if depth == 0 => return &text[at..at + i + 1],
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[at..at + i + 1];
                }
            }
            _ => {}
        }
    }
    &text[at..]
}

/// The old dark palette in items `names` of module `rel`, and in the
/// module's own items they use in turn: (line, item, code).
fn item_palette(rel: &str, names: &BTreeSet<String>) -> Vec<(usize, String, String)> {
    let lines = code_of(rel);
    let text: String = lines
        .iter()
        .map(|(_, l)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let line_at = |offset: usize| text[..offset].matches('\n').count();
    let mut todo: Vec<String> = names.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    while let Some(name) = todo.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(at) = definition(&text, &name) else {
            continue;
        };
        let item = item_text(&text, at);
        let first = line_at(at);
        let count = item.matches('\n').count() + 1;
        for (n, line) in palette(&lines[first..(first + count).min(lines.len())]) {
            found.push((n, name.clone(), line));
        }
        // The module's own items it uses: what it calls, and its
        // constants (a local named as a function is not a use of it).
        for word in uses_in(item) {
            if !seen.contains(&word) && definition(&text, &word).is_some() {
                todo.push(word);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// The names `item` calls (`row(`, `row::<`) or names as constants
/// (`ROW_SPACING`).
fn uses_in(item: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut chars = item.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !(c.is_ascii_alphabetic() || c == '_') || !word_starts(item, start) {
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(i, c)) = chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end = i + c.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        let word = &item[start..end];
        let after = item[end..].trim_start();
        let constant = word.len() > 1
            && word
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if after.starts_with('(') || after.starts_with("::<") || constant {
            found.push(word.to_owned());
        }
    }
    found
}

/// Whether `line` of `rel` is one of [`CONTENT`]'s colours.
fn content(rel: &str, line: &str) -> bool {
    CONTENT
        .iter()
        .any(|(file, words, _)| *file == rel && line.contains(words))
}

/// What keeps a screen whose own files are `own` from the light
/// appearance: one line each.
fn problems(own: &[&str], modules: &BTreeMap<String, String>) -> Vec<String> {
    let shared: Vec<String> = SHARED
        .iter()
        .map(|s| s.to_string())
        .chain(
            modules
                .iter()
                .filter(|(m, _)| m.starts_with("ui::") && !UI_NOT_SHARED.contains(&m.as_str()))
                .map(|(_, rel)| rel.clone()),
        )
        .collect();
    let read_whole: BTreeSet<String> = own
        .iter()
        .map(|r| r.to_string())
        .chain(shared.iter().cloned())
        .map(|r| module_of(&r))
        .collect();
    let skip =
        |module: &str| read_whole.contains(module) || INERT.contains(&module) || module == "ui";
    let mut found = Vec::new();
    let mut items: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rel in own
        .iter()
        .map(|s| s.to_string())
        .chain(shared.iter().cloned())
    {
        if !modules.values().any(|r| *r == rel) {
            found.push(format!("{rel}: no such file under ui/src"));
            continue;
        }
        let lines = code_of(&rel);
        for (n, line) in palette(&lines) {
            if !content(&rel, &line) {
                found.push(format!("{rel}:{n}: {line}"));
            }
        }
        for (module, used) in borrowed(&lines, &module_of(&rel), modules, &skip) {
            items.entry(module).or_default().extend(used);
        }
    }
    for (module, used) in &items {
        let rel = &modules[module];
        for (n, item, line) in item_palette(rel, used) {
            if !content(rel, &line) {
                found.push(format!("{rel}:{n} ({module}::{item}): {line}"));
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

// --- The guard -----------------------------------------------------------------

/// Every screen that follows the system draws with nothing of the old
/// dark palette.
#[test]
fn a_screen_follows_the_system_only_with_nothing_dark_only() {
    let modules = modules();
    let mut report = Vec::new();
    for screen in Screen::ALL.into_iter().filter(|s| s.follows_system()) {
        for problem in problems(draws_with(screen), &modules) {
            report.push(format!("{}: {problem}", screen.name()));
        }
    }
    assert!(
        report.is_empty(),
        "{}\n\nKeep the screen off Screen::follows_system until its page is \
         on the tokens (docs/nsh/R18-AUDIT.md).",
        report.join("\n")
    );
}

/// The old dark palette is nowhere: every file under `ui/src` (but the
/// tokens and the theme, where colours are defined) draws from the tokens.
/// R17 emptied the list of files still on it (`docs/nsh/R18-AUDIT.md` §2).
#[test]
fn the_old_palette_is_nowhere() {
    let mut report = Vec::new();
    for (module, rel) in modules() {
        if module == "theme" || module == "ui::tokens" {
            continue;
        }
        let dark = palette(&code_of(&rel))
            .into_iter()
            .find(|(_, line)| !content(&rel, line));
        if let Some((n, line)) = dark {
            report.push(format!("{rel}:{n}: {line} (draw it from a token)"));
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}

/// The guard holds back a screen drawing with the old palette, its own or
/// a helper's it borrows, and passes one on the tokens. No file is on the
/// palette since R17, so these pages are stand-ins.
#[test]
fn the_guard_knows_a_page_still_dark() {
    let mut modules = modules();
    let pages = [
        (
            "dark_page",
            "pub fn page() -> View {\n    view().bg(hex(0x11141A, 0.94))\n}\n",
        ),
        (
            "dark_helper",
            "pub fn tint() -> Rgba {\n    white(0.08)\n}\n",
        ),
        (
            "borrowing_page",
            "use crate::dark_helper::tint;\n\npub fn page() -> View {\n    view().bg(tint())\n}\n",
        ),
        (
            "token_page",
            "pub fn page() -> View {\n    view().bg(move || token(\"card\"))\n}\n",
        ),
    ];
    FIXTURES.with(|f| {
        let mut f = f.borrow_mut();
        for (module, text) in pages {
            f.insert(format!("{module}.rs"), text.to_owned());
        }
    });
    for (module, _) in pages {
        modules.insert(module.to_owned(), format!("{module}.rs"));
    }
    for (own, why) in [
        ("dark_page.rs", "its own colour literal"),
        ("borrowing_page.rs", "a helper's white"),
    ] {
        assert!(
            !problems(&[own], &modules).is_empty(),
            "{own} ({why}) passes the guard"
        );
    }
    assert_eq!(
        problems(&["token_page.rs"], &modules),
        Vec::<String>::new(),
        "a page on the tokens passes"
    );
}

/// What the guard reads as the old dark palette, and what it does not.
#[test]
fn the_old_palette_is_what_the_guard_finds() {
    let found = |text: &str| palette(&code(text)).len();
    for dark in [
        "nsh_core::color::rgb(0x6C6BEF)",
        ".bg(hex(0x11141A, 0.94))",
        ".bg(white(0.08))",
        ".bg(faded(tint, 0.14))",
        "const ON_ACCENT: Rgba = Rgba {\n    r: 1.0,",
        ".bg(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.01 })",
    ] {
        assert_eq!(found(dark), 1, "{dark}");
    }
    for fine in [
        ".text_color(move || token(\"foreground\"))",
        "let accent = Rgba { a: 1.0, ..accent };",
        "pub fn color(&self, slot: Slot) -> Rgba {",
        "json!(theme::hex_string(c))",
        "fn is_white(c: Rgba) -> bool",
        "// .bg(white(0.08))",
        "let url = \"https://example.com\"; // white(0.08)",
        "let shade = whitewash(0.08);",
    ] {
        assert_eq!(found(fine), 0, "{fine}");
    }
    assert_eq!(
        expand("crate::{face::{self, TextStyle}, form, ui::{row::row, icon::{Tone, icon}}}"),
        [
            "crate::face",
            "crate::face::TextStyle",
            "crate::form",
            "crate::ui::row::row",
            "crate::ui::icon::Tone",
            "crate::ui::icon::icon",
        ]
    );
    let modules = modules();
    assert_eq!(
        resolve("crate::ui::row::PADDING_X", "home", &modules),
        Some(("ui::row".into(), vec!["PADDING_X".into()]))
    );
    assert_eq!(
        resolve("super::prompt::Prompt", "terminal::picker", &modules),
        Some(("terminal::prompt".into(), vec!["Prompt".into()]))
    );
    assert_eq!(
        resolve("super::Ctx", "home::card", &modules),
        Some(("home".into(), vec!["Ctx".into()]))
    );
}
