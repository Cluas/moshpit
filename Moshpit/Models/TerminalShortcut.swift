import Foundation
import Observation
import UIKit

// MARK: - Shortcut model (prototype screens 6/7)

enum ShortcutKind: String, Codable, CaseIterable {
    case keyCombo
    case text
    case command
    /// The aggregated arrow joystick. Renders as the drag D-pad in the bar
    /// (not a tap chip) and isn't user-creatable — there's one builtin.
    case dpad
    /// Pastes the clipboard into the PTY. A normal, reorderable/removable chip
    /// (resolved by the caller, which reads UIPasteboard) — not user-creatable.
    case paste
    /// A drag thumb that scrolls the terminal's scrollback (local, never sent to
    /// the remote). Renders as a draggable control in the bar, not a tap chip —
    /// a conflict-free alternative to the on-terminal swipe. Not user-creatable.
    case scroll
    /// Arms sticky-Ctrl: the next typed key is sent as its control code
    /// instead of being appended to the bar's action. A normal, reorderable/
    /// removable chip like `.paste` (resolved by the caller, which flips a
    /// flag on the typing coordinator) — not user-creatable, there's one
    /// builtin.
    case ctrl
    /// Starts/stops voice input. A normal, reorderable/removable chip
    /// (resolved by the caller, which drives the dictation controller) — not
    /// user-creatable. It used to be pinned beside the keyboard toggle,
    /// outside the scroll; that permanent slot cost bar width on every
    /// session whether or not dictation was ever used, and made the one
    /// shortcut nobody could reorder or hide out of the way.
    case mic
}

enum ShortcutModifier: String, Codable, CaseIterable, Comparable {
    case ctrl, alt, shift, cmd

    var symbol: String {
        switch self {
        case .ctrl: return "⌃"
        case .alt: return "⌥"
        case .shift: return "⇧"
        case .cmd: return "⌘"
        }
    }

    static func < (lhs: ShortcutModifier, rhs: ShortcutModifier) -> Bool {
        let order: [ShortcutModifier] = [.ctrl, .alt, .shift, .cmd]
        return order.firstIndex(of: lhs)! < order.firstIndex(of: rhs)!
    }
}

/// One entry in the keyboard shortcut toolbar / library.
struct TerminalShortcut: Identifiable, Codable, Equatable, Hashable {
    var id: UUID = UUID()
    var kind: ShortcutKind = .keyCombo
    var modifiers: Set<ShortcutModifier> = []
    /// Main key for `.keyCombo` (single character or named key like "esc").
    var key: String = ""
    /// Bytes sent to the PTY. For `.keyCombo` an optional escape-sequence
    /// override; for `.text` / `.command` the literal string.
    var payload: String = ""
    var appendReturn: Bool = false
    var repeatOnHold: Bool = false
    /// ≤ 6 chars, shown on the toolbar chip.
    var chipLabel: String = ""
    /// Human-readable description for the editor list.
    var summary: String = ""
    /// "gray" | "accent" | "mosh" | "amber"
    var colorId: String = "gray"
    /// Empty = global. Otherwise restricted to these connection names.
    var scopeHosts: Set<String> = []
    var onlyInTmux: Bool = false
    var isBuiltin: Bool = false
    /// Whether the shortcut currently occupies a toolbar slot.
    var inToolbar: Bool = true

    var isCustomStyled: Bool { !isBuiltin || colorId != "gray" }

    /// Bytes to write to the PTY when tapped. `nil` for paste-style actions
    /// resolved by the caller.
    func encodedBytes() -> Data? {
        switch kind {
        case .text:
            var s = Self.unescape(payload)
            if appendReturn { s += "\r" }
            return s.data(using: .utf8)
        case .command:
            return (Self.unescape(payload) + "\r").data(using: .utf8)
        case .keyCombo:
            if !payload.isEmpty {
                var s = Self.unescape(payload)
                if appendReturn { s += "\r" }
                return s.data(using: .utf8)
            }
            return comboBytes()
        case .dpad, .scroll:
            // The joystick / scroll thumb act through their own drag gesture
            // (arrows to the PTY, scrollback locally), not a tap.
            return nil
        case .ctrl:
            // Resolved by the caller (arms sticky-Ctrl on the typing coordinator).
            return nil
        case .paste:
            // Resolved by the caller from UIPasteboard.
            return nil
        case .mic:
            // Resolved by the caller (opens/closes the dictation overlay).
            // Dictation deliberately never writes to the PTY on its own —
            // the transcript goes out on Insert, as one paste.
            return nil
        }
    }

