import Foundation

// Pure value-type tmux model. No SwiftUI / UIKit / SwiftTerm / Citadel deps —
// this header is shared between the parser, the controller, and any future
// snapshotting / persistence code, and must stay framework-free.
//
// Identifiers use the same string representation tmux itself prints in
// control-mode notifications:
//   * sessionId  →  "$0", "$1", ...
//   * windowId   →  "@0", "@1", ...
//   * paneId     →  "%0", "%1", ...
//
// We keep them as `String` rather than a tagged newtype so they can flow
// through `%output`, `list-panes` output, callbacks, and SwiftUI `id:` slots
// without translation. Equality / hashing fall out of `String`.

// MARK: - PaneInfo

/// A single tmux pane within a window.
struct PaneInfo: Codable, Sendable, Equatable, Identifiable, Hashable {
    /// Pane identifier in tmux format, e.g. `"%0"`.
    let id: String
    /// Parent window identifier, e.g. `"@0"`.
    var windowId: String
    /// Pane index within its window — the number `display-panes` shows and
    /// `⌃b q <n>` jumps to (respects pane-base-index).
    var index: Int
    /// Current process running in the pane (e.g. `"bash"`, `"vim"`). Falls
    /// back to an empty string when tmux has nothing to report.
    var command: String
    /// Pane width in cells.
    var width: Int
    /// Pane height in cells.
    var height: Int
    /// Whether this pane is the active pane within its window.
    var isActive: Bool
    /// The working directory of what runs in the pane (tmux's
    /// `#{pane_current_path}`, herdr's pane cwd); nil when unknown. Only the
    /// Rust core's controllers fill it.
    var path: String? = nil

    /// What to call the foreground process. tmux reports the comm name, and
    /// Claude Code's native install runs a binary named after its version
    /// (`~/.local/share/claude/versions/2.1.269`), so the comm arrives as
    /// "2.1.269" — which no icon map and no human recognizes. Anything
    /// containing "claude" is claude, and a bare semver comm is claude too
    /// (nothing else in a terminal names its process a version string). The
    /// Home tree and the terminal breadcrumb both read this, so a pane is
    /// called the same thing wherever it appears.
    var displayCommand: String {
        let lowered = command.lowercased()
        if lowered.contains("claude") { return "claude" }
        if lowered.range(of: #"^\d+\.\d+\.\d+$"#, options: .regularExpression) != nil {
            return "claude"
        }
        return command
    }

    init(
        id: String,
        windowId: String,
        index: Int = 0,
        command: String = "",
        width: Int = 80,
        height: Int = 24,
        isActive: Bool = false,
        path: String? = nil
    ) {
        self.id = id
        self.windowId = windowId
        self.index = index
        self.command = command
        self.width = width
        self.height = height
        self.isActive = isActive
        self.path = path
    }
}

// MARK: - WindowInfo

/// A tmux window. Layout is kept verbatim from `list-windows` /
/// `%layout-change`; a layout parser may consume it later.
struct WindowInfo: Codable, Sendable, Equatable, Identifiable, Hashable {
    /// Window identifier, e.g. `"@0"`.
    let id: String
    /// Owning session id, e.g. `"$0"`. Lets the tree group windows under their
    /// session so several sessions can be expanded at once (`list-windows -a`).
    var sessionId: String
    /// Display name (often the foreground command).
    var name: String
    /// Window index within its session (0-based).
    var index: Int
    /// Raw layout descriptor string from tmux (e.g. `"a]180x45,0,0,0"`).
    var layout: String
    /// Whether this is the active window in its session.
    var isActive: Bool
    /// Number of panes the window currently contains.
    var paneCount: Int
    /// Whether the active pane is zoomed to fill the window (`*Z` in
    /// %layout-change). Drives the mosh-mode "selected pane fills the
    /// phone screen" behaviour.
    var isZoomed: Bool

    init(
        id: String,
        sessionId: String = "",
        name: String = "",
        index: Int = 0,
        layout: String = "",
        isActive: Bool = false,
        paneCount: Int = 1,
        isZoomed: Bool = false
    ) {
        self.id = id
        self.sessionId = sessionId
        self.name = name
        self.index = index
        self.layout = layout
        self.isActive = isActive
        self.paneCount = paneCount
        self.isZoomed = isZoomed
    }
}

