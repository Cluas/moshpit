//! Input-method reconciliation: what a platform's text system says
//! happened, against what was already sent, gives the bytes to send.
//!
//! A phone's keyboards, input methods and dictation edit a *document*
//! (UIKit's `UITextInput`, Android's `InputConnection`); a terminal has no
//! document the remote's text is in, only a stream of keys. So the view
//! keeps a shadow document — what was typed since the last Return — and
//! this module turns each edit of it into what the remote must receive:
//! text, backspaces, Return. Everything here is a pure function of the
//! document and the call, testable without a platform.
//!
//! The rules are what the SwiftTerm fork's patches learned on devices
//! (docs/PATCHES.md at cf25626b, docs/nsh/TERMINAL-VIEW.md V6–V10):
//!
//! - **Composition** (marked text: Pinyin, kana, Zhuyin) is never sent; a
//!   delete while composing shrinks it and sends nothing (patch 7); the
//!   commit sends the composed text.
//! - **The phantom**: an empty document gets one newline in front of the
//!   caret, so a keyboard that measures one character back before each
//!   repeat of a held delete always finds one and keeps repeating over text
//!   the remote owns (patches 10, 12). Deleting it sends one backspace, as
//!   before it existed; a delete never bills the remote for it otherwise.
//! - **Edits the input system made itself are not reported back to it**
//!   (patch 23): every outcome says whether the platform must tell its
//!   input delegate. A third-party keyboard (豆包) that deletes by selecting
//!   two characters, deleting them and inserting the first again ends its
//!   voice session when told of its own edits. When the two it deleted
//!   began with the phantom, its re-insert of the phantom restores it here
//!   and sends nothing: committing a newline would press Return.
//! - **Auto-period**: a keyboard's ". " after two spaces replaces the
//!   space a shell already received; the remote gets what the keyboard
//!   meant without a stray period.
//! - **Lines inserted at once** (a keyboard's clipboard panel or quick
//!   phrases, dictation with line breaks) are a paste, not typing: typed,
//!   each line ran as it arrived (B414).
//!
//! Positions are grapheme clusters (what a user calls one character).

use unicode_segmentation::UnicodeSegmentation;

/// What to send the remote, before encoding (Return and backspace are keys
/// the terminal encodes as the program asked).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    Text(String),
    Backspace,
    Enter,
    /// The arrow keys: the caret the input system moved (B695).
    Left,
    Right,
}

/// One call's result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub typed: Vec<Typed>,
    /// The document changed in a way the input system did not make: the
    /// platform tells its input delegate (UIKit's `textWillChange` …).
    pub notify: bool,
    /// Text to send as a paste, through the platform's paste path (the
    /// session's, which knows tmux's and herdr's ways), after `typed`.
    pub paste: Option<String>,
}

/// The newline parked in front of the caret of an empty document.
pub const PHANTOM: &str = "\n";

/// The shadow document.
#[derive(Debug, Clone, Default)]
pub struct Ime {
    doc: Vec<String>,
    marked: Option<(usize, usize)>,
    selected: (usize, usize),
    focused: bool,
    /// The last delete took the phantom with a selection from the front:
    /// the input system may be about to put it back, in the same turn of
    /// the platform's event loop ([`Ime::turn_end`] ends the wait).
    phantom_reinsert: bool,
    /// The last delete removed a space at the end: an auto-period may
    /// follow, in the same turn ([`Ime::turn_end`] ends the wait; a period
    /// typed after it is the person's, B690).
    auto_period_space: bool,
}

fn graphemes(s: &str) -> Vec<String> {
    s.graphemes(true).map(str::to_string).collect()
}

impl Ime {
    pub fn new() -> Self {
        Self::default()
    }

    /// The document's text.
    pub fn text(&self) -> String {
        self.doc.concat()
    }

