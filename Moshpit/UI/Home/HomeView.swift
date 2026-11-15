import MoshpitKit
import SwiftUI

/// Home command board. Saved hosts stay scannable offline; live hosts expand
/// into the tmux SESSIONS tree without changing the underlying SessionHub flow.
struct HomeView: View {
    @Environment(AppSettings.self) private var settings
    @Environment(ThemeStore.self) private var themes
    @Environment(AgentActivityMonitor.self) private var monitor
    @Environment(DeepLinkRouter.self) private var router
    /// The key records: a deleted connection's keychain item goes only once
    /// no key names it either (``ConnectionDeletion``).
    @Environment(SSHKeyStore.self) private var keyStore

    let store: ConnectionStore
    let hub: SessionHub
    let keychain: KeychainService

    #if DEBUG
    // A capture run names the screen it wants (see CaptureScreen); deriving the
    // initial value means the sheet is up from the first render rather than
    // depending on when a .task happens to fire.
    @State private var showSettings = CaptureScreen.requested.map { !CaptureScreen.onHome.contains($0) } ?? false
    @State private var showAddConnection = CaptureScreen.requested == .addConnection
    #else
    @State private var showSettings = false
    @State private var showAddConnection = false
    #endif
    @State private var editingConnection: ServerConnection?
    @State private var deletingConnection: ServerConnection?
    /// Cards whose disconnect teardown is in flight — drives the per-card
    /// spinner and guards against double-taps during the ~2s channel flush.
    @State private var disconnectingIds: Set<UUID> = []
    /// The pages pushed on Home: a connection's terminal, or a host's
    /// sessions (§6.2) with the terminal its rows open on top of it.
    @State private var path: [HomeRoute] = []
    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var splitIsPortrait = false
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    /// Sessions whose handshakes are in flight — a QUEUE, not a single slot:
    /// connecting a second host used to overwrite the first, leaving its
    /// host-key TOFU prompt unobserved → that handshake deadlocked silently.
    /// Prompts and errors surface one at a time, oldest first.
    @State private var connecting: [SessionHub.ActiveSession] = []
    @State private var showConnectError = false
    /// The modal card a connection card asked Home to draw — a rename, a
    /// name for something about to be created, a notice. The card presenter
    /// is an overlay sized to the view it hangs off, so it has to hang off
    /// the whole screen: a List row would clip it to the row.
    @State private var cardModal: ConnectionCard.Modal?
    @State private var cardModalText = ""
    /// The NSH list's model (``HomeNSHPage``), kept here so a page made
    /// again starts from it.
    @State private var nshModel = HomeNSHModel()
    /// The NSH list could not run: the page says so.
    @State private var nshFailed = false

    private var theme: TerminalTheme {
        themes.theme(id: settings.themeId)
    }

    /// Oldest in-flight session with a pending host-key prompt / error — the
    /// one the (single) alert presents. The next surfaces when this resolves.
    private var promptingSession: SessionHub.ActiveSession? {
        asking.first { $0.viewModel.hostKeyPrompt != nil }
    }
    /// Oldest in-flight session waiting for its key's passphrase.
    private var passphraseSession: SessionHub.ActiveSession? {
        asking.first { $0.viewModel.keyPassphrasePrompt != nil }
    }
    /// Oldest in-flight session the server asked a question at sign-in.
    private var questionSession: SessionHub.ActiveSession? {
        asking.first { $0.viewModel.signInQuestion != nil }
    }
    /// The in-flight sessions whose questions Home asks: not one whose
    /// terminal is open, which asks them itself (two presenters of one
    /// question would race for the screen).
    private var asking: [SessionHub.ActiveSession] {
        connecting.filter { !path.contains(.terminal($0.id)) }
    }
    private var erroredSession: SessionHub.ActiveSession? {
        connecting.first { $0.viewModel.errorMessage?.isEmpty == false }
    }

    /// Connection `id`'s session while it is connected: what its Host Setup
    /// runs over.
    private func liveSession(_ id: UUID) -> SessionHub.ActiveSession? {
        guard let connection = store.connections.first(where: { $0.id == id }),
              let session = hub.session(for: connection),
              case .connected = session.viewModel.status else { return nil }
        return session
    }

    /// The saved connections for Settings › Set up a host, in Home's order,
    /// each connected or not.
    private var settingsHosts: [SettingsHost] {
        store.connections.map { SettingsHost($0, live: liveSession($0.id) != nil) }
    }

