import UIKit
import SwiftTerm

/// Restores the multistage-input (IME) candidate bar for SwiftTerm terminals.
///
/// ### Why this exists
/// SwiftTerm's ``TerminalView`` hardcodes its `UITextInputTraits` to the most
/// terminal-friendly values — `autocorrectionType = .no`,
/// `spellCheckingType = .no`, `smartQuotesType/.smartDashesType = .no`. Those
/// keep ASCII command input pristine (no autocorrect mangling `ls`, no smart
/// quotes breaking `'foo'`), but on a **real device** they also collapse the
/// keyboard's prediction/assistant strip — the very region where composed-input
/// keyboards (Chinese Pinyin, Japanese Romaji, …) draw their candidate bar.
/// The result the user hit: typing Pinyin shows *no candidate bar at all*, so
/// there's no way to see or pick what you're composing. (The Simulator's
/// software keyboard always renders its candidate strip, which is why this only
/// bites on hardware.)
///
/// SwiftTerm still has a complete `UITextInput` conformance with full marked
/// text support, so the only thing missing is permission to show the bar. We
/// flip the traits that gate it back on for the terminals Moshpit mints.
///
/// ### What we keep safe
/// Marked-text composition already routes through SwiftTerm's
/// `setMarkedText`/`unmarkText`/`commitTextInput`, which only emit bytes to the
/// remote on commit — re-enabling the candidate bar does not change the byte
/// stream for control keys or tmux. We deliberately leave smart quotes / smart
/// dashes OFF so straight quotes and `--` survive in the shell.
enum TerminalKeyboard {
    /// Re-enable the candidate / prediction bar so composed-input methods
    /// (Pinyin, Japanese, etc.) can show their candidates. Call once, right
    /// after minting a ``TerminalView`` and before it becomes first responder.
    static func enableComposingInput(on terminalView: TerminalView) {
        // The candidate/prediction strip is gated on spell-checking (it lives
        // in the same assistant region). Turning spell-checking back on brings
        // the composed-input candidate bar back without flipping on Latin
        // autocorrect — `autocorrectionType` stays `.no` so the shell never
        // rewrites a command (e.g. `ls` → `last`). Autocapitalization and
        // smart quotes/dashes also stay off so ASCII shell syntax is pristine.
        terminalView.spellCheckingType = .yes
    }
}