// MARK: - SessionInfo

/// A tmux session.
struct SessionInfo: Codable, Sendable, Equatable, Identifiable, Hashable {
    /// Session identifier, e.g. `"$0"`.
    let id: String
    /// Display name (e.g. `"moshpit"`).
    var name: String
    /// Whether this session is currently attached.
    var isAttached: Bool

    init(id: String, name: String = "", isAttached: Bool = false) {
        self.id = id
        self.name = name
        self.isAttached = isAttached
    }
}

// MARK: - Ids

enum TmuxId {
    /// Whether `id` has tmux's own shape (`%3`, `@12`, `$0`): checked before
    /// any id read off the wire is stored or spliced into a command.
    static func isValid(_ id: String) -> Bool {
        guard let first = id.first, first == "%" || first == "@" || first == "$" else { return false }
        let digits = id.dropFirst()
        return !digits.isEmpty && digits.allSatisfy { $0.isASCII && $0.isNumber }
    }
}

// MARK: - AgentHook

/// One pane's raw `@moshpit_*` hook stamp, as the core reads it off tmux
/// (and herdr's agent states, in the same shape). Deliberately
/// framework-free and string-typed: the mapping to
/// `AgentActivityAttributes.AgentState` is the monitor's job, and an unset
/// tmux user option reads back as an empty string, normalised to `nil`.
///
///   - `state`: `"working"` | `"attention"` | `"done"`, or `nil` when the pane
///     has no precise hook data (the monitor falls back to the output
///     heuristic for that pane).
///   - `agent`: `@moshpit_agent`, e.g. `"claude"` / `"codex"` (the row label).
///   - `since`: `@moshpit_since` as a `Date`, or `nil` (the caller uses `Date()`).
///   - `command`: what is RUNNING in the pane now, polled fresh every round:
///     a killed agent fires no final hook, so the monitor spots stamps whose
///     agent has died by asking what the pane is doing now.
///   - `title`: optional `@moshpit_title` headline.
struct AgentHook: Equatable, Sendable {
    var state: String?
    var agent: String?
    var since: Date?
    var command: String?
    var title: String?
}

// MARK: - The multiplexer's words (E7c)

extension Multiplexer {
    /// The multiplexer's words in the app's language, the screens' one list
    /// (`ui/src/terminal/picker.rs` `Vocab`, through the core): herdr's
    /// workspace, tab and Close; tmux's session, window and Kill (a plain
    /// shell's too, never shown).
    var words: MultiplexerWords {
        multiplexerWords(herdr: self == .herdr, locale: NSHScreenParams.locale)
    }
}

extension WindowInfo {
    /// The row/crumb title for a window, saying each thing once.
    ///
    /// tmux names windows after their command, so "2: logs" carries two facts.
    /// herdr names tabs after their own number by default, and "1: 1" is the
    /// same fact twice — a tree of five of those is a wall of digits. Those
    /// (and the pathological empty name) render as the multiplexer's own word:
    /// "Tab 1" / "Window 1".
    func displayTitle(_ words: MultiplexerWords) -> String {
        if name.isEmpty || name == String(index) {
            return "\(words.window) \(index)"
        }
        return "\(index): \(name)"
    }
}

extension TmuxSnapshot {
    /// A session row's label, disambiguated when the name alone isn't one.
    ///
    /// herdr labels a workspace after its cwd, so four workspaces sitting at
    /// `~` all read "~" — identical rows with no way to tell which is which
    /// short of tapping through. When a name is shared, the raw id ("w4") is
    /// appended: it's what `herdr` CLI addressing uses, so it's a handle, not
    /// decoration. tmux never hits this branch — it refuses duplicate session
    /// names outright.
    func sessionDisplayName(_ session: SessionInfo) -> String {
        let twins = sessions.values.lazy.filter { $0.name == session.name }
        guard twins.count > 1 else { return session.name }
        return "\(session.name) · \(session.id)"
    }
}
