import Foundation
import Observation
import UIKit

// MARK: - Shortcut model (prototype screens 6/7)
//
// The shortcut bar's rules are the Rust store's (`crates/store/src/shortcuts.rs`,
// through crates/ffi): the built-ins and their reconcile step, the toolbar's
// twelve, the stored form every build since 1.0.8 reads, the sealed-launch
// rule, and the bytes a tap sends. What is here is the value the views
// show (``TerminalShortcut``) and the observable store they watch
// (``ShortcutStore``), which reads and writes through it.

enum ShortcutKind: String, Codable, CaseIterable {
    case keyCombo
    case text
    case command
    /// The aggregated arrow joystick. Renders as the drag D-pad in the bar
    /// (not a tap chip) and isn't user-creatable — there's one builtin.
    ///
    /// Superseded in the default bar by ``arrows``: resolving a direction from
    /// a *push* is what made a single arrow key cost a press-and-drag. Kept a
    /// builtin (out of the bar) because pushing one chip is still the most
    /// compact way to reach four directions, and people who arranged their bar
    /// around it shouldn't lose it.
    case dpad
    /// Four tappable arrow zones in one chip-height control: tap sends one
    /// arrow, hold repeats it. The default arrow control. Renders as its own
    /// cluster rather than a tap chip, and isn't user-creatable — there's one
    /// builtin. Individual `←`/`↑`/`↓`/`→` keyCombo builtins exist too, for a
    /// bar arranged one direction at a time.
    case arrows
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
    /// Attach an image: opens the photo picker, uploads over the session's
    /// SSH channel, and pastes the remote path — how a screenshot reaches a
    /// CLI agent that can only read files. A normal, reorderable/removable
    /// chip (resolved by the caller, which drives the attachment flow) — not
    /// user-creatable.
    case image
    /// Opens the folder sheet at the focused pane's working directory on the
    /// host — where an agent's output lands, a tap away from the pane it
    /// came out of. A normal, reorderable/removable chip (resolved by the
    /// caller, which asks the session for the directory) — not
    /// user-creatable.
    case files

    /// What a stored shortcut says in `kind`: a kind every build since 1.0.8
    /// decodes. A kind newer than that is stored there as an inert key combo
    /// (no key, no payload: a tap in 1.0.8 sends nothing); the real kind goes
    /// beside it in `shortcutKind` (the Rust store's stored form,
    /// `crates/store/src/shortcuts.rs`).
    ///
    /// No `default:` — a kind added later must say here what 1.0.8 gets.
    var olderBuildsName: ShortcutKind {
        switch self {
        case .keyCombo, .text, .command, .dpad, .arrows, .paste, .scroll, .ctrl, .mic, .image:
            return self
        case .files:
            return .keyCombo
        }
    }
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

    /// Display order for a chip's caption — ⌃⌥⇧⌘, the order macOS writes
    /// modifiers in.
    ///
    /// A `switch` rather than the `[ShortcutModifier]` lookup table this
    /// replaces. That table was searched with a force-unwrapped
    /// `firstIndex(of:)`, so a case added to the enum and forgotten in the table
    /// crashed while *sorting* — nowhere near the edit that caused it. Here the
    /// same omission is a compile error.
    private var sortRank: Int {
        switch self {
        case .ctrl: return 0
        case .alt: return 1
        case .shift: return 2
        case .cmd: return 3
        }
    }

    static func < (lhs: ShortcutModifier, rhs: ShortcutModifier) -> Bool {
        lhs.sortRank < rhs.sortRank
    }
}

/// One entry in the keyboard shortcut toolbar / library.
struct TerminalShortcut: Identifiable, Equatable, Hashable {
    var id = UUID()
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

    /// The write(s) a tap makes, in order — the Rust store's rule: normally
    /// one; a payload that is a run of keys (`\t\r`) one write per key, so a
    /// TUI reads two key presses rather than a paste (the "⇥⏎ only did the
    /// tab" report). None for the chips the caller resolves (paste, the
    /// arrows, the mic…).
    func encodedStages() -> [Data] {
        (try? storeShortcutStages(shortcut: ShortcutRecord(self))) ?? []
    }
}

// MARK: - Store

/// The shortcut bar, as the views watch it: the Rust store
/// (`crates/store/src/shortcuts.rs`, through ``ShortcutStoreBacking``) does
/// every read and write — the built-ins and their reconcile step, the
/// toolbar's twelve, the stored form 1.0.8 reads, the sealed-launch rule —
/// and this object keeps what the views observe. Array order == toolbar
/// order.
@Observable
final class ShortcutStore {
    static let toolbarLimit = 12

    private(set) var shortcuts: [TerminalShortcut] = []

