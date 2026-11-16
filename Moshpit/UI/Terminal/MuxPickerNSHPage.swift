import MoshpitKit
import SwiftUI
import UIKit

/// Which of the multiplexer's lists a picker shows.
enum MuxPickerKind: String {
    case windows, sessions, panes
}

/// A picker drawn by NSH (`ui/src/terminal/picker.rs`) in the sheet's own
/// stack, in a compact sheet: medium and large detents and the grabber on
/// a phone, the 400 × 380 popover on an iPad (``AdaptivePresentation``),
/// iOS 26's glass or the app's sheet fill below. The list is the
/// controller's, posted as it changes (``MuxPickerState``); a row's
/// long-press menu is UIKit's on the surface (``NSHContextMenus``), its
/// pick handed back to the screen, which asks for a name (UIKit's alert
/// with a text field, ``NSHTextPrompt``) or confirms a kill (NSH's action
/// sheet) before the host acts. If its screen can't run, the picker says
/// so (``NSHPageFailed``).
struct MuxPickerNSHPage<C: MultiplexerControlling>: View {
    let controller: C
    let kind: MuxPickerKind

    @Environment(\.dismiss) private var dismiss
    @State private var model = MuxPickerModel()
    @State private var menus = NSHContextMenus()
    @State private var failed = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            let state = MuxPickerState(kind: kind, controller: controller)
            NavigationStack {
                NSHPage(
                    screen: .muxPicker,
                    attach: { surface in
                        model.attach(surface, current: state)
                        menus.install(on: surface)
                    },
                    onEvent: { if case .muxPicker(let event) = $0 { handle(event) } },
                    onFailure: { failed = true })
                    .nshPage()
                    .onAppear { show(state) }
                    .onChange(of: state) { _, new in show(new) }
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .sheetSurface()
            .task {
                controller.refresh()
                // tmux says nothing when a window's place changes: the list
                // asks again while it shows (B586).
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(30))
                    guard !Task.isCancelled else { break }
                    controller.relist()
                }
            }
        }
    }

    private func show(_ state: MuxPickerState) {
        model.update(state)
        menus.menus = MuxPickerModel.menus(for: state) { [weak model] action, id in
            model?.post(.muxPickerAct(action: action, id: id))
        }
        menus.cornerRadius = [:]
    }

    /// What the screen tells the host.
    private func handle(_ event: MuxPickerEvent) {
        switch event {
        case .select(let id):
            Haptics.select()
            switch kind {
            case .windows: controller.selectWindow(id)
            case .sessions: controller.selectSession(id)
            case .panes: controller.selectPane(id)
            }
            dismiss()
        case .new(let name):
            Haptics.tap()
            switch kind {
            case .windows:
                controller.newWindow(named: name)
                // The new window becomes active: the person lands on it.
                dismiss()
            case .sessions:
                controller.newSession(named: name)
                dismiss()
            case .panes:
                controller.newPane()
            }
        case .rename(let id, let name):
            switch kind {
            case .windows: controller.renameWindow(id, to: name)
            case .sessions: controller.renameSession(id, to: name)
            case .panes: break
            }
        case .kill(let id):
            switch kind {
            case .windows: controller.killWindow(id)
            case .sessions: controller.killSession(id)
            case .panes: break
            }
        case .prompt(let question):
            guard let surface = model.surface else { return }
            NSHTextPrompt.present(question, from: surface) { [weak model] answer in
                model?.post(answer)
            }
        }
    }
}

/// What the NSH picker shows: the controller's list as the screen's
/// `Picker` carries it (``MuxPickerPageState``, through the page bridge),
/// the facts of each row; the screen words them.
struct MuxPickerState: Equatable {
    struct Row: Equatable {
        var id: String
        var index: Int?
        var name = ""
        var panes: Int?
        var command = ""
        var agent: String?
        /// Where a window's active pane works (the active session's: its
        /// active window's): the page names a window tmux named after its
        /// program by its folder, as Home does.
        var path: String?
        var signal: AgentSignalValue?
        var attached = false
        var windows: Int?
        var window: String?
    }

    var kind: MuxPickerKind
    var mux: Multiplexer
    var rows: [Row] = []
    var active: String?
    /// The list picks where a held share goes (B622): the share's agent
    /// (``sending(_:controller:)``, ``ShareSendPicker``).
    var send: String?

    init(kind: MuxPickerKind, mux: Multiplexer, rows: [Row], active: String?) {
        self.kind = kind
        self.mux = mux
        self.rows = rows
        self.active = active
    }