    /// + on Home: adds a new connection. No free-tier cap — every feature is
    /// unlocked for everyone.
    private func handleAdd() {
        showAddConnection = true
    }

    /// Start (or resume) a connection in-place. Binds the session to `connecting`
    /// BEFORE start() so the host-key TOFU prompt — which fires mid-handshake
    /// through `viewModel.hostKeyPrompt` — has an observer and can be answered;
    /// otherwise the handshake deadlocks. The card reads `hub.session(for:)`
    /// to show its in-place spinner / sessions tree as state arrives.
    private func connect(_ connection: ServerConnection) {
        // Being deleted: its card is still here for a few seconds (B327).
        guard !ConnectionDeletion.isUnderWay(connection.id) else { return }
        let session = hub.prepare(connection)
        guard !connecting.contains(where: { $0.id == session.id }) else { return }
        connecting.append(session)
        Task {
            await hub.start(session, theme: theme, fontSize: settings.fontSize, fontName: settings.fontName,
                            cursorShape: settings.cursorShape, cursorColorId: settings.cursorColorId,
                            cursorBlink: settings.cursorBlink)
            // When-ready, not right-now: over mosh the control plane is wired
            // by a detached task after start() returns, so a one-shot read
            // here left the island blind on that transport.
            monitor.trackWhenReady(connection: connection, session: session)
            // The buttons the Host Setup sheet used to require, run silently:
            // refresh drifted scripts, mint/repair this device's pairing. Asks
            // (once) before ever touching a host that has no hooks.
            HostCare.shared.tend(connection)
            // Keep failed sessions queued so the error alert (which reads
            // erroredSession) has its message; its OK button dequeues them.
            if session.viewModel.errorMessage == nil {
                connecting.removeAll { $0.id == session.id }
            }
        }
    }

    var body: some View {
        layout
        // The window's appearance (R18): the one chosen in Settings.
        .background(WindowAppearance(style: settings.appearance.interfaceStyle))
        // `initial: true`: a cold launch from a tapped notification can file
        // the request before this view is on screen, and a plain `.onChange`
        // never reports the value it starts with — the tap opened Home.
        // The island's "+N more" (B661): Home itself, at "Needs you".
        .onChange(of: router.needsYouRequest) { _, _ in
            path = []
            nshModel.revealNeedsYou()
        }
        .onChange(of: router.paneRequest, initial: true) { _, request in
            guard let request, store.connections.contains(where: { $0.id == request.connectionId })
            else { return }
            // Only push if we're not already there — reassigning `path` to its
            // current value is a no-op for NavigationStack (this closure won't
            // re-run), which is exactly why TerminalScreen owns the pane
            // selection itself instead of relying on a fresh push here.
            // Deep link jumps straight into the terminal; TerminalScreen.task
            // still owns prepare/start + the host-key prompt for that path
            // (idempotent — re-entry from a card tap won't re-handshake).
            Log.island.info("land: home opens terminal path=\(path.count, privacy: .public)")
            open(request.connectionId)
        }
        .alert("Connection Error", isPresented: $showConnectError) {
            Button("OK") {
                showConnectError = false
                // Drop the dead session so its card returns to the offline
                // state (tappable to retry); the next queued error (if any)
                // surfaces via onChange.
                if let failed = erroredSession {
                    failed.viewModel.errorMessage = nil
                    let id = failed.id
                    connecting.removeAll { $0.id == id }
                    Task { await hub.disconnect(id) }
                }
            }
        } message: {
            Text(erroredSession?.viewModel.errorMessage ?? "")
        }
        // A connection saved while its session runs: the session's next
        // connect uses what was saved (B329).
        .onChange(of: store.connections) { _, saved in
            for connection in saved where hub.sessions[connection.id] != nil {
                hub.adopt(connection)
            }
        }
        .onChange(of: erroredSession?.viewModel.errorMessage) { _, message in
            showConnectError = message?.isEmpty == false
        }
        .alert(cardModal.map(\.title) ?? Text(verbatim: ""),
               isPresented: Binding(get: { cardModal != nil }, set: { if !$0 { cardModal = nil } }),
               presenting: cardModal) { modal in
            cardModalActions(modal)
        } message: { modal in
            modal.message
        }
        .connectPrompts(
            hostKey: { promptingSession?.viewModel.hostKeyPrompt },
            passphrase: { passphraseSession?.viewModel.keyPassphrasePrompt },
            question: { questionSession?.viewModel.signInQuestion })
        .appSheet(isPresented: $showSettings) {
            SettingsScreen(hosts: settingsHosts, session: liveSession, onConnect: { id in
                // Connect from Set up a host: Settings closes, the terminal opens.
                showSettings = false
                open(id)
            })
        }
        .appSheet(isPresented: $showAddConnection) {
            connectionForm(existing: nil)
        }
        .appSheet(item: $editingConnection) { connection in
            connectionForm(existing: connection)
        }
        .confirmationDialog(
            "Delete \(deletingConnection?.displayName ?? String(localized: "connection"))?",
            isPresented: Binding(get: { deletingConnection != nil },
                                 set: { if !$0 { deletingConnection = nil } }),
            titleVisibility: .visible,
            presenting: deletingConnection
        ) { connection in
            Button("Delete", role: .destructive) { delete(connection) }
            Button("Cancel", role: .cancel) { deletingConnection = nil }
        } message: { _ in
            Text("Removes the saved server and its stored credentials from this device.")
        }
    }

