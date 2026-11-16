//! The remote-path model (W6): what a path on the host is worth, how the
//! host is asked about the paths a terminal shows, the link a confirmed path
//! carries, a folder's listing as the folder sheet shows it, and the exec
//! forms of reading and changing files on a host without SFTP. One copy for
//! iOS and Android; it was `Moshpit/Models/RemotePaths.swift` (nsh at
//! 81c121a1).
//!
//! Finding the candidates in terminal text is the terminal core's
//! (`moshpit_term::paths`, T3); everything after — asking the host, the
//! link, the sheet — is here. Pure builders and parsers: the hosts run the
//! commands over whatever connection they have (SFTP first) and hand the
//! output back. Every command runs through `sh -c` with its values as
//! positional arguments, quoted by the app's one quoter, so neither the
//! login shell (zsh, fish…) nor a quote in a file name ever reinterprets
//! it.
//!
//! What stays with the hosts: the windowed SFTP reads (`WindowedRead`,
//! `InOrderBytes`), which are their SFTP clients' async loops.

use std::collections::{BTreeMap, BTreeSet};

use moshpit_install::commands::quote;

// MARK: - What a tap can open

/// A file a tap on a terminal path opens in place: what QuickLook shows
/// without help. Source code is deliberately not here — Claude Code
/// mentions dozens of source paths a turn, and underlining every one would
/// bury the outputs a person wants to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Document,
    Text,
}

/// The extensions worth a link, by kind.
pub const KINDS: [(Kind, &[&str]); 6] = [
    (
        Kind::Image,
        &[
            "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "avif",
            "ico",
        ],
    ),
    (Kind::Video, &["mp4", "mov", "m4v"]),
    (Kind::Audio, &["mp3", "m4a", "wav", "aac", "aiff", "flac"]),
    (Kind::Pdf, &["pdf"]),
    (
        Kind::Document,
        &[
            "docx", "doc", "xlsx", "xls", "pptx", "ppt", "key", "pages", "numbers", "rtf",
        ],
    ),
    (
        Kind::Text,
        &[
            "md", "markdown", "txt", "csv", "tsv", "json", "html", "htm", "svg", "log",
        ],
    ),
];

/// Every extension worth a link, without its dot.
pub fn extensions() -> impl Iterator<Item = &'static str> {
    KINDS.iter().flat_map(|(_, exts)| exts.iter().copied())
}

/// The kind a path's extension names, case-insensitively: none for anything
/// QuickLook wouldn't show, or a path whose last dot is in a folder's name.
pub fn kind(path: &str) -> Option<Kind> {
    let dot = path.rfind('.')?;
    let ext = &path[dot + 1..];
    if ext.is_empty() || ext.contains('/') {
        return None;
    }
    let ext = ext.to_lowercase();
    KINDS
        .iter()
        .find(|(_, exts)| exts.contains(&ext.as_str()))
        .map(|(kind, _)| *kind)
}

/// The extensions as one ERE, sorted, for the probe's "does this folder hold
/// anything previewable" check.
pub fn extension_pattern() -> String {
    let sorted: BTreeSet<&str> = extensions().collect();
    format!("\\.({})$", sorted.into_iter().collect::<Vec<_>>().join("|"))
}

// MARK: - Asking the host about paths

/// A path the probe confirmed: where it is, and whether it is a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub is_directory: bool,
}

/// How deep the below-the-working-directory lookup goes: enough for
/// `repo/episodes/name/build/concept/v2/a.png`; about a second on a
/// 65k-entry project tree.
pub const SEARCH_DEPTH: usize = 6;

/// The probe: for each candidate (`F` or `D`, then its text), whether it is
/// a file — or a folder holding something previewable — resolved against the
/// working directory, and when it is not there, looked up below it, newest
/// match winning (an agent that ran `cd v2 && …` reports `out/a.png`
/// relative to `v2` while the pane never left the parent). One `find`
/// covers every miss in the batch, and a pane in `$HOME` or `/` is never
/// searched. It prints `index<TAB>f|d<TAB>path` per hit.
pub fn probe_script() -> String {
    PROBE_SCRIPT.replace("{SEARCH_DEPTH}", &SEARCH_DEPTH.to_string())
}

