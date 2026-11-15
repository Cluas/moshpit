import MoshpitKit
import SwiftUI
import UIKit

/// Home's connection list drawn by NSH (`ui/src/home`).
///
/// ``HomeView`` stays the owner of everything around the list: the
/// navigation (the phone's stack, the iPad's split view), the sheets, the
/// screen-wide cards (a rename, a name before creating, a notice), the
/// host-key and passphrase prompts, the delete confirmation. This page asks
/// the core for Home's record (``HomeRecord``: `app::home`, built from the
/// session registry and the agents monitor's clock) and posts it whenever it
/// changes; the screen draws it and tells the host what the user asks for
/// (``HomeIntent``). Connection-level asks go to Home (``HomeNSHActions``);
/// the tree's (open a window or a pane, rename,
/// kill, a new session, window, pane or agent task, a worktree's removal)
/// are run against the session's multiplexer by ``HomeTreeActions``, as a
/// host's sessions page (``SessionsNSHPage``) runs them.
struct HomeNSHPage: View {
    let store: ConnectionStore
    let hub: SessionHub
    /// Cards whose disconnect is in flight (Home's `disconnectingIds`).
    let disconnecting: Set<UUID>
    /// Home is the iPad split view's sidebar: the bar carries its button.
    var sidebar = false
    /// Home's model of the page, kept by Home so a page made again starts
    /// from it.
    let model: HomeNSHModel
    let actions: HomeNSHActions
    var onFailure: () -> Void

    @Environment(AgentActivityMonitor.self) private var monitor
    /// The companion (moshpit-design PET-DESIGN §7.3): its message is posted
    /// beside Home's record whenever it changes.
    private let pet = CompanionPet.shared
    /// The app in front or not: the pet talks only while Home is seen.
    @Environment(\.scenePhase) private var scenePhase
    @State private var killTarget: HomeKillTarget?
    @State private var worktreeTarget: ConnectionCard.WorktreeTarget?
    @State private var worktreeForceTarget: ConnectionCard.WorktreeTarget?
    @State private var agentTask: HomeAgentTaskTarget?
    /// The rows' long-press menus (``HomeNSHMenus``), on the surface.
    @State private var menus = NSHContextMenus()

    var body: some View {
        let record = HomeRecord.now(
            core: .shared, connections: store.connections, session: { hub.session(for: $0)?.rust?.id },
            agents: monitor.machine, disconnecting: disconnecting, sidebar: sidebar)
        let cards = tree.rows(of: record)
        NSHPage(
            screen: .home,
            prepare: { HomeNSHComponents.register(on: $0) },
            attach: { surface in
                model.attach(surface, current: record)
                menus.install(on: surface)
            },
            onMessage: { message in
                guard let intent = model.handle(message) else { return }
                // A tray's action: its confirmation hangs from its row (B499).
                menus.anchor(to: message)
                perform(intent)
            },
            onFailure: onFailure)
            // G5's first rule, but for the top and the bottom only: the
            // iPad's sidebar column reaches 100 pt past the screen's leading
            // edge behind a safe area of that width (iOS 18.2), and a page
            // that ignored it laid its list out there. Home has no field: no
            // keyboard to avoid.
            .ignoresSafeArea(.all, edges: .vertical)
            .toolbar(.hidden, for: .navigationBar)
            // The title a pushed page's back button shows.
            .navigationTitle("Moshpit")
            .background { shortcuts }
            .onAppear {
                model.update(record)
                show(cards)
                model.visible = true
                pet.onHome = true
                model.updatePet(pet.message)
                pet.seen()
                // A ready egg hatches when Home shows (the core refuses
                // unless it's ready and iCloud has settled).
                pet.hatch()
            }
            .onDisappear {
                model.visible = false
                pet.onHome = false
            }
            .task {
                // tmux says nothing when a window's place changes
                // (`swap-window`, `move-window -r`): the trees ask as Home
                // shows, and every 30 s while it does (B586).
                while !Task.isCancelled {
                    relistTrees()
                    try? await Task.sleep(for: .seconds(30))
                }
            }
            .onChange(of: pet.revision) { _, _ in model.updatePet(pet.message) }
            .onChange(of: scenePhase) { _, phase in model.active = phase == .active }
            .onChange(of: record) { _, new in model.update(new) }
            .onChange(of: cards) { _, new in show(new) }
            .modifier(HomeTreeDialogs(
                killTarget: $killTarget, worktreeTarget: $worktreeTarget,
                worktreeForceTarget: $worktreeForceTarget, agentTask: $agentTask,
                menus: menus, actions: actions))
    }