    /// Its length, in graphemes.
    pub fn len(&self) -> usize {
        self.doc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    /// The text of graphemes `start..end` (clamped).
    pub fn text_in(&self, start: usize, end: usize) -> String {
        let (a, b) = (self.clamp(start.min(end)), self.clamp(start.max(end)));
        self.doc[a..b].concat()
    }

    pub fn marked(&self) -> Option<(usize, usize)> {
        self.marked
    }

    /// The composition's text, if one is in progress.
    pub fn marked_text(&self) -> Option<String> {
        self.marked
            .map(|(a, b)| self.text_in(a, b))
            .filter(|s| !s.is_empty())
    }

    pub fn selected(&self) -> (usize, usize) {
        self.selected
    }

    fn clamp(&self, i: usize) -> usize {
        i.min(self.doc.len())
    }

    fn splice(&mut self, start: usize, end: usize, text: &str) -> usize {
        let (a, b) = (self.clamp(start), self.clamp(end.max(start)));
        let new = graphemes(text);
        let n = new.len();
        self.doc.splice(a..b, new);
        n
    }

    /// The phantom in front of a caret with nothing in front of it: an
    /// empty document's, or one a delete left at the front of what remains
    /// (B412: typed text after it, the caret moved before it, a held delete
    /// took the phantom and stopped). While the view holds the keyboard and
    /// nothing is being composed.
    fn prime(&mut self) {
        if !self.focused || self.marked.is_some() {
            return;
        }
        if self.doc.is_empty() {
            self.doc.push(PHANTOM.to_string());
            self.selected = (1, 1);
        } else if self.selected == (0, 0) {
            if self.doc[0] != PHANTOM {
                self.doc.insert(0, PHANTOM.to_string());
            }
            self.selected = (1, 1);
        }
    }

    /// Backspaces a stretch of the document is worth on the remote: the
    /// phantom owes nothing, but a delete that lands only on it still sends
    /// one; an empty stretch is an insertion and sends none.
    fn remote_backspaces(text: &[String]) -> usize {
        if text.is_empty() {
            return 0;
        }
        text.iter().filter(|g| *g != PHANTOM).count().max(1)
    }

    /// The view took or gave up the keyboard. Taking it primes the phantom:
    /// the delete key repeats over the remote's text from the first hold.
    pub fn focus(&mut self, focused: bool) -> Outcome {
        self.focused = focused;
        let before = self.doc.len();
        self.prime();
        Outcome {
            typed: vec![],
            notify: self.doc.len() != before,
            paste: None,
        }
    }

    /// Forgets everything typed (Return, a new prompt).
    pub fn reset(&mut self) -> Outcome {
        self.doc.clear();
        self.marked = None;
        self.selected = (0, 0);
        self.auto_period_space = false;
        self.prime();
        Outcome {
            typed: vec![],
            notify: true,
            paste: None,
        }
    }

    /// The input system inserts `text` at the caret, or in place of the
    /// composition (its commit) or of the selection.
    pub fn insert(&mut self, text: &str) -> Outcome {
        if std::mem::take(&mut self.phantom_reinsert) && text == PHANTOM {
            return self.restore_phantom();
        }
        if is_lines(text) {
            // Lines at once: a paste. The line is the remote's now.
            let mut out = self.reset();
            out.paste = Some(text.to_string());
            return out;
        }
        let had_space = self.auto_period_space;
        if !is_auto_period(text) {
            self.auto_period_space = false;
        }
        let (start, end) = self.marked.unwrap_or(self.selected);
        let mut text = text.to_string();
        if let Some(t) = self.auto_period_insertion(&text, (start, end), had_space) {
            text = t;
        }
        let n = self.splice(start, end, &text);
        self.marked = None;
        self.selected = (start + n, start + n);
        if text == "\n" || text == "\r" {
            // Return: the line is the remote's now; the delegate hears of
            // the reset even inside the input system's own call.
            let mut out = self.reset();
            out.typed = vec![Typed::Enter];
            return out;
        }
        Outcome {
            typed: vec![Typed::Text(text)],
            notify: false,
            paste: None,
        }
    }

    /// The platform's event loop finished the turn of a delete (UIKit: the
    /// main queue's next turn). A keyboard puts back the first character
    /// of a selection it deleted within that turn; a Return after it is
    /// the person's (B410). Without this, every delete into the remote's
    /// text (a third-party keyboard selects the one character before the
    /// caret: the phantom) left the next Return swallowed. The keyboard's
    /// double-space period comes in that turn too; a period typed after a
    /// delete of a space is the person's (B690).
    pub fn turn_end(&mut self) {
        self.phantom_reinsert = false;
        self.auto_period_space = false;
    }

    /// UIKit putting back the phantom it just deleted: only the document
    /// gets it back; nothing is sent.
    fn restore_phantom(&mut self) -> Outcome {
        let caret = self.selected.0;
        if caret > 0 && self.doc.get(caret - 1).is_some_and(|g| g == PHANTOM) {
            return Outcome::default();
        }
        let (a, b) = self.selected;
        self.splice(a, b, PHANTOM);
        self.selected = (a + 1, a + 1);
        Outcome::default()
    }

    /// The delete key.
    pub fn delete_backward(&mut self) -> Outcome {
        self.phantom_reinsert = false;
        if let Some((a, b)) = self.marked
            && b > a
        {
            // Composing: the composition shrinks; nothing was sent for it.
            self.doc.remove(b - 1);
            self.marked = if b - 1 <= a { None } else { Some((a, b - 1)) };
            self.selected = (b - 1, b - 1);
            return Outcome::default();
        }
        let (a, b) = self.selected;
        let mut typed = Vec::new();
        let start;
        if a == b {
            if a == 0 {
                // Nothing in front of the caret here: the remote's text.
                self.auto_period_space = false;
                typed.push(Typed::Backspace);
                let before = self.doc.len();
                self.prime();
                return Outcome {
                    typed,
                    notify: self.doc.len() != before,
                    paste: None,
                };
            }
            start = a - 1;
            let deleted = self.doc.remove(start);
            self.auto_period_space = a == self.doc.len() + 1 && deleted == " ";
            typed.push(Typed::Backspace);
        } else {
            self.auto_period_space = false;
            let old: Vec<String> = self.doc[self.clamp(a)..self.clamp(b)].to_vec();
            typed.extend(std::iter::repeat_n(
                Typed::Backspace,
                Self::remote_backspaces(&old),
            ));
            // The input system may put this selection's first character
            // back; if that is the phantom, it is not a Return.
            if a == 0 && old.first().is_some_and(|g| g == PHANTOM) {
                self.phantom_reinsert = true;
            }
            self.splice(a, b, "");
            start = a;
        }
        self.marked = None;
        self.selected = (start, start);
        self.prime();
        Outcome {
            typed,
            notify: false,
            paste: None,
        }
    }

    /// The input system's composition: `text` (None or empty: cancelled)
    /// with its caret at `sel` (graphemes into it).
    pub fn set_marked(&mut self, text: Option<&str>, sel: (usize, usize)) -> Outcome {
        self.phantom_reinsert = false;
        let (start, end) = self.marked.unwrap_or(self.selected);
        match text.filter(|t| !t.is_empty()) {
            Some(t) => {
                let n = self.splice(start, end, t);
                self.marked = Some((start, start + n));
                let caret = start + sel.0.min(n);
                self.selected = (caret, (caret + sel.1).min(start + n));
            }
            None => {
                self.splice(start, end, "");
                self.marked = None;
                self.selected = (start, start);
            }
        }
        Outcome::default()
    }

    /// The composition ends as it stands: committed.
    pub fn unmark(&mut self) -> Outcome {
        self.phantom_reinsert = false;
        let Some((a, b)) = self.marked else {
            return Outcome::default();
        };
        let text = self.text_in(a, b);
        if !text.is_empty() {
            return self.insert(&text);
        }
        self.marked = None;
        self.selected = (b, b);
        Outcome::default()
    }

    /// The input system replaces graphemes `start..end` with `text`
    /// (autocorrection, a keyboard's own edits, dictation's corrections).
    /// Ignored while composing.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) -> Outcome {
        self.phantom_reinsert = false;
        if self.marked.is_some() {
            return Outcome::default();
        }
        let (a, b) = (self.clamp(start.min(end)), self.clamp(start.max(end)));
        let old: Vec<String> = self.doc[a..b].to_vec();
        if !is_auto_period(text) {
            self.auto_period_space = false;
        }
        let text = self
            .auto_period_replacement(text, &old, b)
            .unwrap_or_else(|| text.to_string());
        let mut typed: Vec<Typed> =
            std::iter::repeat_n(Typed::Backspace, Self::remote_backspaces(&old)).collect();
        if !text.is_empty() {
            typed.push(Typed::Text(text.clone()));
        }
        let n = self.splice(a, b, &text);
        let (s0, s1) = self.selected;
        if b <= s0 {
            let shift = n as isize - (b - a) as isize;
            self.selected = (
                (s0 as isize + shift).max(0) as usize,
                (s1 as isize + shift).max(0) as usize,
            );
        } else if a < s1 {
            self.selected = (a + n, a + n);
        }
        Outcome {
            typed,
            notify: false,
            paste: None,
        }
    }

