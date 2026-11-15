import MoshpitKit
import SwiftUI

/// Where a held share goes, picked by the person (B622): the pane picker's
/// NSH screen (`ui/src/terminal/picker.rs`) in its send mode, over
/// `controller`'s whole tree, the panes running the share's agent first. A
/// pick sends the share there (``ShareQueueDrainer/send(_:toPane:hub:)``)
/// and closes; Home's notice goes once it is delivered. Presented from
/// Home's notice, in the same compact sheet as the pickers the breadcrumb
/// opens.
struct ShareSendPicker<C: MultiplexerControlling>: View {
    let controller: C
    /// The agent the share was queued for.
    let agent: String
    let pick: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var model = MuxPickerModel()
    @State private var failed = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            let state = MuxPickerState.sending(agent, controller: controller)
            NavigationStack {
                NSHPage(
                    screen: .muxPicker,
                    attach: { model.attach($0, current: state) },
                    onEvent: { event in
                        guard case .muxPicker(.select(let pane)) = event else { return }
                        Haptics.select()
                        pick(pane)
                        dismiss()
                    },
                    onFailure: { failed = true })
                    .nshPage()
                    .onAppear { model.update(state) }
                    .onChange(of: state) { _, new in model.update(new) }
            }
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .sheetSurface()
            .task { controller.refresh() }
        }
    }
}

extension MuxPickerState {
    /// The pane picker for a held share queued for `agent` (B622): every
    /// pane of `controller`'s tree, session by session, window by window,
    /// each with its place; the panes running `agent` first (the core's
    /// own reading of a pane, ``shareTarget(pane:agent:panes:)``, as the
    /// drain reads it), each group in the tree's order. No active row: the
    /// list picks a destination, not where to switch.
    @MainActor
    static func sending<C: MultiplexerControlling>(_ agent: String, controller: C) -> MuxPickerState {
        let snapshot = controller.snapshot
        let hooks = controller.agentHooks
        let herdr = controller.multiplexer == .herdr
        let vocab = controller.multiplexer.vocabulary
        var panes: [(pane: PaneInfo, place: String)] = []
        for session in snapshot.sortedSessions {
            for window in snapshot.windows(inSession: session.id) {
                let place = "\(snapshot.sessionDisplayName(session)) · \(window.displayTitle(vocab))"
                panes += snapshot.panes.values.filter { $0.windowId == window.id }
                    .sorted { $0.index < $1.index }
                    .map { ($0, place) }
            }
        }
        // herdr names a pane by the agent it detects, not by a process: only
        // that detection says an agent runs there (as the drain reads it).
        let records = panes.map { item in
            let stamped = hooks[item.pane.id]?.agent
            return SharePaneRecord(id: item.pane.id, agent: stamped,
                                   command: herdr ? (stamped ?? "") : item.pane.command)
        }
        let runs = Set(records.map(\.id).filter {
            if case .deliver = shareTarget(pane: $0, agent: agent, panes: records) { return true }
            return false
        })
        let rows = panes.map { item -> Row in
            let hook = hooks[item.pane.id]
            var row = Row(id: item.pane.id, index: item.pane.index, command: item.pane.command,
                          agent: hook?.agent, signal: AgentSignal(hook?.state))
            row.window = item.place
            return row
        }
        var state = MuxPickerState(kind: .panes, mux: controller.multiplexer,
                                   rows: rows.filter { runs.contains($0.id) } + rows.filter { !runs.contains($0.id) },
                                   active: nil)
        state.send = agent
        return state
    }
}