    /// Every live connection's tree, re-listed quietly (B586).
    private func relistTrees() {
        for connection in store.connections {
            hub.session(for: connection)?.tmuxControl?.relist()
        }
    }

    /// ⌘, and ⌘N: the bar is the screen's, so the shortcuts ride on
    /// buttons nobody sees.
    private var shortcuts: some View {
        ZStack {
            Button("Settings") { actions.settings() }
                .keyboardShortcut(",", modifiers: .command)
            Button("Add Connection") { actions.add() }
                .keyboardShortcut("n", modifiers: .command)
        }
        .opacity(0)
        .frame(width: 0, height: 0)
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }

    /// The rows' menus to the surface.
    private func show(_ rows: [HomeNSHMenus.Row]) {
        menus.menus = HomeNSHMenus.menus(for: rows, perform: perform)
        menus.cornerRadius = HomeNSHMenus.corners(for: rows)
    }

    // MARK: What the screen asks for

    /// The tree's asks, run as a host's sessions page runs them.
    private var tree: HomeTreeActions {
        HomeTreeActions(store: store, hub: hub, actions: actions,
                        confirmKill: { killTarget = $0 },
                        removeWorktree: { worktreeTarget = $0 },
                        startAgentTask: { agentTask = $0 })
    }

    private func perform(_ intent: HomeIntent) {
        if tree.perform(intent) { return }
        switch intent {
        case .settings: actions.settings()
        case .add: actions.add()
        case .toggleSidebar: actions.toggleSidebar()
        case .connect(let id): actions.connect(id)
        case .enter(let id):
            Haptics.select()
            actions.enter(id)
        case .sessions(let id):
            Haptics.select()
            actions.sessions(id)
        case .edit(let id): actions.edit(id)
        case .delete(let id): actions.delete(id)
        case .disconnect(let id): actions.disconnect(id)
        case .retryAttach(let id): actions.retryAttach(id)
        default: break
        }
    }
}

/// What Home does for the NSH page's connection-level asks, and Home's
/// screen-wide card.
struct HomeNSHActions {
    var settings: () -> Void
    var add: () -> Void
    var toggleSidebar: () -> Void
    var connect: (UUID) -> Void
    var enter: (UUID) -> Void
    /// A host's sessions page (§6.2), pushed on Home's stack.
    var sessions: (UUID) -> Void
    var edit: (UUID) -> Void
    var delete: (UUID) -> Void
    var disconnect: (UUID) -> Void
    var retryAttach: (UUID) -> Void
    /// Asks with the system's alert over the whole screen (``HomeView``'s
    /// `cardModal`).
    var present: (ConnectionCard.Modal) -> Void

    /// The rename card, pre-filled with `currentName`, applying through
    /// `apply`.
    static func rename(title: String, currentName: String,
                       apply: @escaping (String) -> Void) -> ConnectionCard.Modal {
        ConnectionCard.Modal(kind: .input(ConnectionCard.InputRequest(
            title: "\(title)", placeholder: "Name",
            initialText: currentName, confirmLabel: "Rename",
            refuses: refusal, apply: apply)))
    }