    /// The input system moves the caret or selection (it is the one moving
    /// it: nothing to report). The caret never goes in front of the phantom
    /// (B412): with nothing in front of it, a held delete stops after one
    /// press (V8) — the space bar's trackpad, dragged left into the
    /// remote's text, put it there. A selection from the front, phantom
    /// and all, stays (a third-party keyboard's delete, V9).
    pub fn select(&mut self, start: usize, end: usize) {
        let (a, b) = (self.clamp(start.min(end)), self.clamp(start.max(end)));
        let leading_phantom = self.doc.first().is_some_and(|g| g == PHANTOM);
        self.selected = if b == 0 && leading_phantom {
            (1, 1)
        } else {
            (a, b)
        };
    }

    /// The input system set its caret or selection (UIKit's
    /// `selectedTextRange`). A caret it moves (a third-party keyboard's
    /// paired punctuation stepping back into `（）`, a slide on its space
    /// bar) moves the remote's cursor too: arrows, one per grapheme, as the
    /// space bar's trackpad sends them (B695); left alone, typing went in at
    /// the remote's end, `（）你`. The move is bounded by the line typed: the
    /// caret stays behind the phantom and within the document, so no
    /// arrow goes into the remote's own text in front of it. A selection
    /// (a keyboard's delete selects what it deletes, V9), a caret coming
    /// out of one, and a caret inside a composition send nothing.
    pub fn input_select(&mut self, start: usize, end: usize) -> Outcome {
        let before = self.selected;
        self.select(start, end);
        let after = self.selected;
        if self.marked.is_some() || before.0 != before.1 || after.0 != after.1 || after == before {
            return Outcome::default();
        }
        let (arrow, n) = if after.0 < before.0 {
            (Typed::Left, before.0 - after.0)
        } else {
            (Typed::Right, after.0 - before.0)
        };
        Outcome {
            typed: vec![arrow; n.min(self.doc.len())],
            notify: false,
            paste: None,
        }
    }