    /// The add / edit form (``ConnectionFormNSHPage``).
    private func connectionForm(existing: ServerConnection?) -> some View {
        ConnectionFormNSHPage(store: store, keychain: keychain, existing: existing,
                              onDelete: { delete($0) }, releaseSecret: releaseSecret)
    }

    /// A save left connection `id`'s own keychain item `ref` named by
    /// nothing (it signs in with a managed key now, B325): drop it. While its
    /// session runs, only once that session has connected again as the
    /// connection was saved, or has ended (B329) — until then its
    /// reconnects read the item it started with. A session on the Swift
    /// path keeps it until the app ends. Either way the item is listed to
    /// drop first, so an app that ends before its session lets go drops it
    /// at the next launch, never offers it back as a key (B428).
    private func releaseSecret(_ id: UUID, _ ref: String) async {
        let release = { [store, keyStore, keychain] in
            _ = await KeychainReferences.release(ref, connections: store, keys: keyStore, keychain: keychain)
        }
        if let active = hub.sessions[id] {
            switch active.viewModel.status {
            case .connecting, .connected, .reconnecting:
                KeychainReferences.hold(ref)
                if !hub.releaseAfterReconnect(id, release) {
                    Log.store.info("connection save: its old item stays while its session runs; the next launch drops it")
                }
                return
            default:
                break
            }
        }
        await release()
    }

    /// Forget the connection's push pairing, tear down any live session, drop
    /// the keychain secret, then forget the connection (``ConnectionDeletion``)
    /// — so a deleted server leaves no secrets behind, and no send token the
    /// relay keeps being asked to mint. A pairing the phone can't forget
    /// stops the delete before anything is removed, and Home says so.
    private func delete(_ connection: ServerConnection) {
        Task {
            do {
                try await ConnectionDeletion.delete(
                    connection, from: store, keys: keyStore, care: HostCare.shared,
                    tearDown: {
                        monitor.untrack(connectionId: connection.id)
                        await hub.disconnect(connection.id)
                    },
                    dropSecret: { ref in
                        do {
                            try await keychain.delete(forRef: ref)
                        } catch {
                            // A later launch tries again (B428).
                            Log.store.error("delete: the keychain kept its item (\(String(describing: error), privacy: .public)); listed to drop later")
                            KeychainReferences.hold(ref)
                        }
                    },
                    holdSecret: { KeychainReferences.hold($0) })
                // Its finish cards, now that it is no longer saved (B437).
                monitor.sweepDelivered()
            } catch {
                Log.push.error("delete \(connection.displayName, privacy: .public) stopped: \(error.localizedDescription, privacy: .public)")
                cardModal = ConnectionCard.Modal(kind: .notice(ConnectionCard.NoticeRequest(
                    title: "Couldn't delete the connection",
                    message: String(localized: "Its push pairing couldn't be removed from this phone, so the connection was kept. The diagnostics log has the reason."))))
            }
            deletingConnection = nil
        }
    }