    /// The name-before-create card: blank lets the multiplexer name it
    /// (`apply` gets nil).
    static func create(noun: String, apply: @escaping (String?) -> Void) -> ConnectionCard.Modal {
        ConnectionCard.Modal(kind: .input(ConnectionCard.InputRequest(
            title: "New \(noun)",
            message: Text("Leave blank for an automatic name."),
            placeholder: "Name (optional)", confirmLabel: "Create",
            refuses: refusal,
            apply: { typed in
                let trimmed = typed.trimmingCharacters(in: .whitespacesAndNewlines)
                apply(trimmed.isEmpty ? nil : trimmed)
            })))
    }

    /// A name the multiplexer would not get as typed (B139).
    nonisolated static func refusal(_ typed: String) -> String? {
        MultiplexerName.refusal(typed.trimmingCharacters(in: .whitespacesAndNewlines))
    }
}

/// A kill waiting on its confirmation: what the button says, the dialog's
/// question, what the kill does.
struct HomeKillTarget: Identifiable {
    let id = UUID()
    let label: String
    let confirmTitle: String
    let perform: () -> Void
}

/// A new agent task's sheet waiting to show.
struct HomeAgentTaskTarget: Identifiable {
    let id = UUID()
    let initialRepos: [String]
    let load: () async -> (repos: [String], agents: [String])
    let start: (AgentTaskRequest) async -> AgentTaskOutcome
}

// MARK: - The intents

/// What the NSH Home asks for (`ui/src/home/mod.rs`'s `Intent`), each
/// naming its connection and, on a tree row, the session, window or pane.
enum HomeIntent: Equatable {
    case settings, add, toggleSidebar
    case connect(UUID), enter(UUID), sessions(UUID), edit(UUID), delete(UUID), disconnect(UUID), retryAttach(UUID)
    case refresh(UUID), newSession(UUID), newAgentTask(UUID), restartServer(UUID)
    case window(UUID, String), pane(UUID, String)
    case renameSession(UUID, String), killSession(UUID, String), newWindow(UUID, String)
    case removeWorktree(UUID, String)
    case renameWindow(UUID, String), killWindow(UUID, String), newPane(UUID, String)
    case killPane(UUID, String)

    /// The intent of the screen's message, or nil when it is none.
    init?(_ message: [String: Any]) {
        guard let ev = message["ev"] as? String else { return nil }
        switch ev {
        case "settings": self = .settings; return
        case "add": self = .add; return
        case "toggle-sidebar": self = .toggleSidebar; return
        default: break
        }
        guard let id = (message["connection"] as? String).flatMap(UUID.init(uuidString:)) else { return nil }
        let session = message["session"] as? String
        let window = message["window"] as? String
        let pane = message["pane"] as? String
        switch (ev, session, window, pane) {
        case ("connect", _, _, _): self = .connect(id)
        case ("enter", _, _, _): self = .enter(id)
        case ("sessions", _, _, _): self = .sessions(id)
        case ("edit", _, _, _): self = .edit(id)
        case ("delete", _, _, _): self = .delete(id)
        case ("disconnect", _, _, _): self = .disconnect(id)
        case ("retry-attach", _, _, _): self = .retryAttach(id)
        case ("refresh", _, _, _): self = .refresh(id)
        case ("new-session", _, _, _): self = .newSession(id)
        case ("new-agent-task", _, _, _): self = .newAgentTask(id)
        case ("restart-server", _, _, _): self = .restartServer(id)
        case ("window", _, let w?, _): self = .window(id, w)
        case ("pane", _, _, let p?): self = .pane(id, p)
        case ("rename-session", let s?, _, _): self = .renameSession(id, s)
        case ("kill-session", let s?, _, _): self = .killSession(id, s)
        case ("new-window", let s?, _, _): self = .newWindow(id, s)
        case ("remove-worktree", let s?, _, _): self = .removeWorktree(id, s)
        case ("rename-window", _, let w?, _): self = .renameWindow(id, w)
        case ("kill-window", _, let w?, _): self = .killWindow(id, w)
        case ("new-pane", _, let w?, _): self = .newPane(id, w)
        case ("kill-pane", _, _, let p?): self = .killPane(id, p)
        default: return nil
        }
    }
}

// MARK: - The model

