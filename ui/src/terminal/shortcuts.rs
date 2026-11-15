//! The hardware keyboard's ⌘ shortcuts on the terminal screen
//! (`TerminalCommands`): ⌘K the pane overview (open, or closed again), ⌘W
//! back home (the session stays), ⌘1–⌘9 the n-th window in the picker's
//! order.
//!
//! One handler, reached two ways:
//!
//! - NSH's keymap ([`on_key`]): the presses the terminal leaves to its next
//!   responder (SwiftTerm hands up the ⌘ chords it has no use for), where
//!   there is a ⌘ key (an iPad's keyboard; not Android, where `mod` is
//!   Control and ^W is the shell's).
//! - The host's own key commands (`{"op": "key", "stroke": "mod+k"}`): on
//!   iOS the scene's menu declares them, for the iPad's ⌘-hold list and the
//!   Mac's menu bar, and UIKit runs a menu's command before it delivers the
//!   press, so the host forwards them here.
//!
//! The two must not both run for one press. On a Mac (Catalyst) the menu
//! bar runs its shortcut whatever a key handler did with the press, so the
//! page binds none of the keys the menu owns there. Elsewhere a press the
//! menu took never reaches the keymap; should both arrive anyway, the
//! second of the same keystroke within [`TWIN`] is dropped.

use std::{cell::Cell, rc::Rc, time::Duration};

use nsh_core::prelude::*;

use super::{Ctx, Intent};

/// The keystrokes, as NSH's keymap writes them (`mod` is ⌘ on Apple
/// platforms, Control elsewhere).
pub const STROKES: [&str; 11] = [
    "mod+k", "mod+w", "mod+1", "mod+2", "mod+3", "mod+4", "mod+5", "mod+6", "mod+7", "mod+8",
    "mod+9",
];

/// Two arrivals of one keystroke closer than this are one press.
pub const TWIN: Duration = Duration::from_millis(150);

/// Where a keystroke came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// NSH's keymap.
    Keymap,
    /// The host's key commands.
    Host,
}

/// What a keystroke does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Asks the host.
    Ask(Intent),
    /// Opens the pane overview, or closes it (the page's own).
    Overview,
}

/// What a keystroke does, on a screen where `tree` says whether the
/// multiplexer's tree is live (its windows and panes are reachable).
pub fn action(stroke: &str, tree: bool) -> Option<Action> {
    match stroke {
        // A plain shell has no panes to show: nothing, as the crumbs of a
        // tree coming back are inert.
        "mod+k" => tree.then_some(Action::Overview),
        "mod+w" => Some(Action::Ask(Intent::Back)),
        _ => {
            let number = stroke.strip_prefix("mod+")?.parse::<u8>().ok()?;
            ((1..=9).contains(&number) && tree).then_some(Action::Ask(Intent::Window(number)))
        }
    }
}

/// The keystrokes the page binds on this platform: none the Mac's menu bar
/// runs by itself, and none where `mod` is Control rather than ⌘
/// (Android): ^W, ^K and ^1–^9 are the shell's and the terminal's keys
/// there, never the screen's.
pub fn bound(catalyst: bool, apple: bool) -> Vec<&'static str> {
    if catalyst || !apple {
        Vec::new()
    } else {
        STROKES.to_vec()
    }
}

/// A keystroke run: what, which way, when.
type Run = (&'static str, Via, std::time::Instant);

/// What remembers the last keystroke run, to drop its twin.
#[derive(Clone, Default)]
pub struct Shortcuts {
    last: Rc<Cell<Option<Run>>>,
}

impl Shortcuts {
    /// Whether `stroke` arriving `via` now runs (not the twin of the last).
    pub fn admit(&self, stroke: &'static str, via: Via, now: std::time::Instant) -> bool {
        if let Some((last, last_via, at)) = self.last.get()
            && last == stroke
            && last_via != via
            && now.saturating_duration_since(at) < TWIN
        {
            return false;
        }
        self.last.set(Some((stroke, via, now)));
        true
    }

    /// Runs `stroke` from `via`: tells the host what it asks.
    pub(crate) fn run(&self, ctx: &Ctx, stroke: &str, via: Via) {
        let Some(stroke) = STROKES.iter().copied().find(|s| *s == stroke) else {
            return;
        };
        // The engine's clock (the system's on a device, a headless run's own
        // in a test), so "the same press" means the same here and there.
        if !self.admit(stroke, via, nsh_core::time::now()) {
            return;
        }
        let tree = ctx
            .chrome
            .with_untracked(|c| c.crumbs.as_ref().is_some_and(|c| c.live));
        match action(stroke, tree) {
            Some(Action::Ask(intent)) => ctx.ask(intent),
            // The overview needs the session's tree; before it has come,
            // the windows list, as ⌘K always opened.
            Some(Action::Overview) if ctx.switcher.with_untracked(|s| s.tree.is_some()) => {
                ctx.overview.update(|open| *open = !*open)
            }
            Some(Action::Overview) => ctx.ask(Intent::Crumb(super::Crumb::Window)),
            None => {}
        }
    }
}

/// Binds the screen's keystrokes while it lives.
pub(crate) fn bind(ctx: &Ctx, shortcuts: &Shortcuts) {
    for stroke in bound(is_catalyst(), apple_keys()) {
        let (ctx, shortcuts) = (ctx.clone(), shortcuts.clone());
        on_key(stroke, move |s: &str| shortcuts.run(&ctx, s, Via::Keymap));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_keystroke_does_what_the_menu_says() {
        assert_eq!(
            action("mod+k", true),
            Some(Action::Overview),
            "⌘K: the pane overview, not the windows list"
        );
        assert_eq!(action("mod+k", false), None, "no panes to show");
        assert_eq!(action("mod+w", false), Some(Action::Ask(Intent::Back)));
        assert_eq!(action("mod+3", true), Some(Action::Ask(Intent::Window(3))));
        assert_eq!(action("mod+3", false), None);
        assert_eq!(action("mod+0", true), None);
        assert_eq!(action("mod+x", true), None);
    }

    #[test]
    fn the_mac_binds_none_its_menu_bar_runs_and_control_keys_stay_the_shells() {
        assert!(bound(true, true).is_empty());
        assert!(bound(false, false).is_empty(), "Android: ^W is the shell's");
        assert_eq!(bound(false, true).len(), 11);
    }

    #[test]
    fn one_press_runs_once_whichever_way_comes_first() {
        let shortcuts = Shortcuts::default();
        let t = Instant::now();
        assert!(shortcuts.admit("mod+k", Via::Host, t));
        assert!(!shortcuts.admit("mod+k", Via::Keymap, t + Duration::from_millis(5)));
        // A second press, later: runs.
        assert!(shortcuts.admit("mod+k", Via::Keymap, t + Duration::from_millis(400)));
        // Two presses on the same way are two presses, however quick.
        assert!(shortcuts.admit("mod+k", Via::Keymap, t + Duration::from_millis(420)));
        // Another key is another press.
        assert!(shortcuts.admit("mod+1", Via::Host, t + Duration::from_millis(425)));
    }
}