const PROBE_SCRIPT: &str = r#"cwd=$1; re=$2; shift 2; i=0; miss=; tab=$(printf '\t')
for a in "$@"; do
  t=${a#?}; k=${a%"$t"}
  case $t in
    "~/"*) p=$HOME/${t#??} ;;
    /*) p=$t ;;
    *) p=${cwd:-$HOME}/$t ;;
  esac
  hit=
  case $k in
    F) [ -f "$p" ] && hit=f ;;
  esac
  if [ -z "$hit" ] && [ -d "$p" ]; then
    ls -1 "$p" 2>/dev/null | grep -qiE "$re" && hit=d
  fi
  if [ -n "$hit" ]; then
    printf '%s\t%s\t%s\n' "$i" "$hit" "$p"
  else
    case $t in
      /*|"~/"*|../*) ;;
      *) s=${t#./}; miss="$miss$i$tab$k$tab${s%/}
" ;;
    esac
  fi
  i=$((i+1))
done
case $cwd in ""|/|"$HOME") exit 0 ;; esac
set --
while IFS=$tab read -r i k s; do
  [ -n "$s" ] || continue
  [ $# -eq 0 ] || set -- "$@" -o
  set -- "$@" -name "${s##*/}"
done <<EOF
$miss
EOF
[ $# -gt 0 ] || exit 0
found=$(find "$cwd" -maxdepth {SEARCH_DEPTH} \( -name .git -o -name node_modules -o -name Library \
  -o -name .cache -o -name .build -o -name DerivedData -o -name Pods \) -prune \
  -o \( "$@" \) -print 2>/dev/null)
[ -n "$found" ] || exit 0
while IFS=$tab read -r i k s; do
  [ -n "$s" ] || continue
  set --
  while IFS= read -r f; do
    case $f in
      */"$s")
        if [ "$k" = F ] && [ -f "$f" ]; then set -- "$@" "$f"
        elif [ -d "$f" ] && ls -1 "$f" 2>/dev/null | grep -qiE "$re"; then set -- "$@" "$f"
        fi ;;
    esac
  done <<EOF
$found
EOF
  [ $# -gt 0 ] || continue
  h=$(ls -1td -- "$@" 2>/dev/null | head -n 1)
  [ -n "$h" ] || continue
  if [ -d "$h" ]; then hit=d; else hit=f; fi
  printf '%s\t%s\t%s\n' "$i" "$hit" "$h"
done <<EOF
$miss
EOF"#;

/// A candidate the probe is asked about: its text, and whether only a
/// folder can be meant (it ends in `/`, or names no previewable file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub token: String,
    pub directory_only: bool,
}

/// The command line for one batch. `cwd` none = unknown (a plain shell that
/// never reported one): relative paths resolve against `$HOME` and nothing
/// is searched for.
pub fn probe_command(cwd: Option<&str>, candidates: &[Candidate]) -> String {
    let mut words = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        quote(&probe_script()),
        "moshpit-probe".to_owned(),
        quote(cwd.unwrap_or_default()),
        quote(&extension_pattern()),
    ];
    words.extend(candidates.iter().map(|c| {
        let flag = if c.directory_only { "D" } else { "F" };
        quote(&format!("{flag}{}", c.token))
    }));
    words.join(" ")
}