/// What the NSH Home shows, owned by Home so a surface made again starts
/// from it (``NSHPage``'s third rule): the last record, and the trees'
/// disclosure the screen told the host.
@MainActor
final class HomeNSHModel {
    private(set) var record: HomeRecord?
    /// By connection id: the sessions collapsed, the windows showing their
    /// panes, and whether Home's idle group is open, as the screen last
    /// told them; `told` once the sessions page told its part (a tree Home's
    /// idle group alone made is not its disclosure, B553).
    private(set) var trees: [String: (collapsed: [String], expanded: [String], idle: Bool, told: Bool)] = [:]
    private weak var surface: NSHSurface?
    /// The companion's last message (`{"op": "pet"}`).
    private var pet: String?
    /// Home is on screen (not under a pushed page): the egg hatches only
    /// then, so the person sees it, and the pet talks only then.
    var visible = false {
        didSet { if visible != oldValue { postVisible() } }
    }
    /// The app is in front.
    var active = true {
        didSet { if active != oldValue { postVisible() } }
    }

    /// `{"op": "visible"}`: whether Home is seen, with the local hour and
    /// the local day's number (the pet's night and morning lines).
    private func postVisible() {
        let now = Date()
        let hour = Calendar.current.component(.hour, from: now)
        let offset = Double(TimeZone.current.secondsFromGMT(for: now))
        let day = Int(((now.timeIntervalSince1970 + offset) / 86_400).rounded(.down))
        surface?.post(json: "{\"op\":\"visible\",\"visible\":\(visible && active),\"hour\":\(hour),\"day\":\(day)}")
    }

    /// `surface` shows Home from now on: the trees as they were, then the
    /// current state.
    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if !trees.isEmpty { surface.post(treesMessage) }
        if let record { surface.post(json: record.message) }
        if let pet { surface.post(json: pet) }
        postVisible()
        if revealOnAttach {
            revealOnAttach = false
            surface.post(json: #"{"op":"top"}"#)
        }
    }

    /// Scroll Home to its top, where "Needs you" lists the agents waiting
    /// (B661); once the page is up if it isn't yet.
    func revealNeedsYou() {
        guard let surface else {
            revealOnAttach = true
            return
        }
        surface.post(json: #"{"op":"top"}"#)
    }

    /// A "Needs you" asked for before the page was up.
    private var revealOnAttach = false

    /// The companion as it is now.
    func updatePet(_ message: String) {
        guard message != pet else { return }
        pet = message
        surface?.post(json: message)
    }

    /// ``attach(_:)``, the model filled from `current` (the page's record as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current record: HomeRecord) {
        if self.record == nil {
            self.surface = nil
            update(record)
        }
        attach(surface)
    }

    /// Home as it is now.
    func update(_ record: HomeRecord) {
        guard record != self.record else { return }
        self.record = record
        surface?.post(json: record.message)
    }

    /// What the screen told the host: its trees' disclosure is kept here,
    /// anything else is an intent. Each page sends only what it keeps
    /// (Home its idle group, the sessions page its sessions and windows),
    /// so a field a message leaves out keeps its value.
    func handle(_ message: [String: Any]) -> HomeIntent? {
        switch message["ev"] as? String {
        case "hatch":
            if visible { CompanionPet.shared.hatch() }
            return nil
        case "egg-tap":
            if visible { CompanionPet.shared.tapEgg() }
            return nil
        case "pet-seen":
            if visible { CompanionPet.shared.seen() }
            return nil
        default:
            break
        }
        if message["ev"] as? String == "tree", let connection = message["connection"] as? String {
            var tree = trees[connection] ?? (collapsed: [], expanded: [], idle: false, told: false)
            if let collapsed = message["collapsed"] as? [String] {
                tree.collapsed = collapsed
                tree.told = true
            }
            if let expanded = message["expanded"] as? [String] {
                tree.expanded = expanded
                tree.told = true
            }
            if let idle = message["idle"] as? Bool { tree.idle = idle }
            trees[connection] = tree
            return nil
        }
        guard let intent = HomeIntent(message) else {
            Log.ui.error("home: no intent in \(String(describing: message["ev"]), privacy: .public)")
            return nil
        }
        return intent
    }

