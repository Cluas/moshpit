import Foundation

/// Pure value-type tmux model. No SwiftUI / UIKit / SwiftTerm / Citadel deps —
/// this header is shared between the parser, the controller, and any future
/// snapshotting / persistence code, and must stay framework-free.
///
/// Identifiers use the same string representation tmux itself prints in
/// control-mode notifications:
///   * sessionId  →  "$0", "$1", ...
///   * windowId   →  "@0", "@1", ...
///   * paneId     →  "%0", "%1", ...
///
/// We keep them as `String` rather than a tagged newtype so they can flow
/// through `%output`, `list-panes` output, callbacks, and SwiftUI `id:` slots
/// without translation. Equality / hashing fall out of `String`.

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

    init(
        id: String,
        windowId: String,
        index: Int = 0,
        command: String = "",
        width: Int = 80,
        height: Int = 24,
        isActive: Bool = false
    ) {
        self.id = id
        self.windowId = windowId
        self.index = index
        self.command = command
        self.width = width
        self.height = height
        self.isActive = isActive
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
    /// Display name (e.g. `"moshi"`).
    var name: String
    /// Whether this session is currently attached.
    var isAttached: Bool

    init(id: String, name: String = "", isAttached: Bool = false) {
        self.id = id
        self.name = name
        self.isAttached = isAttached
    }
}