    /// xterm CSI modifier parameter: 1=plain, +1 shift, +2 alt, +4 ctrl.
    private var csiModifier: Int {
        var m = 1
        if modifiers.contains(.shift) { m += 1 }
        if modifiers.contains(.alt) { m += 2 }
        if modifiers.contains(.ctrl) || modifiers.contains(.cmd) { m += 4 }
        return m
    }

    /// CSI letter key (arrows, Home=H, End=F): plain `ESC[X`, modified
    /// `ESC[1;<m>X` — the xterm encoding every TUI (Claude Code, vim, less)
    /// understands, e.g. ⌃End = ESC[1;5F.
    private func csiLetter(_ letter: String) -> Data {
        let m = csiModifier
        return Data((m == 1 ? "\u{1B}[\(letter)" : "\u{1B}[1;\(m)\(letter)").utf8)
    }

    /// CSI tilde key (PgUp=5, PgDn=6): plain `ESC[<n>~`, modified `ESC[<n>;<m>~`.
    private func csiTilde(_ number: Int) -> Data {
        let m = csiModifier
        return Data((m == 1 ? "\u{1B}[\(number)~" : "\u{1B}[\(number);\(m)~").utf8)
    }

    private func comboBytes() -> Data? {
        // Named CSI keys carry their modifiers IN the sequence — return before
        // the legacy alt-prefix path below (it would double-encode alt).
        switch key.lowercased() {
        case "up":    return csiLetter("A")
        case "down":  return csiLetter("B")
        case "right": return csiLetter("C")
        case "left":  return csiLetter("D")
        case "home":  return csiLetter("H")
        case "end":   return csiLetter("F")
        case "pgup", "pageup":   return csiTilde(5)
        case "pgdn", "pagedown": return csiTilde(6)
        default: break
        }

        var data = Data()
        if modifiers.contains(.alt) { data.append(0x1B) }
        switch key.lowercased() {
        case "esc", "escape": data.append(0x1B)
        case "tab":
            // ⇧Tab is its own sequence (back-tab) — Claude Code's mode toggle.
            if modifiers.contains(.shift) { return Data("\u{1B}[Z".utf8) }
            data.append(0x09)
        case "return", "enter": data.append(0x0D)
        case "space": data.append(0x20)
        default:
            guard let scalar = key.lowercased().unicodeScalars.first else { return nil }
            if modifiers.contains(.ctrl) || modifiers.contains(.cmd) {
                // ⌘ on a phone keyboard maps to ctrl semantics in the PTY.
                let value = UInt8(scalar.value & 0x1F)
                data.append(value)
            } else if modifiers.contains(.shift) {
                data.append(contentsOf: Array(key.uppercased().utf8))
            } else {
                data.append(contentsOf: Array(key.utf8))
            }
        }
        return data.isEmpty ? nil : data
    }

    /// Interpret `\r` `\n` `\t` `\e` escapes typed in the payload editor.
    static func unescape(_ s: String) -> String {
        s.replacingOccurrences(of: "\\r", with: "\r")
            .replacingOccurrences(of: "\\n", with: "\n")
            .replacingOccurrences(of: "\\t", with: "\t")
            .replacingOccurrences(of: "\\e", with: "\u{1B}")
    }

    static func chipColor(forId id: String) -> (bg: String, custom: Bool) {
        (id, id != "gray")
    }
}

// MARK: - Store

/// Owns the shortcut toolbar configuration. Array order == toolbar order.
@Observable
final class ShortcutStore {
    static let toolbarLimit = 12
    private static let storageKey = "moshpit.shortcuts.v1"

    private(set) var shortcuts: [TerminalShortcut] = []

