import Foundation

/// The shortcut editor's own words in the app's language: the quick keys'
/// group names and descriptions, and what the built-in shortcuts are called
/// (B234). They were English in every language — a built-in's `summary` is
/// stored, and is its identity (``ShortcutStore``'s reconcile matches on
/// it), so it stays English in storage and is put into words only here.
/// The NSH pages say the same (`ui/src/add_shortcut.rs`'s `describe`; the
/// Shortcuts page gets the built-ins' names from the host, through
/// ``TerminalShortcut/displaySummary``).
enum ShortcutCopy {

    /// `english`, a built-in's summary or a quick key's description, in the
    /// app's language (`bundle`'s: a test's names one); anything else as it
    /// is (the user's own words).
    static func describe(_ english: String, bundle: Bundle = .main) -> String {
        switch english {
        // Built-ins (ShortcutStore.builtins).
        case "Escape": String(localized: "Escape", bundle: bundle)
        case "Tab": String(localized: "Tab key", bundle: bundle)
        case "Interrupt (SIGINT)": String(localized: "Interrupt (SIGINT)", bundle: bundle)
        case "Clear screen": String(localized: "Clear screen", bundle: bundle)
        case "Paste clipboard": String(localized: "Paste clipboard", bundle: bundle)
        case "Arrow keys": String(localized: "Arrow keys", bundle: bundle)
        case "Tap arrows": String(localized: "Tap arrows", bundle: bundle)
        case "Control": String(localized: "Control", bundle: bundle)
        case "Scroll history": String(localized: "Scroll history", bundle: bundle)
        case "Voice input": String(localized: "Voice input", bundle: bundle)
        case "Attach image": String(localized: "Attach image", bundle: bundle)
        case "Browse files": String(localized: "Browse files", bundle: bundle)
        case "Return": String(localized: "Return", bundle: bundle)
        case "Accept suggestion and send": String(localized: "Accept suggestion and send", bundle: bundle)
        case "End of file": String(localized: "End of file", bundle: bundle)
        case "Reverse search": String(localized: "Reverse search", bundle: bundle)
        case "Jump to end": String(localized: "Jump to end", bundle: bundle)
        case "Back-tab / toggle mode": String(localized: "Back-tab / toggle mode", bundle: bundle)
        case "History prev": String(localized: "History prev", bundle: bundle)
        case "History next": String(localized: "History next", bundle: bundle)
        case "Cursor left": String(localized: "Cursor left", bundle: bundle)
        case "Cursor right": String(localized: "Cursor right", bundle: bundle)
        case "Clear line before cursor": String(localized: "Clear line before cursor", bundle: bundle)
        case "Clear line after cursor": String(localized: "Clear line after cursor", bundle: bundle)
        case "Beginning of line": String(localized: "Beginning of line", bundle: bundle)
        case "End of line": String(localized: "End of line", bundle: bundle)
        case "Delete word": String(localized: "Delete word", bundle: bundle)
        case "Suspend": String(localized: "Suspend", bundle: bundle)
        case "End": String(localized: "End", bundle: bundle)
        case "Home": String(localized: "Home", bundle: bundle)
        case "Page up": String(localized: "Page up", bundle: bundle)
        case "Page down": String(localized: "Page down", bundle: bundle)
        // Quick keys (AddShortcutView.presetGroups) not named above.
        case "multiplexer prefix": String(localized: "multiplexer prefix", bundle: bundle)
        case "Interrupt": String(localized: "Interrupt", bundle: bundle)
        case "EOF": String(localized: "EOF", bundle: bundle)
        case "Quit": String(localized: "Quit", bundle: bundle)
        case "Clear": String(localized: "Clear", bundle: bundle)
        case "Line start": String(localized: "Line start", bundle: bundle)
        case "Line end": String(localized: "Line end", bundle: bundle)
        case "Search": String(localized: "Search", bundle: bundle)
        case "Del word": String(localized: "Del word", bundle: bundle)
        case "Del line": String(localized: "Del line", bundle: bundle)
        case "Kill EOL": String(localized: "Kill EOL", bundle: bundle)
        case "Yank": String(localized: "Yank", bundle: bundle)
        case "Prev": String(localized: "Prev", bundle: bundle)
        case "Next": String(localized: "Next", bundle: bundle)
        case "Back-tab": String(localized: "Back-tab", bundle: bundle)
        case "Forward delete": String(localized: "Forward delete", bundle: bundle)
        case "Backspace": String(localized: "Backspace", bundle: bundle)
        case "Left": String(localized: "Left", bundle: bundle)
        case "Down": String(localized: "Down", bundle: bundle)
        case "Up": String(localized: "Up", bundle: bundle)
        case "Right": String(localized: "Right", bundle: bundle)
        default: english
        }
    }

    /// A quick keys group's name (its kicker) in the app's language.
    static func group(_ english: String, bundle: Bundle = .main) -> String {
        switch english {
        case "PREFIX": String(localized: "PREFIX", bundle: bundle)
        case "CONTROL": String(localized: "CONTROL", bundle: bundle)
        case "SPECIAL": String(localized: "SPECIAL", bundle: bundle)
        case "NAVIGATION": String(localized: "NAVIGATION", bundle: bundle)
        case "F-KEYS": String(localized: "F-KEYS", bundle: bundle)
        default: english
        }
    }
}

extension TerminalShortcut {
    /// What the editor calls this shortcut: a built-in's name in the app's
    /// language, the user's own words as they wrote them.
    var displaySummary: String {
        isBuiltin ? ShortcutCopy.describe(summary) : summary
    }
}