    /// `controller`'s list of `kind`, as the SwiftUI sheets read it.
    @MainActor
    init<C: MultiplexerControlling>(kind: MuxPickerKind, controller: C) {
        let snapshot = controller.snapshot
        let hooks = controller.agentHooks
        self.kind = kind
        mux = controller.multiplexer
        let herdr = controller.multiplexer == .herdr
        switch kind {
        case .windows:
            active = snapshot.activeWindowId
            rows = snapshot.sortedWindows.map { window in
                let panes = snapshot.panes.values.filter { $0.windowId == window.id }
                let shown = panes.first { $0.isActive } ?? panes.min { $0.index < $1.index }
                let signal = agentSignalsAggregate(signals: panes.map { hooks[$0.id]?.dot(herdr: herdr) })
                var row = Row(id: window.id, index: window.index, name: window.name, panes: window.paneCount,
                              command: shown?.command ?? "", signal: signal)
                row.path = shown?.path
                row.agent = shown.flatMap { hooks[$0.id]?.agent }.flatMap { $0.isEmpty ? nil : $0 }
                return row
            }
        case .sessions:
            active = snapshot.activeSessionId
            let windows = snapshot.sortedWindows
            let activeWindow = snapshot.activeWindowId.flatMap { snapshot.windows[$0] }
            rows = snapshot.sortedSessions.map { session in
                var row = Row(id: session.id, name: session.name, attached: session.isAttached)
                if session.id == snapshot.activeSessionId {
                    row.windows = windows.count
                    row.window = activeWindow.map { "\($0.index):\($0.name)" }
                    // What names that window the way its row is named.
                    let pane = snapshot.activePaneId.flatMap { snapshot.panes[$0] }
                    row.command = pane?.command ?? ""
                    row.path = pane?.path
                    row.agent = pane.flatMap { hooks[$0.id]?.agent }.flatMap { $0.isEmpty ? nil : $0 }
                }
                return row
            }
        case .panes:
            active = snapshot.activePaneId
            rows = snapshot.activePanes.sorted { $0.index < $1.index }.map { pane in
                let hook = hooks[pane.id]
                return Row(id: pane.id, index: pane.index, command: pane.command, agent: hook?.agent,
                           signal: hook?.dot(herdr: herdr))
            }
        }
    }

    /// The list as the page bridge carries it.
    var page: MuxPickerPageState {
        MuxPickerPageState(kind: kind.rawValue, mux: mux == .herdr ? "herdr" : "tmux",
                           rows: rows.map(Self.row), active: active, send: send)
    }

    private static func row(_ row: Row) -> MuxPickerRow {
        let count = { (n: Int?) in n.map { UInt32(clamping: $0) } }
        let signal: String? = row.signal.map {
            switch $0 {
            case .attention: "attention"
            case .working: "working"
            case .done: "done"
            }
        }
        return MuxPickerRow(id: row.id, index: count(row.index), name: row.name, panes: count(row.panes),
                            command: row.command, agent: row.agent, path: row.path, signal: signal,
                            attached: row.attached, windows: count(row.windows), window: row.window)
    }
}

/// What the NSH picker was last told, kept so a surface made again starts
/// from it, and what it tells the host.
@MainActor
final class MuxPickerModel {
    private(set) weak var surface: NSHSurface?
    private var state: MuxPickerState?

    func attach(_ surface: NSHSurface) {
        self.surface = surface
        if let state { surface.post(.muxPicker(state: state.page)) }
    }

    /// ``attach(_:)``, the model filled from `current` (the page's picker as
    /// its body has it now) while it has none (B366): the screen starts from
    /// the page's state whether or not `onAppear` came first. A state the
    /// model has is newer (`onChange` keeps it) and stays.
    func attach(_ surface: NSHSurface, current state: MuxPickerState) {
        if self.state == nil {
            self.surface = nil
            update(state)
        }
        attach(surface)
    }

    func update(_ state: MuxPickerState) {
        guard state != self.state else { return }
        self.state = state
        surface?.post(.muxPicker(state: state.page))
    }

    @discardableResult
    func post(_ message: PageMessage) -> Bool {
        surface?.post(message) ?? false
    }

    /// The long-press menus of `state`'s rows, by the row's identifier
    /// (`mux-picker-row-<id>`): Rename and the kill, as the SwiftUI rows'
    /// `.contextMenu`; panes have none. A pick goes back to the screen
    /// (`perform(action, id)`), which asks before the host acts.
    static func menus(for state: MuxPickerState,
                      perform: @escaping (MuxPickerAction, String) -> Void) -> [String: () -> [UIMenuElement]] {
        guard state.kind != .panes else { return [:] }
        let vocab = state.mux.words
        let noun = state.kind == .sessions ? vocab.session : vocab.window
        var menus: [String: () -> [UIMenuElement]] = [:]
        for row in state.rows {
            let id = row.id
            menus["mux-picker-row-\(id)"] = {
                [
                    UIAction(title: String(localized: "Rename"), image: UIImage(systemName: "pencil")) { _ in
                        perform(.rename, id)
                    },
                    UIAction(title: String(localized: "\(vocab.kill) \(noun)"),
                             image: UIImage(systemName: "trash"), attributes: .destructive) { _ in
                        perform(.kill, id)
                    },
                ]
            }
        }
        return menus
    }
}
