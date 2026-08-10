import Foundation

/// What a multiplexer calls its own concepts, and which keys its prefix mode
/// actually binds.
///
/// Moshpit keeps ONE data model (`TmuxSnapshot`) for both multiplexers, which
/// is what let herdr reuse every sheet. But reusing the model is not a reason
/// to reuse tmux's *words*: a herdr user has workspaces and tabs, and reading
/// "Kill Window" for something herdr calls a tab is a small tax on every
/// glance.
///
/// The keyboard hints are not cosmetic at all. They were tmux's under herdr
/// too, and the two disagree in a way that bites: **`⌃ b q` lists panes in
/// tmux and DETACHES in herdr** — the Select Pane sheet was printing the key
/// that drops the user's session. Same story for `⌃ b s` (unbound in herdr;
/// workspaces are `⌃ b w`) and `⌃ 1-9` (herdr steps with `n`/`p`).
struct MultiplexerVocabulary: Equatable {
    /// Top level — tmux session, herdr workspace.
    let session: String
    let sessionPlural: String
    /// Middle level — tmux window, herdr tab.
    let window: String
    let windowPlural: String

    /// Keys the sheet footers print, as `["⌃", "b s"]` style pairs.
    let sessionKeys: [String]
    let windowKeys: [String]
    let paneKeys: [String]
    /// What the pane sheet's `+` does, spelled in this multiplexer's keys.
    let splitHint: String
    /// The verb for destroying one of these. tmux kills; herdr closes
    /// (`workspace close` / `tab close` / `pane close`). "Kill Workspace" is
    /// a tmux idiom stapled to a herdr noun.
    let killVerb: String

    /// SF Symbols for the two levels. Small thing, but `macwindow` — literal
    /// window chrome — sitting next to the word "Tab" is the kind of detail
    /// that tells a herdr user this app was built for something else.
    let sessionIcon: String
    let windowIcon: String

    static let tmux = MultiplexerVocabulary(
        session: String(localized: "Session"),
        sessionPlural: String(localized: "Sessions"),
        window: String(localized: "Window"),
        windowPlural: String(localized: "Windows"),
        sessionKeys: ["⌃", "b s"],
        windowKeys: ["⌃", "1-9"],
        // tmux: display-panes.
        paneKeys: ["⌃", "b q"],
        splitHint: String(localized: "＋ splits a new pane"),
        killVerb: String(localized: "Kill"),
        sessionIcon: "square.stack.3d.up",
        windowIcon: "macwindow")

    static let herdr = MultiplexerVocabulary(
        session: String(localized: "Workspace"),
        sessionPlural: String(localized: "Workspaces"),
        window: String(localized: "Tab"),
        windowPlural: String(localized: "Tabs"),
        // herdr: workspace navigation.
        sessionKeys: ["⌃", "b w"],
        // herdr steps through tabs rather than jumping by number.
        windowKeys: ["⌃", "b n/p"],
        // herdr has no "show me the panes" key; the useful one is the split.
        // NOT `b q`, which detaches.
        paneKeys: ["⌃", "b v"],
        splitHint: String(localized: "＋ splits a new pane"),
        killVerb: String(localized: "Close"),
        // A workspace is a board of work, not a stack of terminals; a tab is
        // a tab.
        sessionIcon: "rectangle.grid.2x2",
        windowIcon: "square.on.square")

    /// Plain-shell sessions never show these sheets; the tmux wording is a
    /// harmless default for any code that asks anyway.
    static let none = tmux
}

extension Multiplexer {
    var vocabulary: MultiplexerVocabulary {
        switch self {
        case .none:  return .none
        case .tmux:  return .tmux
        case .herdr: return .herdr
        }
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
    func displayTitle(_ vocab: MultiplexerVocabulary) -> String {
        if name.isEmpty || name == String(index) {
            return "\(vocab.window) \(index)"
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