/// The probe's `index<TAB>f|d<TAB>path` lines, back onto a batch of `count`:
/// an index outside it, a kind it does not know, a relative path or a
/// malformed line is ignored.
pub fn parse_probe(output: &str, count: usize) -> BTreeMap<usize, Hit> {
    let mut hits = BTreeMap::new();
    for line in output.split('\n').filter(|l| !l.is_empty()) {
        let mut fields = line.splitn(3, '\t');
        let (Some(index), Some(kind), Some(path)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some(index) = swift_int(index).filter(|&i| i < count) else {
            continue;
        };
        if (kind != "f" && kind != "d") || !path.starts_with('/') {
            continue;
        }
        hits.insert(
            index,
            Hit {
                path: tidy(path),
                is_directory: kind == "d",
            },
        );
    }
    hits
}

/// A non-negative index as Swift's `Int(_:)` reads one: ASCII digits after
/// an optional sign (a negative one is refused, "-0" is 0).
fn swift_int(text: &str) -> Option<usize> {
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'+') => (false, &text[1..]),
        Some(b'-') => (true, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = digits.parse().ok()?;
    (!negative || n == 0).then_some(n)
}

/// The lexical cleanup of what the shell built (`/repo/./out/`, `//`).
/// `..` is left alone: resolving it lexically can disagree with the
/// filesystem once symlinks are involved, and the host is the authority.
pub fn tidy(path: &str) -> String {
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    format!("/{}", parts.join("/"))
}

// MARK: - The link a tap carries

/// The scheme a confirmed path is tagged with on the terminal's cells: one
/// only this app reads, so a tap can tell a host path from a web URL
/// without guessing. A program's own OSC-8 `file://` links route the same
/// way.
pub const LINK_SCHEME: &str = "moshpit-preview";

/// The link for host path `path`: `moshpit-preview://` and the path, each
/// byte a URL path may not hold percent-encoded (Foundation's
/// `URLComponents`, `urlPathAllowed`).
pub fn link(path: &str) -> String {
    let mut out = format!("{LINK_SCHEME}://");
    if !path.is_empty() && !path.starts_with('/') {
        // Not a path a URL with a host can carry: as it is.
        out.push_str(path);
        return out;
    }
    for &b in path.as_bytes() {
        if path_allowed(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// RFC 3986's pchar and `/`: unreserved, sub-delims, `:` and `@`.
fn path_allowed(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b)
}

/// The host path a tapped link names: one of ours or a `file://` link, its
/// path percent-decoded and without a trailing slash (as Foundation's
/// `URL.path` reads it); none for another scheme, or a path that is not
/// absolute.
pub fn link_path(link: &str) -> Option<String> {
    let link = link.trim();
    let colon = link.find(':')?;
    let scheme = &link[..colon];
    let valid_scheme = scheme
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
    if !valid_scheme {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    if scheme != LINK_SCHEME && scheme != "file" {
        return None;
    }
    let mut rest = &link[colon + 1..];
    if let Some(after) = rest.strip_prefix("//") {
        // The authority runs to the path.
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        rest = &after[end..];
    }
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let mut path = percent_decode(&rest[..end])?;
    // Foundation's `URL.path`: no trailing slash but the root's.
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    path.starts_with('/').then_some(path)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|_| bytes[i] == b'%')
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

// MARK: - A folder on the host

/// One file or folder on the host, as the folder sheet needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: String,
    pub is_directory: bool,
    /// None when the host didn't say (the exec listing gives names only).
    pub size: Option<u64>,
    /// Seconds since 1970; none when the host didn't say.
    pub modified: Option<f64>,
}

impl Entry {
    /// The last part of its path.
    pub fn name(&self) -> &str {
        let trimmed = self.path.trim_end_matches('/');
        trimmed.rsplit('/').next().unwrap_or(trimmed)
    }

    /// What a tap opens it as: none for a folder or a file QuickLook
    /// wouldn't show.
    pub fn kind(&self) -> Option<Kind> {
        if self.is_directory {
            None
        } else {
            kind(self.name())
        }
    }

    /// Images and video: what a folder shows as thumbnails, ahead of
    /// everything else.
    pub fn is_media(&self) -> bool {
        matches!(self.kind(), Some(Kind::Image | Kind::Video))
    }
}

/// The folders enclosing `path`, nearest first, down to `/`: the folder
/// sheet's title menu. None for a relative path.
pub fn enclosing_folders(path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if !path.starts_with('/') || parts.is_empty() {
        return Vec::new();
    }
    let mut folders = Vec::new();
    while !parts.is_empty() {
        parts.pop();
        folders.push(format!("/{}", parts.join("/")));
    }
    folders
}

/// What a folder shows of `entries`: subfolders and previewable files, no
/// dot files, newest first (a folder an agent just wrote into leads with
/// what it wrote); entries with no time keep their order, after the dated
/// ones.
pub fn browsable(entries: &[Entry]) -> Vec<Entry> {
    let mut shown: Vec<(usize, &Entry)> = entries
        .iter()
        .filter(|e| !e.name().starts_with('.') && (e.is_directory || e.kind().is_some()))
        .enumerate()
        .collect();
    shown.sort_by(|(i, a), (j, b)| match (a.modified, b.modified) {
        (Some(x), Some(y)) if x != y => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => i.cmp(j),
    });
    shown.into_iter().map(|(_, e)| e.clone()).collect()
}

/// The thumbnail grid: images and video, in the listing's order.
pub fn media(entries: &[Entry]) -> Vec<Entry> {
    entries.iter().filter(|e| e.is_media()).cloned().collect()
}

/// The list under the grid: subfolders, then every other file.
pub fn rows(entries: &[Entry]) -> Vec<Entry> {
    entries
        .iter()
        .filter(|e| e.is_directory)
        .chain(entries.iter().filter(|e| !e.is_directory && !e.is_media()))
        .cloned()
        .collect()
}

// MARK: - Reading a host without SFTP

/// The exec download moves the file as base64 in one reply; past this it
/// isn't worth holding in memory twice.
pub const EXEC_DOWNLOAD_LIMIT: u64 = 32 * 1024 * 1024;

/// The line an exec reader prints before its answer: what came before it
/// is the account's shell's (B463, the exec side).
pub const OUTPUT_MARK: &str = "MOSHPIT_OUTPUT";

/// The reader's own output: what follows its mark. sshd runs every exec
/// command as `$SHELL -c`, and bash reads `~/.bashrc` for that, so an rc
/// file's `echo` came ahead of the answer: the stat read it as the answer
/// (the file "no longer on the host") and the listing took it into its
/// first entry. Output without the mark (an older command) is taken whole.
fn past_mark(output: &str) -> &str {
    let mark = format!("{OUTPUT_MARK}\n");
    match output.find(&mark) {
        Some(at) => &output[at + mark.len()..],
        None => output,
    }
}

/// Whether `path` is a folder (`d`) or a file and its size (`f <bytes>`).
pub fn stat_command(path: &str) -> String {
    let q = quote(path);
    format!(
        "printf '%s\\n' {OUTPUT_MARK}; if [ -d {q} ]; then echo d; elif [ -f {q} ]; then printf 'f '; wc -c < {q}; fi"
    )
}

/// What [`stat_command`] printed, for `path`.
pub fn parse_stat(output: &str, path: &str) -> Option<Entry> {
    let line = past_mark(output).trim();
    if line == "d" {
        return Some(Entry {
            path: path.to_owned(),
            is_directory: true,
            size: None,
            modified: None,
        });
    }
    let size = line.strip_prefix('f')?.trim_matches([' ', '\t']);
    Some(Entry {
        path: path.to_owned(),
        is_directory: false,
        size: size.parse().ok(),
        modified: None,
    })
}

/// Lists the folder (B333). The names come NUL-separated, so a name with a
/// line break stays one name, each with its kind taken through links (a
/// link to a folder is a folder, under the link's own name) and its
/// modified time: `<d|f> <seconds|-> <name>\0`. GNU find does it in one
/// process (`%Y` follows links; asked first whether it can, so a find that
/// fails part way never runs the fallback after it); elsewhere a POSIX
/// loop takes the kind from `[ -d ]`, which follows links, and the time
/// from GNU's or BSD's `stat`, or none.
pub fn list_command(path: &str) -> String {
    sh(
        concat!(
            "printf '%s\\n' MOSHPIT_OUTPUT; ",
            "cd \"$1\" || exit 1; ",
            "if find . -maxdepth 0 -printf '' >/dev/null 2>&1; then ",
            "find . -mindepth 1 -maxdepth 1 -printf '%Y %T@ %P\\0' 2>/dev/null; exit 0; fi; ",
            "for f in .* *; do ",
            "case $f in .|..) continue;; esac; ",
            "[ -e \"$f\" ] || [ -L \"$f\" ] || continue; ",
            "if [ -d \"$f\" ]; then k=d; else k=f; fi; ",
            "t=$(stat -c %Y -- \"$f\" 2>/dev/null || stat -f %m -- \"$f\" 2>/dev/null); ",
            "printf '%s %s %s\\0' \"$k\" \"${t:--}\" \"$f\"; ",
            "done"
        ),
        &[path],
    )
}

/// What [`list_command`] printed, in `folder`, newest first (as `ls -t`
/// gave it): entries with no time after the dated ones, in the order they
/// came.
pub fn parse_list(output: &str, folder: &str) -> Vec<Entry> {
    let base = folder.strip_suffix('/').unwrap_or(folder);
    let mut entries: Vec<Entry> = past_mark(output)
        .split('\0')
        .filter_map(|record| {
            let (kind, rest) = record.split_once(' ')?;
            let (time, name) = rest.split_once(' ')?;
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                return None;
            }
            Some(Entry {
                path: format!("{base}/{name}"),
                is_directory: kind == "d",
                size: None,
                modified: time.parse::<f64>().ok(),
            })
        })
        .collect();
    entries.sort_by(|a, b| match (a.modified, b.modified) {
        (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    entries
}

/// The file as base64, for the exec download.
pub fn cat_command(path: &str) -> String {
    format!("base64 < {}", quote(path))
}

// MARK: - Changing files on the host

/// Printed by an exec edit that did what it was asked.
pub const EDIT_OK: &str = "MOSHPIT_EDIT_OK";
/// Printed when the target name is already taken.
pub const EDIT_TAKEN: &str = "MOSHPIT_EDIT_TAKEN";
/// Printed when a delete would take the home folder (B816).
pub const EDIT_HOME: &str = "MOSHPIT_EDIT_HOME";

/// A name the user typed, trimmed; none when it can't name one item in a
/// folder: empty, `.` or `..`, or holding a `/` or a NUL anywhere — among
/// the scalars, as the host reads the bytes (B141: a `/` with a combining
/// mark after it is one character that isn't "/", and named two levels).
pub fn valid_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return None;
    }
    Some(name.to_owned())
}

/// Whether `a` and `b` are one name: the same in one Unicode form (NFC).
/// "café" composed (the phone's) and decomposed (an old Mac disk's) are
/// one name to a person, and to a disk that normalises (B332).
pub fn same_name(a: &str, b: &str) -> bool {
    use unicode_normalization::UnicodeNormalization;
    a == b || a.nfc().eq(b.nfc())
}

/// Whether `name` is among `taken`, as [`same_name`] compares.
pub fn is_taken(name: &str, taken: &BTreeSet<String>) -> bool {
    taken.contains(name) || taken.iter().any(|t| same_name(t, name))
}

/// `name`, or the first Finder-style variant not in `taken`: `shot.png` →
/// `shot 2.png` → `shot 3.png`. An upload never overwrites. Names compare
/// in one Unicode form ([`same_name`]); what comes back keeps the bytes of
/// `name`.
pub fn unique_name(name: &str, taken: &BTreeSet<String>) -> String {
    if !is_taken(name, taken) {
        return name.to_owned();
    }
    let (stem, suffix) = match name.rfind('.') {
        // `NSString.pathExtension`: none for a dot file's only dot.
        Some(dot) if dot > 0 && dot + 1 < name.len() => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let mut n = 2;
    loop {
        let candidate = format!("{stem} {n}{suffix}");
        if !is_taken(&candidate, taken) {
            return candidate;
        }
        n += 1;
    }
}

/// `name` in `directory`.
pub fn child(name: &str, directory: &str) -> String {
    if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

/// Whether the sheet may delete `path` at all: absolute, at least two
/// levels deep, no `..` — never `/`, never a top-level `/usr`.
pub fn deletable(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    path.starts_with('/')
        && parts.iter().filter(|p| **p != ".").count() >= 2
        && !parts.contains(&"..")
}

fn sh(script: &str, args: &[&str]) -> String {
    let mut words = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        quote(script),
        "moshpit-edit".to_owned(),
    ];
    words.extend(args.iter().map(|a| quote(a)));
    words.join(" ")
}

/// Makes folder `path`.
pub fn make_directory_command(path: &str) -> String {
    sh(&format!("mkdir -- \"$1\" && echo {EDIT_OK}"), &[path])
}

/// Renames `path` to `new_path`, never over something there. The file
/// itself is not in the way (r42): on a disk that ignores case (a Mac's),
/// a new name that only changes the old one's case finds the file it
/// renames (`-ef`, the same file).
pub fn rename_command(path: &str, new_path: &str) -> String {
    sh(
        &format!(
            "if {{ [ -e \"$2\" ] || [ -L \"$2\" ]; }} && ! [ \"$1\" -ef \"$2\" ]; then echo {EDIT_TAKEN}; exit 1; fi; mv -- \"$1\" \"$2\" && echo {EDIT_OK}"
        ),
        &[path, new_path],
    )
}

/// Deletes `path`; none for a path [`deletable`] refuses.
///
/// Fix (B816): [`deletable`] lets any folder two levels deep go, and the
/// home folder is one (`/Users/dev`, `/home/dev`): one Delete in the folder
/// sheet took it and everything in it, and its parent the same way. The
/// host refuses, saying [`EDIT_HOME`]: never the home folder, nor a folder
/// it is in — the home and the path compared as given and as their real
/// paths, so neither `/var` being `/private/var` on a Mac nor a linked
/// folder on the way gets one past.
pub fn remove_command(path: &str, is_directory: bool) -> Option<String> {
    if !deletable(path) {
        return None;
    }
    let rm = if is_directory { "rm -rf" } else { "rm -f" };
    Some(sh(
        &format!(
            "h=$(cd \"$HOME\" 2>/dev/null && pwd -P); p=$(cd \"$1\" 2>/dev/null && pwd -P); \
             for d in \"$HOME\" \"$h\"; do [ -n \"$d\" ] || continue; \
             for t in \"$1\" \"$p\"; do [ -n \"$t\" ] || continue; \
             case ${{d%/}}/ in \"${{t%/}}/\"*) echo {EDIT_HOME}; exit 1;; esac; done; done; \
             {rm} -- \"$1\" && echo {EDIT_OK}"
        ),
        &[path],
    ))
}

/// An upload over exec: base64 appended to a side file, decoded at the
/// end; it never starts over an existing name.
///
/// B331: one stopped or failed part way left `<name>.b64part` in the
/// person's folder for good, and a decode that failed left a half-written
/// `<name>`, which then made every retry say the name was taken. The
/// decode now goes to a second side file, moved into place only once it
/// worked, and [`UploadPlan::abandon`] clears both side files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlan {
    pub begin: String,
    pub finish: String,
    /// What an upload given up (stopped, or failed at any step) runs: its
    /// side files removed. `<name>` itself is only ever made by a decode
    /// that worked, so it is not touched.
    pub abandon: String,
    path: String,
}

impl UploadPlan {
    /// Appends one base64 chunk (the alphabet is inert in single quotes).
    pub fn append(&self, chunk: &str) -> String {
        sh("printf %s \"$2\" >> \"$1.b64part\"", &[&self.path, chunk])
    }
}

/// The upload to `path`.
pub fn upload_plan(path: &str) -> UploadPlan {
    UploadPlan {
        begin: sh(
            &format!(
                "if [ -e \"$1\" ] || [ -L \"$1\" ]; then echo {EDIT_TAKEN}; exit 1; fi; : > \"$1.b64part\" && echo {EDIT_OK}"
            ),
            &[path],
        ),
        finish: sh(
            &format!(
                "{{ base64 -d < \"$1.b64part\" > \"$1.b64dec\" 2>/dev/null || base64 -D < \"$1.b64part\" > \"$1.b64dec\"; }} && mv -- \"$1.b64dec\" \"$1\" && rm -f \"$1.b64part\" && echo {EDIT_OK}"
            ),
            &[path],
        ),
        abandon: sh("rm -f -- \"$1.b64part\" \"$1.b64dec\"", &[path]),
        path: path.to_owned(),
    }
}

#[cfg(test)]
mod tests;