    @ObservationIgnored private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        load()
    }

    /// Ordered toolbar entries (≤ 12).
    var toolbar: [TerminalShortcut] {
        Array(shortcuts.filter(\.inToolbar).prefix(Self.toolbarLimit))
    }

    var custom: [TerminalShortcut] { shortcuts.filter { !$0.isBuiltin } }

    /// Built-ins not currently in the toolbar.
    var available: [TerminalShortcut] { shortcuts.filter { $0.isBuiltin && !$0.inToolbar } }

    var toolbarCount: Int { toolbar.count }

    /// Toolbar entries visible for a given connection (scope filter).
    /// `inMultiplexer` covers tmux AND herdr. It was `inTmux`, fed from
    /// `tmuxController != nil`, so a herdr session counted as "not in a
    /// multiplexer" and silently dropped any chip the user had marked — on the
    /// one connection they wanted it on. The stored flag keeps its name for
    /// compatibility with saved shortcuts; only the meaning is corrected.
    func toolbar(forHost hostName: String?, inMultiplexer: Bool) -> [TerminalShortcut] {
        toolbar.filter { sc in
            if sc.onlyInTmux && !inMultiplexer { return false }
            if sc.scopeHosts.isEmpty { return true }
            guard let hostName else { return true }
            return sc.scopeHosts.contains(hostName)
        }
    }

    func add(_ shortcut: TerminalShortcut) {
        var sc = shortcut
        sc.inToolbar = toolbarCount < Self.toolbarLimit
        shortcuts.append(sc)
        persist()
    }

    func update(_ shortcut: TerminalShortcut) {
        guard let i = shortcuts.firstIndex(where: { $0.id == shortcut.id }) else { return }
        shortcuts[i] = shortcut
        persist()
    }

    func remove(id: UUID) {
        guard let i = shortcuts.firstIndex(where: { $0.id == id }) else { return }
        if shortcuts[i].isBuiltin {
            shortcuts[i].inToolbar = false
        } else {
            shortcuts.remove(at: i)
        }
        persist()
    }

    func addToToolbar(id: UUID) {
        guard toolbarCount < Self.toolbarLimit,
              let i = shortcuts.firstIndex(where: { $0.id == id }) else { return }
        shortcuts[i].inToolbar = true
        persist()
    }

    func removeFromToolbar(id: UUID) {
        guard let i = shortcuts.firstIndex(where: { $0.id == id }) else { return }
        shortcuts[i].inToolbar = false
        persist()
    }

    func moveInToolbar(fromOffsets: IndexSet, toOffset: Int) {
        // Map toolbar indices back into the master array by reordering the
        // toolbar slice and rebuilding.
        var bar = toolbar
        bar.move(fromOffsets: fromOffsets, toOffset: toOffset)
        let rest = shortcuts.filter { !$0.inToolbar }
        shortcuts = bar + rest
        persist()
    }

    /// Move `movingId` to just before `targetId`. Operates on the master array
    /// directly (toolbar order == filtered order), which is far more reliable
    /// for drag-and-drop than IndexSet math.
    func moveInToolbar(_ movingId: UUID, before targetId: UUID) {
        guard movingId != targetId,
              let from = shortcuts.firstIndex(where: { $0.id == movingId }) else { return }
        let item = shortcuts.remove(at: from)
        if let target = shortcuts.firstIndex(where: { $0.id == targetId }) {
            shortcuts.insert(item, at: target)
        } else {
            shortcuts.append(item)
        }
        persist()
    }

    // MARK: Persistence

    private func load() {
        if let data = defaults.data(forKey: Self.storageKey),
           let decoded = try? JSONDecoder().decode([TerminalShortcut].self, from: data),
           !decoded.isEmpty {
            shortcuts = decoded
            reconcileBuiltins()
            return
        }
        shortcuts = Self.builtins
        persist()
    }

    /// One-time migrations for built-in shortcuts whose defaults changed
    /// after a user already has a persisted set.
    private func reconcileBuiltins() {
        var changed = false
        // Arrow keys moved to the dedicated D-pad — drop the old ↑/↓ chips.
        for i in shortcuts.indices where shortcuts[i].isBuiltin
            && (shortcuts[i].key == "up" || shortcuts[i].key == "down")
            && shortcuts[i].inToolbar {
            shortcuts[i].inToolbar = false
            changed = true
        }
        // The D-pad is now a builtin shortcut; inject it for users whose set
        // predates it, at the front of the toolbar.
        if !shortcuts.contains(where: { $0.kind == .dpad }),
           let dpad = Self.builtins.first(where: { $0.kind == .dpad }) {
            shortcuts.insert(dpad, at: 0)
            changed = true
        }
        // Sticky-Ctrl became a normal builtin chip; inject it at the very
        // front (ahead of even the D-pad) for users whose set predates it —
        // it was previously a fixed element always rendered ahead of the
        // toolbar, not part of it. Runs AFTER the D-pad injection above so
        // its insert-at-0 wins and lands ctrl ahead of the D-pad, matching
        // ``builtins``' order for a fresh install.
        if !shortcuts.contains(where: { $0.kind == .ctrl }),
           let ctrl = Self.builtins.first(where: { $0.kind == .ctrl }) {
            shortcuts.insert(ctrl, at: 0)
            changed = true
        }
        // Paste became a normal reorderable chip; inject it for older sets.
        if !shortcuts.contains(where: { $0.kind == .paste }),
           let paste = Self.builtins.first(where: { $0.kind == .paste }) {
            shortcuts.append(paste)
            changed = true
        }
        // The mic moved out of its pinned slot beside the keyboard toggle and
        // became a normal chip. Append rather than insert: it lands at the
        // trailing end of the bar, which is where it already appeared, so an
        // upgrade doesn't shuffle the keys under anyone's thumb.
        if !shortcuts.contains(where: { $0.kind == .mic }),
           let mic = Self.builtins.first(where: { $0.kind == .mic }) {
            var injected = mic
            if shortcuts.filter({ $0.inToolbar }).count >= Self.toolbarLimit {
                injected.inToolbar = false
            }
            shortcuts.append(injected)
            changed = true
        }
        // The mic makes the old six-chip default a seventh, which overflows a
        // phone-width bar by about ten points — a half-cut chip rather than a
        // row that reads as scrollable. ^L gives up its slot (it is the only
        // one of those defaults you can also just type: `clear`).
        //
        // Deliberately its OWN step rather than a branch of the mic injection
        // above: written there it would silently skip anyone whose toolbar
        // already grew a mic, and a migration that only fires on one exact
        // upgrade path is a migration that mostly doesn't fire. Guarded on the
        // toolbar still being the untouched default — someone who arranged
        // their own bar keeps every key they put in it, because quietly
        // deleting a shortcut you chose is far worse than a row you can drag.
        let staleDefault = ["Escape", "Tab", "Interrupt (SIGINT)",
                            "Clear screen", "Paste clipboard", "Arrow keys", "Voice input"]
        if toolbar.map(\.summary) == staleDefault,
           let clear = shortcuts.firstIndex(where: { $0.summary == "Clear screen" }) {
            shortcuts[clear].inToolbar = false
            changed = true
        }
        // The scroll thumb is a builtin; inject it for users who predate it,
        // just after the D-pad.
        if !shortcuts.contains(where: { $0.kind == .scroll }),
           let scroll = Self.builtins.first(where: { $0.kind == .scroll }) {
            let at = (shortcuts.firstIndex(where: { $0.kind == .dpad }).map { $0 + 1 }) ?? 0
            shortcuts.insert(scroll, at: min(at, shortcuts.count))
            changed = true
        }
        // Any builtin added AFTER the user's set was persisted gets injected
        // (identity = summary, stable per builtin). Respects the toolbar cap:
        // when the bar is full the new chip lands in the editor's available
        // list instead of silently vanishing.
        let knownSummaries = Set(shortcuts.filter(\.isBuiltin).map(\.summary))
        for builtin in Self.builtins where !knownSummaries.contains(builtin.summary) {
            var injected = builtin
            if injected.inToolbar,
               shortcuts.filter({ $0.inToolbar }).count >= Self.toolbarLimit {
                injected.inToolbar = false
            }
            shortcuts.append(injected)
            changed = true
        }
        if changed { persist() }
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(shortcuts) {
            defaults.set(data, forKey: Self.storageKey)
        }
    }

    /// Default toolbar per prototype screens 3/6.
    static var builtins: [TerminalShortcut] {
        func combo(_ chip: String, _ name: String, mods: Set<ShortcutModifier> = [], key: String,
                   repeats: Bool = false, inBar: Bool = true) -> TerminalShortcut {
            var sc = TerminalShortcut()
            sc.kind = .keyCombo
            sc.modifiers = mods
            sc.key = key
            sc.chipLabel = chip
            sc.summary = name
            sc.repeatOnHold = repeats
            sc.isBuiltin = true
            sc.inToolbar = inBar
            return sc
        }
        func text(_ chip: String, _ name: String, payload: String,
                  inBar: Bool = false) -> TerminalShortcut {
            var sc = TerminalShortcut()
            sc.kind = .text
            sc.payload = payload
            sc.chipLabel = chip
            sc.summary = name
            sc.isBuiltin = true
            sc.inToolbar = inBar
            return sc
        }
        func special(_ kind: ShortcutKind, _ chip: String, _ name: String, inBar: Bool) -> TerminalShortcut {
            var sc = TerminalShortcut()
            sc.kind = kind
            sc.chipLabel = chip
            sc.summary = name
            sc.isBuiltin = true
            sc.inToolbar = inBar
            return sc
        }
        return [
            // The lean default toolbar: the handful of keys actually reached
            // for constantly (interrupt, clear, paste, arrows). Everything
            // else below is a builtin too, just starting outside the bar —
            // add it back from the editor's "available" list if you want it.
            combo("esc", "Escape", key: "esc"),
            combo("tab", "Tab", key: "tab"),
            combo("^C", "Interrupt (SIGINT)", mods: [.ctrl], key: "c"),
            // Out of the bar since the mic joined it: seven chips overflow a
            // phone-width row by about ten points, which reads as a broken
            // half-chip rather than a scrollable row. ^L is the default that
            // gives up its slot most cheaply — it is the only one with a
            // plain-text equivalent you can just type (`clear`), whereas esc,
            // tab and ^C have none on a software keyboard. Still a builtin,
            // one tap away in the shortcut editor.
            combo("^L", "Clear screen", mods: [.ctrl], key: "l", inBar: false),
            // Paste is a normal chip (reorderable / removable), not forced
            // to the end of the bar.
            special(.paste, "paste", "Paste clipboard", inBar: true),
            // The arrow joystick is a first-class shortcut: reorder it, or move
            // it out of the toolbar, like any other.
            special(.dpad, "✛", "Arrow keys", inBar: true),
            // Sticky-Ctrl: reorder it, hide it, or move it out of the toolbar
            // like any other chip — it used to be a fixed, non-configurable
            // element pinned ahead of everything else.
            special(.ctrl, "ctrl", "Control", inBar: false),
            special(.scroll, "⇅", "Scroll history", inBar: false),
            // Voice input: a normal chip like paste, so it can be reordered,
            // moved out of the bar, or scrolled past. Starts in the bar
            // because it was previously always visible — taking it away
            // silently on upgrade would read as the feature disappearing.
            special(.mic, "mic", "Voice input", inBar: true),
            // Return, and the Claude Code two-step. Tab accepts whatever
            // Claude Code is suggesting and Return sends it, which is two taps
            // in the one place you least want them — so `⇥⏎` sends 0x09 0x0D
            // as a single write. A `.text` payload rather than a new kind:
            // `unescape` already turns these into exactly those two bytes, and
            // the PTY delivers them in order.
            //
            // Both start outside the bar. Return is reachable on the software
            // keyboard, and ⇥⏎ only means anything to an agent that suggests
            // prompts — neither earns a default slot on a row that already has
            // to fit on a phone.
            combo("⏎", "Return", key: "return", inBar: false),
            text("⇥⏎", "Accept suggestion and send", payload: #"\t\r"#),
            combo("^D", "End of file", mods: [.ctrl], key: "d", inBar: false),
            combo("^R", "Reverse search", mods: [.ctrl], key: "r", inBar: false),
            // Claude Code daily drivers: jump the transcript to the live end,
            // and toggle the input mode — neither is typeable on the software
            // keyboard (there is no End key, and ⇧Tab needs a hardware Tab).
            combo("⌃End", "Jump to end", mods: [.ctrl], key: "end", inBar: false),
            combo("⇧Tab", "Back-tab / toggle mode", mods: [.shift], key: "tab", inBar: false),
            // Arrow keys live in the dedicated D-pad on the shortcut bar, so
            // they're kept out of the chip row to avoid duplication.
            combo("↑", "History prev", key: "up", repeats: true, inBar: false),
            combo("↓", "History next", key: "down", repeats: true, inBar: false),
            // Bulk clear. Holding backspace is the only other way to empty a
            // long line, and that depends on the OS repeat cadence outrunning a
            // control-mode round trip PER KEYSTROKE (TmuxSessionController
            // .sendInput sends one `send-keys` command per byte, serialized
            // through writeChain) — which it does not on a slow link, and least
            // of all just after a reconnect. ^U is one command whatever the
            // length. Verified against Claude Code 2.1.227: its input handler
            // maps ctrl-u to deleteToLineStart and ctrl-k to deleteToLineEnd,
            // the readline meaning. (Its ctrl+u → scroll:halfPageUp binding is
            // the transcript/settings context, not the prompt.)
            combo("^U", "Clear line before cursor", mods: [.ctrl], key: "u", inBar: false),
            combo("^K", "Clear line after cursor", mods: [.ctrl], key: "k", inBar: false),
            combo("^A", "Beginning of line", mods: [.ctrl], key: "a", inBar: false),
            combo("^E", "End of line", mods: [.ctrl], key: "e", inBar: false),
            combo("^W", "Delete word", mods: [.ctrl], key: "w", inBar: false),
            combo("^Z", "Suspend", mods: [.ctrl], key: "z", inBar: false),
            combo("End", "End", key: "end", inBar: false),
            combo("Home", "Home", key: "home", inBar: false),
            combo("PgUp", "Page up", key: "pgup", repeats: true, inBar: false),
            combo("PgDn", "Page down", key: "pgdn", repeats: true, inBar: false),
        ]
    }
}