    /// `{"op": "trees", …}`: the disclosure the screen last told.
    var treesMessage: [String: Any] {
        ["op": "trees", "trees": trees.keys.sorted().map { id in
            ["connection": id, "collapsed": trees[id]!.collapsed, "expanded": trees[id]!.expanded,
             "idle": trees[id]!.idle, "told": trees[id]!.told]
        }]
    }
}

// MARK: - The record

/// Home's record as the core builds it (`app::home`, `ui/src/home/model.rs`'s
/// `HomeState`): the `{"op": "home"}` message, from the registry's view of
/// each saved connection's session and the agents monitor's clock, posted
/// to the screen as it is.
struct HomeRecord: Equatable {
    /// `{"op": "home", …}`.
    var message: String

    /// A card's id and where its session is, as the message says.
    struct Card: Equatable {
        var id: UUID
        var link: HomeLink
    }

    /// The message's cards, in its order.
    var cards: [Card] {
        guard let home = try? JSONDecoder().decode(Home.self, from: Data(message.utf8)) else { return [] }
        return home.connections.compactMap { card in UUID(uuidString: card.id).map { Card(id: $0, link: card.link) } }
    }

    private struct Home: Decodable {
        struct Connection: Decodable {
            var id: String
            var link: HomeLink
        }
        var connections: [Connection]
    }

    /// Home now, as `core` builds it: `connections` in the store's order,
    /// each with its session on the core (`session`: the registry's id of
    /// the hub's session for it), `agents` the monitor whose clock dates
    /// the panes' stamps. Read in a view, it is read again after every
    /// batch of the sessions' events (``RustCore/changes``).
    /// A mosh card's UDP port is Settings' first, the one its connect asks
    /// for (B430).
    @MainActor
    static func now(core: RustCore, connections: [ServerConnection],
                    session: (ServerConnection) -> UInt64?, agents: AgentMachine?,
                    disconnecting: Set<UUID>, sidebar: Bool,
                    udpPort: Int = AppSettings.shared.udpRangeStart) -> HomeRecord {
        _ = core.changes.batches
        let connections = connections.map { connection in
            HomeConnection(
                id: connection.id.uuidString, name: connection.displayName, user: connection.username,
                host: connection.host, port: UInt16(clamping: connection.port), emoji: connection.emoji,
                mosh: connection.connectionProtocol == .mosh,
                udpPort: UInt32(clamping: udpPort),
                tmux: connection.multiplexer == .tmux,
                disconnecting: disconnecting.contains(connection.id),
                session: session(connection))
        }
        return HomeRecord(message: core.sessions.homeMessage(
            connections: connections, agents: agents, version: HomeFooter.line,
            sidebar: sidebar, locale: NSHScreenParams.locale))
    }
}

/// Where a card's session is (`ui/src/home/model.rs`'s `Link`).
enum HomeLink: String, Decodable {
    case saved, connecting, reconnecting, attaching, stalled, dead, live
}

// MARK: - The host's components for Home

/// What Home's screen places that NSH has no equivalent of, registered on
/// its surface before it runs:
///
/// - `moshpit/signal-grid`: `HomeStage`'s signal grid, the same path
///   SwiftUI strokes (``SignalGrid``). Props `color` (`#RRGGBBAA`),
///   `lineWidth`.
/// - `moshpit/app-icon-mark`: the app's mark in the icon the user picked
///   (``AppIconMark``), the empty state's.
/// - `moshpit/action-menu`: a clear button over its box that opens the
///   system's pop-up menu (SwiftUI's `Menu`): props `items`
///   (`[{id, title, subtitle, image, destructive, group}]`, a divider
///   between groups), `identifier`, `label`; event `select` (the item's
///   id).
@MainActor
enum HomeNSHComponents {
    static func register(on surface: NSHSurface) {
        surface.register(component: "moshpit/signal-grid") { _ in SignalGridComponent() }
        surface.register(component: "moshpit/app-icon-mark") { _ in AppIconMarkComponent() }
        surface.register(component: "moshpit/action-menu") { ActionMenuComponent(context: $0) }
    }
}