    fn auto_period_insertion(
        &mut self,
        text: &str,
        range: (usize, usize),
        had_space: bool,
    ) -> Option<String> {
        if !is_auto_period(text) {
            return None;
        }
        if had_space {
            self.auto_period_space = false;
            return pending_auto_period(text);
        }
        self.auto_period_space = false;
        (text == ". "
            && range.0 == range.1
            && range.1 == self.doc.len()
            && self.doc.last().is_some_and(|g| g == " "))
        .then(|| " ".to_string())
    }

    fn auto_period_replacement(
        &mut self,
        text: &str,
        old: &[String],
        end: usize,
    ) -> Option<String> {
        if self.auto_period_space
            && let Some(t) = pending_auto_period(text)
        {
            self.auto_period_space = false;
            return Some(t);
        }
        if !is_auto_period(text)
            || end != self.doc.len()
            || old.len() > 2
            || !old.iter().all(|g| g == " ")
        {
            return None;
        }
        Some(match (text, old.len()) {
            (".", 1) => " ".into(),
            (".", _) => old.concat(),
            (_, 1) => "  ".into(),
            _ => old.concat(),
        })
    }
}

/// Text that holds a line break among other text: more than a Return.
fn is_lines(text: &str) -> bool {
    let breaks = |c: char| c == '\n' || c == '\r';
    text.contains(breaks) && text.contains(|c: char| !breaks(c))
}

fn is_auto_period(text: &str) -> bool {
    text == "." || text == ". "
}

/// After a delete of a trailing space, a keyboard's period stands for the
/// space it took.
fn pending_auto_period(text: &str) -> Option<String> {
    match text {
        "." => Some(" ".into()),
        ". " => Some("  ".into()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(o: &Outcome) -> String {
        o.typed
            .iter()
            .map(|t| match t {
                Typed::Text(s) => s.clone(),
                Typed::Backspace => "⌫".into(),
                Typed::Enter => "⏎".into(),
                Typed::Left => "←".into(),
                Typed::Right => "→".into(),
            })
            .collect()
    }

    fn focused() -> Ime {
        let mut ime = Ime::new();
        ime.focus(true);
        ime
    }

    #[test]
    fn focus_primes_the_phantom() {
        let mut ime = Ime::new();
        let o = ime.focus(true);
        assert!(o.notify, "a change the input system did not make");
        assert_eq!(ime.text(), "\n");
        assert_eq!(ime.selected(), (1, 1));
    }

    #[test]
    fn typing_and_return() {
        let mut ime = focused();
        assert_eq!(typed(&ime.insert("l")), "l");
        assert_eq!(typed(&ime.insert("s")), "s");
        let o = ime.insert("\n");
        assert_eq!(typed(&o), "⏎");
        assert!(
            o.notify,
            "Return resets the document: the delegate hears it"
        );
        assert_eq!(ime.text(), "\n", "primed again");
    }
}