    /// The buttons (and the field) of the alert a row asked for: a rename
    /// or a name-before-create, or a notice. A name the multiplexer would
    /// not take as typed asks again, the reason as its message, the text
    /// kept.
    @ViewBuilder
    private func cardModalActions(_ modal: ConnectionCard.Modal) -> some View {
        switch modal.kind {
        case .input(let input):
            TextField(input.placeholder, text: $cardModalText)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .accessibilityIdentifier("card-input-field")
            Button("Cancel", role: .cancel) { cardModal = nil }
            Button(input.confirmLabel) {
                let typed = cardModalText
                cardModal = nil
                if let refusal = input.refuses?(typed) {
                    var again = input
                    again.message = Text(refusal)
                    again.initialText = typed
                    // Once this alert has gone: one alert at a time.
                    Task { @MainActor in
                        try? await Task.sleep(for: .milliseconds(400))
                        cardModalText = typed
                        cardModal = ConnectionCard.Modal(kind: .input(again))
                    }
                    return
                }
                input.apply(typed)
            }
            .keyboardShortcut(.defaultAction)
        case .notice:
            Button("OK") { cardModal = nil }
        }
    }

    // MARK: Layouts

    /// Its own property, not inline in `body`: the modal chain below is long,
    /// and an `if` at its root was one expression too many for the type
    /// checker.
    @ViewBuilder private var layout: some View {
        if usesSplitLayout { splitLayout } else { stackLayout }
    }

    /// iPad at regular width: the connection list and the terminal side by
    /// side. An iPhone — including a Max in landscape, which is also regular
    /// — keeps the stack: a landscape phone is for a wider terminal, not a
    /// sidebar, and swapping hierarchies on every rotation would re-host the
    /// terminal each time.
    private var usesSplitLayout: Bool {
        UIDevice.current.userInterfaceIdiom == .pad && horizontalSizeClass == .regular
    }

    /// The phone: Home pushes the terminal.
    private var stackLayout: some View {
        NavigationStack(path: $path) {
            homeList
            .navigationDestination(for: HomeRoute.self) { route in
                switch route {
                case .sessions(let id):
                    sessionsPage(id)
                case .terminal(let id):
                    terminalPage(id)
                }
            }
        }
    }

    /// A host's sessions page (``SessionsNSHPage``), on the phone's stack
    /// or in the iPad's detail column.
    @ViewBuilder
    private func sessionsPage(_ id: UUID) -> some View {
        if store.connections.contains(where: { $0.id == id }) {
            SessionsNSHPage(connectionId: id, store: store, hub: hub,
                            model: nshModel, actions: nshActions,
                            onLeave: usesSplitLayout ? { leaveSessions() } : nil)
                .id(id)
        }
    }

    /// A connection's terminal pushed on the phone's stack.
    @ViewBuilder
    private func terminalPage(_ id: UUID) -> some View {
        if let connection = store.connections.first(where: { $0.id == id }) {
            // No pane target passed here — TerminalScreen reads the
            // deep-link pane straight from `router.paneRequest` itself
            // (see its own `.onChange`), which also lets it react to a
            // SECOND request naming a different pane on this same
            // connection while already on screen, when this push is a
            // no-op (`path` unchanged) and this closure never re-runs.
            // Plain push, not `.navigationTransition(.zoom)`: the zoom
            // transition installs its own drag-to-dismiss on the pushed
            // view, and a downward swipe in the terminal — a scroll —
            // popped the whole screen (iOS 26.2 simulator, 2026-09-11).
            // SwiftUI has no way to veto that gesture, so the terminal
            // keeps the standard push.
            //
            // `.id(id)` for the same reason the iPad detail below has
            // one: replacing `[A]` with `[B]` — a notification for
            // host B tapped while A's terminal is up — re-renders the
            // destination in place, so without it the screen kept A's
            // `@State` (its `active` session; `.task` never re-ran) under
            // B's `connection`, claimed B's pane request and selected
            // B's pane id on A's tmux. Pane ids like `%5` exist on
            // every host, so that "worked" — on the wrong machine.
            TerminalScreen(connection: connection, hub: hub)
                .id(id)
        }
    }