/// `moshpit/signal-grid`.
@MainActor
private final class SignalGridComponent: NSHComponent {
    private final class Grid: UIView {
        override class var layerClass: AnyClass { CAShapeLayer.self }
        var shape: CAShapeLayer { layer as! CAShapeLayer }

        override func layoutSubviews() {
            super.layoutSubviews()
            shape.path = SignalGrid().path(in: bounds).cgPath
        }
    }

    private let grid = Grid()
    var view: UIView { grid }

    init() {
        grid.isUserInteractionEnabled = false
        grid.backgroundColor = .clear
        grid.shape.fillColor = nil
        grid.shape.lineWidth = 0.6
        grid.shape.contentsScale = UIScreen.main.scale
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "color": grid.shape.strokeColor = NSHHostComponents.color(value)?.cgColor
        case "lineWidth": grid.shape.lineWidth = CGFloat(NSHHostComponents.number(value) ?? 0.6)
        default: break
        }
    }
}

/// `moshpit/app-icon-mark`: ``AppIconMark`` at its box's size.
@MainActor
private final class AppIconMarkComponent: NSHComponent {
    private final class Host: UIView {
        var hosting: UIHostingController<AnyView>?
        override func layoutSubviews() {
            super.layoutSubviews()
            let side = min(bounds.width, bounds.height)
            guard side > 0 else { return }
            if hosting == nil || hosting?.view.bounds.width != side {
                hosting?.view.removeFromSuperview()
                let controller = UIHostingController(rootView: AnyView(
                    AppIconMark(size: side).environment(AppSettings.shared)))
                controller.view.backgroundColor = .clear
                controller.view.isUserInteractionEnabled = false
                addSubview(controller.view)
                hosting = controller
            }
            hosting?.view.frame = CGRect(x: (bounds.width - side) / 2, y: (bounds.height - side) / 2,
                                         width: side, height: side)
        }
    }

    private let host = Host()
    var view: UIView { host }

    init() {
        host.isAccessibilityElement = false
        host.isUserInteractionEnabled = false
    }

    func setProp(_ name: String, _ value: Any?) {}
}

/// `moshpit/action-menu`.
@MainActor
private final class ActionMenuComponent: NSHComponent {
    private let button = UIButton(type: .custom)
    private let context: NSHComponentContext
    var view: UIView { button }

    init(context: NSHComponentContext) {
        self.context = context
        button.showsMenuAsPrimaryAction = true
        button.backgroundColor = .clear
    }

    func setProp(_ name: String, _ value: Any?) {
        switch name {
        case "identifier": button.accessibilityIdentifier = value as? String
        case "label": button.accessibilityLabel = value as? String
        case "items": button.menu = Self.menu((value as? [Any] ?? []).compactMap { $0 as? [String: Any] },
                                              context: context)
        default: break
        }
    }

    /// The menu of `items`, a divider between groups.
    static func menu(_ items: [[String: Any]], context: NSHComponentContext) -> UIMenu {
        var groups: [Int: [UIAction]] = [:]
        for item in items {
            guard let id = item["id"] as? String, let title = item["title"] as? String else { continue }
            let image = (item["image"] as? String).flatMap { UIImage(systemName: $0) }
            let destructive = item["destructive"] as? Bool ?? false
            let group = (item["group"] as? NSNumber)?.intValue ?? 0
            groups[group, default: []].append(UIAction(
                title: title, subtitle: item["subtitle"] as? String, image: image,
                attributes: destructive ? .destructive : []) { _ in
                    context.emit("select", id)
                })
        }
        let sections = groups.keys.sorted().map {
            UIMenu(title: "", options: .displayInline, children: groups[$0]!)
        }
        return UIMenu(children: sections)
    }
}
