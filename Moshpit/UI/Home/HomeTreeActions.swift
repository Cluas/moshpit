import SwiftUI
import UIKit

/// What the multiplexer tree's asks do — open a window or a pane, refresh,
/// a new session, window, pane or agent task, a rename, a kill (asked
/// first), a worktree's removal — run against the session's multiplexer,
/// with their confirmations and cards. Home's page
/// (``HomeNSHPage``) and a host's sessions page (``SessionsNSHPage``) send
/// the same intents (``HomeIntent``), and both hand them here.
@MainActor
struct HomeTreeActions {
    let store: ConnectionStore
    let hub: SessionHub
    let actions: HomeNSHActions
    /// A kill waiting on its confirmation: the page shows the dialog.
    let confirmKill: (HomeKillTarget) -> Void
    /// A worktree's removal waiting on its confirmation.
    let removeWorktree: (ConnectionCard.WorktreeTarget) -> Void
    /// A new agent task's sheet to show.
    let startAgentTask: (HomeAgentTaskTarget) -> Void

    /// The session of connection `id`, and its multiplexer's control plane.
    func control(_ id: UUID) -> (any MultiplexerControlling)? {
        guard let connection = store.connections.first(where: { $0.id == id }),
              let active = hub.session(for: connection) else { return nil }
        return active.tmuxControl ?? active.herdrControl
    }

    /// The record's cards with their sessions' trees as the controllers
    /// have them now: what the rows' long-press menus are built from
    /// (``HomeNSHMenus``). Compared whole, so a change the record doesn't
    /// carry (a window's own active pane) builds them again too.
    func rows(of record: HomeRecord) -> [HomeNSHMenus.Row] {
        record.cards.map { card in
            HomeNSHMenus.Row(id: card.id, link: card.link, tree: control(card.id).map { control in
                let snapshot = control.snapshot
                return HomeNSHMenus.Tree(
                    mux: control.multiplexer, snapshot: snapshot,
                    worktrees: Set(snapshot.sessions.keys.filter { control.worktreeRepo(for: $0) != nil }))
            })
        }
    }

    /// Does `intent` if it is one of the tree's, and says whether it was.
    func perform(_ intent: HomeIntent) -> Bool {
        switch intent {
        case .window(let id, let window):
            guard let control = control(id) else { return true }
            Haptics.select()
            control.selectWindow(window)
            actions.enter(id)
        case .pane(let id, let pane):
            guard let control = control(id) else { return true }
            Haptics.select()
            control.selectPane(pane)
            actions.enter(id)
        case .refresh(let id):
            guard let control = control(id), !control.isRefreshing else { return true }
            Haptics.tap()
            control.refresh()
        case .newSession(let id):
            guard let control = control(id) else { return true }
            Haptics.tap()
            actions.present(HomeNSHActions.create(noun: control.multiplexer.vocabulary.session) {
                control.newSession(named: $0)
            })
        case .newAgentTask(let id):
            Haptics.tap()
            guard let herdr = control(id) as? RustHerdrController else { return true }
            startAgentTask(HomeAgentTaskTarget(
                initialRepos: herdr.openRepoGuesses,
                load: { await (repos: herdr.gitRepos(), agents: herdr.agentNames()) },
                start: { request in await herdr.startAgentTask(request) }))
        case .restartServer(let id):
            guard let herdr = control(id) as? RustHerdrController else { return true }
            Task { await herdr.restartServer() }
        case .renameSession(let id, let session):
            guard let control = control(id),
                  let info = control.snapshot.sessions[session] else { return true }
            actions.present(HomeNSHActions.rename(
                title: String(localized: "Rename \(control.multiplexer.vocabulary.session)"),
                currentName: info.name) { control.renameSession(session, to: $0) })
        case .killSession(let id, let session):
            guard let control = control(id),
                  let info = control.snapshot.sessions[session] else { return true }
            let vocab = control.multiplexer.vocabulary
            confirmKill(HomeKillTarget(
                label: String(localized: "\(vocab.killVerb) \(vocab.session)"),
                confirmTitle: String(localized: "\(vocab.killVerb) \(vocab.sessionLower) \"\(control.snapshot.sessionDisplayName(info))\"?")
            ) { control.killSession(session) })
        case .newWindow(let id, let session):
            guard let control = control(id) else { return true }
            actions.present(HomeNSHActions.create(noun: control.multiplexer.vocabulary.window) {
                control.newWindow(inSession: session, named: $0)
            })
        case .removeWorktree(let id, let session):
            guard let herdr = control(id) as? RustHerdrController,
                  let info = herdr.snapshot.sessions[session],
                  let repo = herdr.worktreeRepo(for: session) else { return true }
            removeWorktree(ConnectionCard.WorktreeTarget(
                sessionName: info.name, repo: repo,
                remove: { force in await herdr.removeWorktree(sessionId: session, force: force) }))
        case .renameWindow(let id, let window):
            guard let control = control(id), let info = control.snapshot.windows[window] else { return true }
            actions.present(HomeNSHActions.rename(
                title: String(localized: "Rename \(control.multiplexer.vocabulary.window)"),
                currentName: info.name) { control.renameWindow(window, to: $0) })
        case .killWindow(let id, let window):
            guard let control = control(id), let info = control.snapshot.windows[window] else { return true }
            let vocab = control.multiplexer.vocabulary
            confirmKill(HomeKillTarget(
                label: String(localized: "\(vocab.killVerb) \(vocab.window)"),
                confirmTitle: String(localized: "\(vocab.killVerb) \(vocab.windowLower) \"\(info.displayTitle(vocab))\"?")
            ) { control.killWindow(window) })
        case .newPane(let id, let window):
            guard let control = control(id) else { return true }
            Haptics.tap()
            control.newPane(inWindow: window)
        case .killPane(let id, let pane):
            guard let control = control(id), let info = control.snapshot.panes[pane] else { return true }
            let vocab = control.multiplexer.vocabulary
            confirmKill(HomeKillTarget(
                label: String(localized: "\(vocab.killVerb) \(String(localized: "Pane"))"),
                confirmTitle: String(localized: "\(vocab.killVerb) pane \(info.index)?")
            ) { control.killPane(pane) })
        default:
            return false
        }
        return true
    }
}

