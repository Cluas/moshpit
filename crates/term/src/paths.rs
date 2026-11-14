//! Paths an agent prints — "对比图：qchar/out/GripLab.png", "saved to
//! ~/Desktop/charts/" — picked out of a logical line as candidates for
//! links the host confirms. The rules are the app's `TerminalPathScanner`
//! (Moshpit/Models/RemotePaths.swift, the SwiftTerm path's), cell for cell,
//! so both terminal views and Android offer the host the same runs; which
//! file kinds are worth a link is the platform's to say (what it can
//! preview: [`Rules`]).
//!
//! Only paths the host confirmed become links ([`crate::Terminal::set_path_links`]),
//! so the text itself is never trusted.

use std::{collections::HashSet, ops::Range};

/// What the platform can preview, by extension: a bare file name is a
/// candidate only when it names one, and a path with any other extension is
/// not a folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    extensions: HashSet<String>,
}

impl Rules {
    /// `extensions` without their dots, any case.
    pub fn new<I, S>(extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Rules {
            extensions: extensions
                .into_iter()
                .map(|e| e.as_ref().trim().trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }

    /// Whether `path`'s extension names a file worth a link
    /// (`PreviewableFile.kind(of:)`).
    pub fn previewable(&self, path: &str) -> bool {
        let Some(dot) = path.rfind('.') else {
            return false;
        };
        let ext = &path[dot + 1..];
        !ext.is_empty() && !ext.contains('/') && self.extensions.contains(&ext.to_lowercase())
    }
}

/// One run that could name a file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The path as printed (`~/…`, `/…`, `./…`, relative, or a bare file
    /// name).
    pub token: String,
    /// The cells it covers, as indices into the scanned cells.
    pub cells: Range<usize>,
    /// It can only be worth a link as a folder (its last component names no
    /// previewable file): the host accepts it only if it is a directory
    /// holding something previewable.
    pub directory_only: bool,
}

/// How far back a CJK name glued to a path is followed.
const HEAD_LIMIT: usize = 16;

/// The candidates among `cells` (each cell's text, in order, a wide
/// character once), in the order the app's scanner offers them: left to
/// right, and for one run the longest start first (the first the host
/// confirms is the link; the shorter ones start inside it).
pub fn scan(cells: &[&str], rules: &Rules) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        if !is_path(cells[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < cells.len() && is_path(cells[i]) {
            i += 1;
        }
        let run: String = cells[start..i].concat();
        let tail = if run.contains('/') {
            wide_name_tail(cells, i, rules)
        } else {
            None
        };
        let body = match &tail {
            Some((name, _)) => format!("{run}{name}"),
            None => run.clone(),
        };
        let end = tail.as_ref().map_or(i, |t| t.1);
        let mut variants = Vec::new();
        for head in wide_head_starts(cells, start, &run) {
            let raw = format!("{}{body}", cells[head..start].concat());
            if let Some(c) = classify(&raw, head, end, rules) {
                variants.push(c);
            }
        }
        if tail.is_some()
            && let Some(c) = classify(&body, start, end, rules)
        {
            variants.push(c);
        }
        if tail.is_some() && !variants.is_empty() {
            found.extend(variants);
            i = end;
            continue;
        }
        if let Some(c) = classify(&run, start, i, rules) {
            variants.push(c);
        }
        found.extend(variants);
    }
    found
}

/// One character a path is made of: ASCII's, or a letter of a script that
/// puts spaces between its words (Latin with its accents, Greek, Cyrillic),
/// which runs on inside a name like any ASCII letter. A cell is a grapheme,
/// so a letter carries its combining marks.
///
/// Fix (B462): only ASCII was; any other letter ended the run, and only a
/// CJK name's head or tail carried a path past it. "café/menu.png" was cut
/// at the accent and offered as "é/menu.png" and "/menu.png". Scripts
/// written without spaces (CJK, Thai) still end the run: there prose runs
/// straight into a path, and the heads and tails sort that out.
fn is_path(cell: &str) -> bool {
    let b = cell.as_bytes();
    if b.len() == 1 {
        return b[0].is_ascii_alphanumeric() || b"._@+-/~".contains(&b[0]);
    }
    cell.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || is_spaced_letter(c))
}