    /// True once `shortcuts` came from preferences known to be readable
    /// (the sealed-launch rule): until then the set is shown, never written.
    @ObservationIgnored private(set) var isAuthoritative = false

    @ObservationIgnored private let backing: ShortcutStoreBacking

    /// The Rust store over `defaults`, readable as `protectedData` says (the core's readiness rule) (tests
    /// and scratch suites; the app's is ``AppStores/shortcuts()``).
    convenience init(defaults: UserDefaults = .standard, protectedData: @escaping () -> Bool = { true }) {
        let host = RustPreferencesHost(defaults: defaults, available: protectedData)
        self.init(backing: RustShortcutStore(preferences: host, mode: .system))
    }

    /// A store whose every read and write is `backing`'s.
    init(backing: ShortcutStoreBacking) {
        self.backing = backing
        adopt()
    }

    /// Take what the backing holds now, changing only what changed.
    private func adopt() {
        let held = backing.shortcuts
        if held != shortcuts { shortcuts = held }
        isAuthoritative = backing.isAuthoritative
    }

    /// Read again if the last read could not be trusted; a no-op otherwise.
    func reloadIfNeeded() {
        backing.reloadIfNeeded()
        adopt()
    }

    /// Ordered toolbar entries (≤ 12).
    var toolbar: [TerminalShortcut] {
        Array(shortcuts.filter(\.inToolbar).prefix(Self.toolbarLimit))
    }

    var custom: [TerminalShortcut] { shortcuts.filter { !$0.isBuiltin } }

    /// Every shortcut not on the toolbar as shown, the user's own too (B233).
    var available: [TerminalShortcut] {
        let shown = Set(toolbar.map(\.id))
        return shortcuts.filter { !shown.contains($0.id) }
    }

    var toolbarCount: Int { toolbar.count }

    /// The toolbar holds ``toolbarLimit`` shortcuts: ＋ adds none until one
    /// comes off (B233).
    var toolbarIsFull: Bool { toolbarCount >= Self.toolbarLimit }

    /// Toolbar entries visible for a given connection: the store's scope
    /// rule (a chip limited to hosts shows only there; one marked "only in
    /// tmux" only in a multiplexer, tmux and herdr alike).
    func toolbar(forHost hostName: String?, inMultiplexer: Bool) -> [TerminalShortcut] {
        backing.toolbar(forHost: hostName, inMultiplexer: inMultiplexer)
    }

    /// Adds `shortcut`, on the toolbar while it has room; whether it went
    /// on the toolbar (B252).
    @discardableResult
    func add(_ shortcut: TerminalShortcut) -> Bool {
        let onBar = backing.add(shortcut)
        adopt()
        return onBar
    }

    func update(_ shortcut: TerminalShortcut) {
        backing.update(shortcut)
        adopt()
    }

    /// The user's own goes; a built-in only comes off the bar.
    func remove(id: UUID) {
        backing.remove(id: id)
        adopt()
    }

    /// Put shortcut `id` on the toolbar; false when the bar is full or
    /// there is no such shortcut.
    @discardableResult
    func addToToolbar(id: UUID) -> Bool {
        let added = backing.addToToolbar(id: id)
        adopt()
        return added
    }

    func removeFromToolbar(id: UUID) {
        backing.removeFromToolbar(id: id)
        adopt()
    }

    /// Back to the shipped set (the DEBUG `-MOSHPIT_RESET` launch seam).
    func restoreDefaults() {
        backing.restoreDefaults()
        adopt()
    }

    func moveInToolbar(fromOffsets: IndexSet, toOffset: Int) {
        backing.moveInToolbar(fromOffsets: fromOffsets, toOffset: toOffset)
        adopt()
    }

    /// The shipped set, as the Rust store seeds it.
    static var builtins: [TerminalShortcut] {
        shortcutBuiltins().compactMap(TerminalShortcut.init)
    }
}

/// What a ``ShortcutStore`` reads and writes through: the Rust store
/// (``RustShortcutStore``).
protocol ShortcutStoreBacking: AnyObject {
    var shortcuts: [TerminalShortcut] { get }
    var isAuthoritative: Bool { get }
    func toolbar(forHost hostName: String?, inMultiplexer: Bool) -> [TerminalShortcut]
    func add(_ shortcut: TerminalShortcut) -> Bool
    func update(_ shortcut: TerminalShortcut)
    func remove(id: UUID)
    func addToToolbar(id: UUID) -> Bool
    func removeFromToolbar(id: UUID)
    func restoreDefaults()
    func moveInToolbar(fromOffsets: IndexSet, toOffset: Int)
    func reloadIfNeeded()
}