/// The tree's confirmations and sheets, on whichever page asked: a kill's
/// dialog, a worktree's removal, a new agent task.
///
/// The confirmations hang from the row whose menu asked (B498,
/// ``NSHConfirmation``): a SwiftUI `confirmationDialog` on the page hung
/// from the page, which from iOS 26 put an iPhone's popover near the top of
/// the screen, over the toolbar, pointing at nothing.
struct HomeTreeDialogs: ViewModifier {
    @Binding var killTarget: HomeKillTarget?
    @Binding var worktreeTarget: ConnectionCard.WorktreeTarget?
    @Binding var worktreeForceTarget: ConnectionCard.WorktreeTarget?
    @Binding var agentTask: HomeAgentTaskTarget?
    /// The page's long-press menus: the row the last one opened on, and the
    /// surface.
    let menus: NSHContextMenus
    let actions: HomeNSHActions

    func body(content: Content) -> some View {
        content
            .onChange(of: killTarget?.id) { _, id in
                guard id != nil, let target = killTarget else { return }
                confirmKill(target)
            }
            .onChange(of: worktreeTarget?.id) { _, id in
                guard id != nil, let target = worktreeTarget else { return }
                confirmRemoval(target)
            }
            .onChange(of: worktreeForceTarget?.id) { _, id in
                guard id != nil, let target = worktreeForceTarget else { return }
                confirmForce(target)
            }
            .appSheet(item: $agentTask) { target in
                AgentTaskNSHPage(initialRepos: target.initialRepos,
                                 load: target.load, start: target.start)
            }
    }

    private func confirmKill(_ target: HomeKillTarget) {
        let shown = NSHConfirmation.present(
            title: target.confirmTitle,
            actions: [.init(title: target.label, destructive: true) {
                target.perform()
                if killTarget?.id == target.id { killTarget = nil }
            }],
            from: menus.openedView, over: menus.surfaceView,
            cancel: { if killTarget?.id == target.id { killTarget = nil } })
        if !shown { killTarget = nil }
    }

    // A worktree's removal: two stages on purpose: the second appears ONLY
    // when herdr reports the checkout is dirty, the one case where this
    // destroys work that exists nowhere else.

    private func confirmRemoval(_ target: ConnectionCard.WorktreeTarget) {
        let shown = NSHConfirmation.present(
            title: String(localized: "Remove the worktree for \"\(target.sessionName)\"?"),
            message: String(localized: "Deletes the branch checkout under ~/.herdr/worktrees. \(target.repo) itself is untouched."),
            actions: [.init(title: String(localized: "Remove"), destructive: true) { remove(target, force: false) }],
            from: menus.openedView, over: menus.surfaceView,
            cancel: { if worktreeTarget?.id == target.id { worktreeTarget = nil } })
        if !shown { worktreeTarget = nil }
    }

    private func confirmForce(_ target: ConnectionCard.WorktreeTarget) {
        let shown = NSHConfirmation.present(
            title: String(localized: "\"\(target.sessionName)\" has uncommitted changes"),
            message: String(localized: "Those changes exist nowhere else. Removing the worktree throws them away."),
            actions: [.init(title: String(localized: "Delete anyway"), destructive: true) { remove(target, force: true) }],
            cancelTitle: String(localized: "Keep it"),
            from: menus.openedView, over: menus.surfaceView,
            cancel: { if worktreeForceTarget?.id == target.id { worktreeForceTarget = nil } })
        if !shown { worktreeForceTarget = nil }
    }

    private func remove(_ item: ConnectionCard.WorktreeTarget, force: Bool) {
        worktreeTarget = nil
        worktreeForceTarget = nil
        Task {
            switch await item.remove(force) {
            case .removed:
                break
            case .needsForce:
                worktreeForceTarget = item
            case .failed(let message):
                actions.present(ConnectionCard.Modal(kind: .notice(ConnectionCard.NoticeRequest(
                    title: "Couldn't remove the worktree",
                    message: message))))
            }
        }
    }
}
