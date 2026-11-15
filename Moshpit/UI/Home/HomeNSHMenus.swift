import SwiftUI
import UIKit

/// The NSH Home's long-press menus: UIKit's own context menu on the
/// surface (``NSHContextMenus``), for the row under the finger: the head's,
/// a session's, a window's, a pane's actions. The screen marks each row with its
/// identifier (`home-head-<connection>`, `home-session-<connection>-<id>`,
/// …); the menus are built from the cards as Home's record has them and
/// their sessions' trees as they are when it last changed, and what they
/// pick goes the way the row's own intents go.
@MainActor
enum HomeNSHMenus {
    /// A card as the menus need it: where its session is, as Home's record
    /// says, and its multiplexer's tree.
    struct Row: Equatable {
        var id: UUID
        var link: HomeLink
        var tree: Tree?
    }

    /// A session's multiplexer and its tree: the ids the rows are marked
    /// with, and each window's own active pane (which Home's record
    /// doesn't carry).
    struct Tree: Equatable {
        var mux: Multiplexer
        var snapshot: TmuxSnapshot
        /// The sessions that are a worktree's (herdr's, made for a task).
        var worktrees: Set<String> = []
    }

    /// The menus of every row of `rows`, by the row's identifier, each
    /// calling `perform` with what is picked.
    static func menus(for rows: [Row],
                      perform: @escaping (HomeIntent) -> Void) -> [String: () -> [UIMenuElement]] {
        var menus: [String: () -> [UIMenuElement]] = [:]
        for card in rows {
            let id = card.id
            menus["home-head-\(id.uuidString)"] = { head(card, perform: perform) }
            guard let control = card.tree, card.link == .live else { continue }
            let vocab = (control.mux == .herdr ? Multiplexer.herdr : .tmux).vocabulary
            for session in control.snapshot.sessions.values {
                menus["home-session-\(id.uuidString)-\(session.id)"] = {
                    var items: [UIMenuElement] = [
                        action(String(localized: "Rename"), "pencil") { perform(.renameSession(id, session.id)) },
                        action(String(localized: "New \(vocab.window)"), "plus.rectangle") {
                            perform(.newWindow(id, session.id))
                        },
                    ]
                    if control.worktrees.contains(session.id) {
                        items.append(action(String(localized: "Remove Worktree"), "trash.slash",
                                            destructive: true) { perform(.removeWorktree(id, session.id)) })
                    }
                    items.append(action(String(localized: "\(vocab.killVerb) \(vocab.session)"), "xmark.circle",
                                        destructive: true) { perform(.killSession(id, session.id)) })
                    return items
                }
            }
            for window in control.snapshot.windows.values {
                menus["home-window-\(id.uuidString)-\(window.id)"] = {
                    var items: [UIMenuElement] = [
                        action(String(localized: "Rename"), "pencil") { perform(.renameWindow(id, window.id)) },
                        action(String(localized: "New Pane"), "rectangle.split.2x1") {
                            perform(.newPane(id, window.id))
                        },
                        action(String(localized: "\(vocab.killVerb) \(vocab.window)"), "xmark.circle", destructive: true) {
                            perform(.killWindow(id, window.id))
                        },
                    ]
                    // The window's active pane — the one Moshpit shows — can be
                    // killed from here without expanding the window.
                    if let pane = activePane(of: window.id, in: control.snapshot) {
                        items.append(UIMenu(options: .displayInline, children: [
                            action(String(localized: "\(vocab.killVerb) \(String(localized: "Pane"))"), "rectangle.split.2x1",
                                   destructive: true) { perform(.killPane(id, pane.id)) },
                        ]))
                    }
                    return items
                }
            }
            for pane in control.snapshot.panes.values {
                menus["home-pane-\(id.uuidString)-\(pane.id)"] = {
                    [action(String(localized: "\(vocab.killVerb) \(String(localized: "Pane"))"), "rectangle.split.2x1",
                            destructive: true) { perform(.killPane(id, pane.id)) }]
                }
            }
        }
        return menus
    }

    /// How round each row's lifted preview is: a head is its card's top
    /// (`radius-lg`, UI-DESIGN §3.3), a saved host's row and the rest a
    /// row in a card (`radius`).
    static func corners(for rows: [Row]) -> [String: CGFloat] {
        var corners: [String: CGFloat] = [:]
        for card in rows {
            corners["home-head-\(card.id.uuidString)"] = card.link == .saved ? 12 : 20
            guard let control = card.tree else { continue }
            for session in control.snapshot.sessions.keys {
                corners["home-session-\(card.id.uuidString)-\(session)"] = 7
            }
            for window in control.snapshot.windows.keys {
                corners["home-window-\(card.id.uuidString)-\(window)"] = 7
            }
            for pane in control.snapshot.panes.keys {
                corners["home-pane-\(card.id.uuidString)-\(pane)"] = 12
            }
        }
        return corners
    }

    /// The head's actions, as its ⋯ words them (UI-DESIGN §4.8: one action,
    /// one wording in ⋯, the long press and the swipe): Open or Connect,
    /// Edit…, Disconnect while there is a session, and Delete… apart.
    static func head(_ card: Row, perform: @escaping (HomeIntent) -> Void) -> [UIMenuElement] {
        let id = card.id
        let live = card.link != .saved
        var items: [UIMenuElement] = [
            live && card.link != .dead
                ? action(String(localized: "Open"), "terminal") { perform(.enter(id)) }
                : action(String(localized: "Connect"), "terminal") { perform(.connect(id)) },
            action(String(localized: "Edit…"), "pencil") { perform(.edit(id)) },
        ]
        if live {
            items.append(action(String(localized: "Disconnect"), "bolt.slash") { perform(.disconnect(id)) })
        }
        items.append(UIMenu(options: .displayInline, children: [
            action(String(localized: "Delete…"), "trash", destructive: true) { perform(.delete(id)) },
        ]))
        return items
    }

    /// The active pane of `window`, else its lowest-indexed one.
    static func activePane(of window: String, in snapshot: TmuxSnapshot) -> PaneInfo? {
        let panes = snapshot.panes.values.filter { $0.windowId == window }
        return panes.first { $0.id == snapshot.activePaneId }
            ?? panes.first(where: \.isActive)
            ?? panes.min { $0.index < $1.index }
    }

    private static func action(_ title: String, _ image: String, destructive: Bool = false,
                               _ handler: @escaping () -> Void) -> UIAction {
        UIAction(title: title, image: UIImage(systemName: image),
                 attributes: destructive ? .destructive : []) { _ in handler() }
    }
}