    /// The iPad: Home is the sidebar, the terminal the detail. `path` is the
    /// one source of truth for both layouts, so an open terminal survives the
    /// layout changing under it (a Split View resize) — the session lives in
    /// the hub and re-hosting is idempotent.
    private var splitLayout: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            homeList
                // Wider than the stock sidebar: a card carries a name, a
                // transport chip and a user@host, and at 320pt the host wraps.
                .navigationSplitViewColumnWidth(min: 320, ideal: 380, max: 480)
        } detail: {
            // A host deleted from the sidebar takes its page with it, as a
            // terminal's does: the placeholder shows.
            if case .sessions(let id)? = path.last, store.connections.contains(where: { $0.id == id }) {
                sessionsPage(id)
            } else if case .terminal(let id)? = path.last,
               let connection = store.connections.first(where: { $0.id == id }) {
                TerminalScreen(connection: connection, hub: hub,
                               onLeave: { leaveTerminal() },
                               onToggleSidebar: { toggleSidebar() })
                    // A different host is a different screen: without this the
                    // struct is reused and keeps the first host's state.
                    .id(id)
            } else {
                ContentUnavailableView {
                    Label("No Terminal Open", systemImage: "terminal")
                } description: {
                    Text("Pick a host from the sidebar, or add one with ⌘N.")
                }
                .background { HomeStage() }
            }
        }
        .onGeometryChange(for: Bool.self) { $0.size.width < $0.size.height } action: {
            splitIsPortrait = $0
        }
    }

    /// The connection list with its title and toolbar (``HomeNSHPage``) —
    /// the root of the stack on a phone, the sidebar on an iPad. One that
    /// can't run says so (``NSHPageFailed``).
    private var homeList: some View {
        Group {
            if nshFailed {
                NSHPageFailed { nshFailed = false }
            } else {
                HomeNSHPage(store: store, hub: hub, disconnecting: disconnectingIds, sidebar: usesSplitLayout,
                            model: nshModel, actions: nshActions, onFailure: { nshFailed = true })
                    .background { HomeStage() }
            }
        }
        #if DEBUG
        .task { await connectSeeded() }
        #endif
    }

    #if DEBUG
    /// `-MOSHPIT_SEED_HOME_FIXTURE`'s connections that ask to, connected as
    /// a tap on their heads would, once a launch (``HomeSeed``).
    private func connectSeeded() async {
        guard !HomeView.seededConnected else { return }
        HomeView.seededConnected = true
        await HomeSeed.prepare()
        for connection in HomeSeed.autoConnect where hub.session(for: connection) == nil {
            connect(connection)
        }
    }

    private static var seededConnected = false
    #endif

    /// What the NSH list's connection-level asks do.
    private var nshActions: HomeNSHActions {
        let find = { (id: UUID) in store.connections.first { $0.id == id } }
        return HomeNSHActions(
            settings: { showSettings = true },
            add: { handleAdd() },
            toggleSidebar: { toggleSidebar() },
            connect: { id in if let connection = find(id) { connect(connection) } },
            enter: { id in open(id) },
            sessions: { id in openSessions(id) },
            edit: { id in editingConnection = find(id) },
            delete: { id in deletingConnection = find(id) },
            disconnect: { id in if let connection = find(id) { disconnect(connection) } },
            retryAttach: { id in if let connection = find(id) { retryAttach(connection) } },
            present: { modal in
                cardModalText = modal.initialText
                cardModal = modal
            })
    }

    /// Show a connection's terminal: a push on the phone, the detail column
    /// on the iPad. Reassigning `path` to its current value is a no-op for
    /// NavigationStack, so the check is only for clarity.
    private func open(_ id: UUID) {
        if path.last == .sessions(id) {
            path.append(.terminal(id))
        } else if path != [.terminal(id)] {
            path = [.terminal(id)]
        }
        // In portrait the sidebar overlays the terminal; get out of its way.
        if usesSplitLayout && splitIsPortrait { columnVisibility = .detailOnly }
    }

    /// A host's sessions page: pushed on the phone, the detail column on the
    /// iPad (out of the sidebar's way in portrait, as a terminal is).
    private func openSessions(_ id: UUID) {
        path = [.sessions(id)]
        if usesSplitLayout && splitIsPortrait { columnVisibility = .detailOnly }
    }

    /// Back/⌘W from the detail: drop the terminal and bring the list back —
    /// in portrait the sidebar was tucked away on open, and a placeholder
    /// with no way to the hosts is not "home".
    private func leaveTerminal() {
        withAnimation {
            // Back to the host's sessions page the terminal was opened from;
            // else nothing is pushed.
            if case .terminal? = path.last { path.removeLast() }
            switch path.last {
            case .sessions?: break
            default: path = []
            }
            columnVisibility = .all
        }
    }

    /// Back from a host's sessions page in the iPad's detail column: the
    /// placeholder, and the list back (in portrait it was tucked away when
    /// the page opened). B554: there was nothing to pop, and back did
    /// nothing.
    private func leaveSessions() {
        withAnimation {
            path = []
            columnVisibility = .all
        }
    }

    private func toggleSidebar() {
        withAnimation {
            columnVisibility = columnVisibility == .detailOnly ? .all : .detailOnly
        }
    }

    /// Teardown flushes the control channel for up to ~2s — without feedback
    /// the button reads as broken and invites a second tap, so the card shows
    /// a spinner for the duration and ignores repeats.
    private func disconnect(_ connection: ServerConnection) {
        guard !disconnectingIds.contains(connection.id) else { return }
        disconnectingIds.insert(connection.id)
        Haptics.tap()
        Task {
            // The agents there are still waiting: their cards stay up.
            monitor.untrack(connectionId: connection.id, keepWaitingCards: true)
            await hub.disconnect(connection.id)
            disconnectingIds.remove(connection.id)
        }
    }

    /// Same teardown as `disconnect`, chained straight into a fresh connect()
    /// — a stalled tmux attach leaves SSH `.connected`, so a plain retry
    /// (onConnect alone) would no-op.
    private func retryAttach(_ connection: ServerConnection) {
        guard !disconnectingIds.contains(connection.id) else { return }
        disconnectingIds.insert(connection.id)
        Haptics.tap()
        Task {
            monitor.untrack(connectionId: connection.id, keepWaitingCards: true)
            await hub.disconnect(connection.id)
            disconnectingIds.remove(connection.id)
            connect(connection)
        }
    }

}