/// A letter of Latin (beyond ASCII), Greek or Cyrillic.
fn is_spaced_letter(c: char) -> bool {
    matches!(
        c,
        '\u{C0}'..='\u{24F}' | '\u{1E00}'..='\u{1EFF}' | '\u{370}'..='\u{3FF}'
            | '\u{1F00}'..='\u{1FFF}' | '\u{400}'..='\u{52F}'
    ) && c.is_alphabetic()
}

/// A wide (CJK) letter or digit: part of a name.
fn is_wide_name(cell: &str) -> bool {
    !cell.is_ascii()
        && cell
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c.is_numeric())
}

/// Where a path may start before its ASCII run, when the run opens on a
/// separator glued to a CJK name: `成品` + `/西游.mp4`, `粗剪` + `.mp4`.
/// Chinese prose runs into the name with no space ("已经输出到成品/a.mp4"),
/// so the start can't be read off the text: every ideograph back to the
/// nearest break is offered, farthest first, and the host settles which
/// exists. A run that opens on a letter ("保存到out/a.png") gets none.
fn wide_head_starts(cells: &[&str], start: usize, run: &str) -> Vec<usize> {
    if !run.starts_with(['/', '.', '-', '_']) {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut i = start;
    while i > 0 && starts.len() < HEAD_LIMIT {
        let c = cells[i - 1];
        if is_wide_name(c) {
            starts.push(i - 1);
        } else if !matches!(c, "-" | "_" | ".") {
            break;
        }
        i -= 1;
    }
    starts.reverse();
    starts
}

/// The CJK-named rest of a path whose ASCII run stopped at an ideograph
/// (`out/` + `羽毛球/a.png`), taken only when it ends the way a path does —
/// a previewable file or a `/` — so prose that merely follows a path
/// ("保存到out/a.png了") stays prose. Its text and the cell after it.
fn wide_name_tail(cells: &[&str], from: usize, rules: &Rules) -> Option<(String, usize)> {
    if from >= cells.len() || !is_wide_name(cells[from]) {
        return None;
    }
    let mut end = from;
    while end < cells.len() && (is_path(cells[end]) || is_wide_name(cells[end])) {
        end += 1;
    }
    while end > from && matches!(cells[end - 1], "." | "-") {
        end -= 1;
    }
    let name: String = cells[from..end].concat();
    let last = name.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    (name.ends_with('/') || rules.previewable(last)).then_some((name, end))
}

/// `raw` (cells `start..end`) as a candidate, or not one.
fn classify(raw: &str, start: usize, end: usize, rules: &Rules) -> Option<Candidate> {
    // A sentence or list that ends on the path ("saved to out/a.png.")
    // leaves its punctuation on the run.
    let token = raw.trim_end_matches(['.', '-']);
    let trimmed = raw.len() - token.len();
    if token.is_empty() || token.chars().count() > 1024 {
        return None;
    }
    // `//` is a URL's remains (`https:` already split off) or a typo; `~`
    // anywhere but a leading `~/` is not a path at all.
    if token.contains("//") {
        return None;
    }
    if let Some(t) = token.find('~')
        && (t != 0 || !token.starts_with("~/") || token[1..].contains('~'))
    {
        return None;
    }
    let ends_with_slash = token.ends_with('/');
    let last = token.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    if last.is_empty() || last == "." || last == ".." {
        return None;
    }
    let previewable = !ends_with_slash && rules.previewable(last);
    // An extension with no name in front (".mp4"): what is left of a CJK
    // file name once the ideographs are cut off, never a file.
    if previewable && last.starts_with('.') && !last[1..].contains('.') {
        return None;
    }
    if !token.contains('/') {
        // A bare word is only a candidate when it names a previewable file.
        if !previewable {
            return None;
        }
    } else if !previewable {
        // Folder candidates: no extension, or a trailing slash. Anything
        // else with an extension (`src/main.swift`) is a file we don't
        // preview — not a folder.
        if !ends_with_slash && last.contains('.') {
            return None;
        }
        // Dates and fractions ("10/18", "1/2").
        let segments: Vec<&str> = token.split('/').filter(|s| !s.is_empty()).collect();
        if segments.iter().all(|s| s.chars().all(char::is_numeric)) {
            return None;
        }
        // A single rooted word is a slash command far more often than a
        // folder of outputs ("/config", "/help").
        if token.starts_with('/') && segments.len() < 2 {
            return None;
        }
    }
    Some(Candidate {
        token: token.to_string(),
        cells: start..end - trimmed,
        directory_only: !previewable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The iOS app's previewable kinds (`PreviewableFile.kinds`).
    fn rules() -> Rules {
        Rules::new([
            "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "avif",
            "ico", "mp4", "mov", "m4v", "mp3", "m4a", "wav", "aac", "aiff", "flac", "pdf", "docx",
            "doc", "xlsx", "xls", "pptx", "ppt", "key", "pages", "numbers", "rtf", "md",
            "markdown", "txt", "csv", "tsv", "json", "html", "htm", "svg", "log",
        ])
    }

    /// `line` as cells: one per character, as a logical line hands them
    /// over (a wide character once).
    fn scan_line(line: &str) -> Vec<Candidate> {
        let cells: Vec<String> = line.chars().map(String::from).collect();
        let refs: Vec<&str> = cells.iter().map(String::as_str).collect();
        scan(&refs, &rules())
    }

    fn tokens(line: &str) -> Vec<String> {
        scan_line(line).into_iter().map(|c| c.token).collect()
    }

    // The cases of the app's RemotePathsTests, scanner section.

    #[test]
    fn the_transcripts_own_line() {
        let line = "  3. 对比图：qchar/out/GripLab.png（准备、放网、平抽）。规则见 CLAUDE.md 第 3 步和 qchar/README.md「体型怎么挑」。";
        let found = scan_line(line);
        assert_eq!(
            found.iter().map(|c| c.token.as_str()).collect::<Vec<_>>(),
            ["qchar/out/GripLab.png", "CLAUDE.md", "qchar/README.md"]
        );
        assert!(found.iter().all(|c| !c.directory_only));
        let chars: Vec<char> = line.chars().collect();
        let first: String = chars[found[0].cells.clone()].iter().collect();
        assert_eq!(first, "qchar/out/GripLab.png");
    }

    #[test]
    fn punctuation_ends_the_path() {
        assert_eq!(tokens("saved to out/a.png."), ["out/a.png"]);
        assert_eq!(
            tokens("**out/plot.png** and (charts/b.jpg), `c.gif`"),
            ["out/plot.png", "charts/b.jpg", "c.gif"]
        );
        assert_eq!(tokens("保存到out/a.png了"), ["out/a.png"]);
        assert_eq!(
            tokens("see src/main.swift:12 and out/a.png:3"),
            ["out/a.png"]
        );
    }

    #[test]
    fn shapes() {
        assert_eq!(
            tokens("/Users/me/Desktop/shot.PNG ~/Downloads/clip.mov ./a.pdf ../b.md"),
            [
                "/Users/me/Desktop/shot.PNG",
                "~/Downloads/clip.mov",
                "./a.pdf",
                "../b.md"
            ]
        );
        assert_eq!(tokens("README Makefile notes.txt"), ["notes.txt"]);
    }

    #[test]
    fn folders() {
        let found = scan_line(
            "outputs in out/plots and ~/Desktop/charts/ v1.2/ src/main.swift 10/18 1/2 /config",
        );
        assert_eq!(
            found.iter().map(|c| c.token.as_str()).collect::<Vec<_>>(),
            ["out/plots", "~/Desktop/charts/", "v1.2/"]
        );
        assert!(found.iter().all(|c| c.directory_only));
    }

    #[test]
    fn wide_names() {
        assert_eq!(
            tokens("改稿：episodes/nolift/改稿建议.md。"),
            ["episodes/nolift/改稿建议.md"]
        );
        assert_eq!(
            tokens("见 out/羽毛球/a.png 和 out/图/，"),
            ["out/羽毛球/a.png", "out/图/"]
        );
        assert_eq!(tokens("在 out/里面有图"), ["out/"]);
        let found = scan_line("看x/图表.png。");
        assert_eq!(found[0].token, "x/图表.png");
        assert_eq!(found[0].cells, 1..9, "every cell of it, not the full stop");
    }

    #[test]
    fn wide_heads() {
        let found = scan_line("for you: 成品/西游-写实版-粗剪.mp4. It's");
        assert_eq!(
            found.iter().map(|c| c.token.as_str()).collect::<Vec<_>>(),
            [
                "成品/西游-写实版-粗剪.mp4",
                "品/西游-写实版-粗剪.mp4",
                "/西游-写实版-粗剪.mp4"
            ]
        );
        assert_eq!(
            found[0].cells,
            9..9 + 2 + 1 + 2 + 1 + 3 + 1 + 2 + 4,
            "not the period"
        );
        assert_eq!(
            tokens("见 写实版-粗剪.mp4。"),
            [
                "写实版-粗剪.mp4",
                "实版-粗剪.mp4",
                "版-粗剪.mp4",
                "粗剪.mp4",
                "剪.mp4"
            ],
            "a bare \".mp4\" is no file"
        );
        assert_eq!(
            tokens("输出到成品/a.png")[..2],
            ["输出到成品/a.png", "出到成品/a.png"]
        );
        assert_eq!(tokens("规则见CLAUDE.md"), ["CLAUDE.md"]);
    }

    #[test]
    fn non_paths() {
        assert!(tokens("https://example.com/a.png and file:///tmp/x.png").is_empty());
        assert!(tokens("a~b/c.png me@example.com").is_empty());
    }

    #[test]
    fn the_platform_says_what_is_worth_a_link() {
        let none = Rules::new(Vec::<String>::new());
        assert!(none.is_empty());
        let cells = ["a", ".", "p", "n", "g"];
        assert!(
            scan(&cells, &none).is_empty(),
            "no previewable kinds: no bare file names"
        );
        let pngs = Rules::new([".PNG"]);
        assert_eq!(scan(&cells, &pngs)[0].token, "a.png");
        assert!(pngs.previewable("x/Y.Png"));
        assert!(!pngs.previewable("x.png/y"));
        assert!(!pngs.previewable("x."));
    }

    /// B462: any letter outside ASCII ended the run, and only a CJK name's
    /// head or tail could carry a path past it. A name that starts in ASCII
    /// and has an accent further on ("café/", "Präsentation/") was cut at
    /// the accent, and what was offered was the rest ("é/menu.png",
    /// "/menu.png"), which no host has.
    #[test]
    fn a_name_in_a_spaced_script_is_one_path() {
        assert_eq!(tokens("saved to café/menu.png"), ["café/menu.png"]);
        assert_eq!(
            tokens("Gespeichert: Präsentation/übersicht.png."),
            ["Präsentation/übersicht.png"]
        );
        assert_eq!(tokens("voir résumé.pdf"), ["résumé.pdf"]);
        assert_eq!(tokens("готово: отчёт/график.png"), ["отчёт/график.png"]);
        let found = scan_line("à Ελλάδα/χάρτης.png");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].token, "Ελλάδα/χάρτης.png");
        assert_eq!(found[0].cells, 2..19);
        // e + combining acute is one cell at the front of a name too.
        let cells = ["c", "a", "f", "e\u{301}", "/", "a", ".", "p", "n", "g"];
        let found = scan(&cells, &rules());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].token, "cafe\u{301}/a.png");
        // Scripts written without spaces still end the run.
        assert_eq!(tokens("保存到out/a.png了"), ["out/a.png"]);
    }

    #[test]
    fn a_grapheme_cell_is_one_character() {
        // "é" as e + combining acute is one cell, as the app's Character is:
        // a letter, part of the name.
        let cells = ["o", "u", "t", "/", "e\u{301}", ".", "p", "n", "g"];
        let found = scan(&cells, &rules());
        assert_eq!(found[0].token, "out/e\u{301}.png");
        assert_eq!(found[0].cells, 0..9);
    }
}