/// Version line closing the connections list. Prefers the
/// `MoshpitBuildStamp` written by scripts/release/build-ipa.sh (git SHA + time, "+"
/// marking uncommitted changes) so a sideloaded build can say exactly what
/// it is; store builds fall back to version (build).
enum HomeFooter {
    static var line: String {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        if let stamp = info?["MoshpitBuildStamp"] as? String, !stamp.isEmpty {
            return "v\(version) · \(stamp)"
        }
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "v\(version) (\(build))"
    }
}

/// What the home list stands on: the theme's screen colour under the signal
/// grid. Forms stay flat; this is the front door.
struct HomeStage: View {
    var body: some View {
        ZStack {
            Ink.screenBG
            SignalGrid()
                .stroke(Ink.terminalGrid, lineWidth: 0.6)
        }
        .ignoresSafeArea()
    }
}

// MARK: - What a list row asks Home to show

/// What Home's rows ask it to draw over the screen (``HomeNSHActions``'s
/// `present`, ``HomeTreeDialogs``): a modal card, and a worktree about to be
/// removed. The SwiftUI card these came from is gone (D8).
enum ConnectionCard {
    /// The system's alert Home presents on a row's behalf.
    struct Modal: Identifiable {
        enum Kind {
            case input(InputRequest)
            case notice(NoticeRequest)
        }
        let id = UUID()
        let kind: Kind
        /// What the input's field starts with — the current name for a
        /// rename, nothing for a name-before-create or a notice.
        var initialText: String {
            if case .input(let input) = kind { return input.initialText }
            return ""
        }

        var title: Text {
            switch kind {
            case .input(let input): Text(input.title)
            case .notice(let notice): Text(notice.title)
            }
        }

        var message: Text {
            switch kind {
            case .input(let input): input.message ?? Text(verbatim: "")
            case .notice(let notice): Text(notice.message)
            }
        }
    }

    /// A one-field prompt: rename, or name the thing about to be created.
    /// `apply` closes over the concrete controller so Home can run it.
    struct InputRequest {
        var title: LocalizedStringKey
        var message: Text?
        var placeholder: LocalizedStringKey
        var initialText = ""
        var confirmLabel: LocalizedStringKey
        /// Why what was typed won't be applied, or nil (see
        /// ``MultiplexerName/refusal(_:)``): it asks again with the reason
        /// as its message.
        var refuses: ((String) -> String?)?
        let apply: (String) -> Void
    }

    struct NoticeRequest {
        var title: LocalizedStringKey
        var message: String
    }

    struct WorktreeTarget: Identifiable {
        let id = UUID()
        let sessionName: String
        let repo: String
        let remove: (Bool) async -> RustHerdrController.WorktreeRemoval
    }
}

/// A page Home pushes: a connection's terminal, or a host's sessions
/// (§6.2), whose rows open the terminal on top of it.
enum HomeRoute: Hashable {
    case terminal(UUID)
    case sessions(UUID)
}
