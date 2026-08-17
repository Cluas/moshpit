import SwiftUI

// MARK: - Screen 4 — Windows sheet

struct WindowsSheet<C: MultiplexerControlling>: View {
    let controller: C
    @Environment(\.dismiss) private var dismiss
    @State private var promptingName = false
    @State private var name = ""
    /// Long-press row actions — closing a finished agent used to take a
    /// six-step detour through Home.
    @State private var renaming: WindowInfo?
    @State private var renameText = ""
    @State private var killing: WindowInfo?

    var body: some View {
        let vocab = controller.multiplexer.vocabulary
        CompactSheet(
            title: LocalizedStringKey(vocab.windowPlural),
            onPlus: { name = ""; promptingName = true },
            // herdr's frame channel never wires `onSwitch` (only tmux's -CC
            // controller and the mosh sidecar do), so the horizontal swipe is
            // dead there — advertising it is a promise the app doesn't keep.
            footerHint: controller.multiplexer == .herdr
                ? LocalizedStringKey("Tap to switch") : LocalizedStringKey("Swipe to switch"),
            footerKeys: vocab.windowKeys
        ) {
            let snapshot = controller.snapshot
            VStack(spacing: 2) {
                ForEach(snapshot.sortedWindows) { window in
                    let signal = agentSignal(window, snapshot: snapshot)
                    SheetListRow(
                        icon: vocab.windowIcon,
                        name: window.displayTitle(vocab),
                        meta: windowMeta(window, snapshot: snapshot),
                        isActive: window.id == snapshot.activeWindowId,
                        statusColor: signal?.color,
                        statusLabel: signal?.label
                    ) {
                        Haptics.select()
                        controller.selectWindow(window.id)
                        dismiss()
                    }
                    .contextMenu {
                        Button { renameText = window.name; renaming = window } label: {
                            Label("Rename", systemImage: "pencil")
                        }
                        Button(role: .destructive) { killing = window } label: {
                            Label("\(controller.multiplexer.vocabulary.killVerb) \(controller.multiplexer.vocabulary.window)", systemImage: "trash")
                        }
                    }
                }
            }
        }
        .alert("Rename \(controller.multiplexer.vocabulary.window)", isPresented: Binding(
            get: { renaming != nil }, set: { if !$0 { renaming = nil } }
        )) {
            TextField("Name", text: $renameText)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Button("Rename") {
                if let target = renaming {
                    controller.renameWindow(target.id, to: renameText)
                }
                renaming = nil
            }
            Button("Cancel", role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            "\(controller.multiplexer.vocabulary.killVerb) \(controller.multiplexer.vocabulary.window.lowercased()) \(killing.map { $0.displayTitle(controller.multiplexer.vocabulary) } ?? "")? Every pane in it dies.",
            isPresented: Binding(get: { killing != nil }, set: { if !$0 { killing = nil } }),
            titleVisibility: .visible
        ) {
            Button("\(controller.multiplexer.vocabulary.killVerb) \(controller.multiplexer.vocabulary.window)", role: .destructive) {
                if let target = killing { controller.killWindow(target.id) }
                killing = nil
            }
        }
        .task { controller.refresh() }
        .alert("New \(controller.multiplexer.vocabulary.window)", isPresented: $promptingName) {
            TextField("Name (optional)", text: $name)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Button("Create") {
                Haptics.tap()
                let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
                controller.newWindow(named: trimmed.isEmpty ? nil : trimmed)
                // The new window becomes active — drop the sheet so the user
                // lands on it instead of wondering whether anything happened.
                dismiss()
            }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("Leave blank to let the program name it.")
        }
    }

    /// Vibe Island signal for a window: amber if ANY of its panes' hook stamp
    /// says "attention", teal if any says "working", nil otherwise. Same
    /// authoritative `@moshpit_state` source the island itself uses. Returns
    /// the signal (not just its colour) so the row can also SAY it — the dot
    /// alone is invisible to VoiceOver and ambiguous to colourblind eyes.
    private func agentSignal(_ window: WindowInfo, snapshot: TmuxSessionController.Snapshot) -> AgentSignal? {
        let paneIds = snapshot.panes.values
            .filter { $0.windowId == window.id }.map(\.id)
        // One shared mapping — see `AgentSignal`. The private ladder that used
        // to live here was the seed of "amber means needs you" drifting apart
        // between the sheets, Home and the Island.
        return AgentSignal.aggregate(paneIds.map { AgentSignal(controller.agentHooks[$0]?.state) })
    }

    private func windowMeta(_ window: WindowInfo, snapshot: TmuxSessionController.Snapshot) -> String {
        let cmd = snapshot.panes.values
            .first { $0.windowId == window.id && $0.isActive }?.command
            ?? snapshot.panes.values.first { $0.windowId == window.id }?.command
            ?? ""
        let panes = String(localized: "\(window.paneCount) panes")
        return cmd.isEmpty ? panes : "\(panes) · \(cmd)"
    }
}

// MARK: - Screen 10 — Sessions sheet

struct SessionsSheet<C: MultiplexerControlling>: View {
    let controller: C
    @Environment(\.dismiss) private var dismiss
    @State private var promptingName = false
    @State private var name = ""
    @State private var renaming: SessionInfo?
    @State private var renameText = ""
    @State private var killing: SessionInfo?

    var body: some View {
        let vocab = controller.multiplexer.vocabulary
        CompactSheet(
            title: LocalizedStringKey(vocab.sessionPlural),
            onPlus: { name = ""; promptingName = true },
            // herdr's frame channel never wires `onSwitch` (only tmux's -CC
            // controller and the mosh sidecar do), so the horizontal swipe is
            // dead there — advertising it is a promise the app doesn't keep.
            footerHint: controller.multiplexer == .herdr
                ? LocalizedStringKey("Tap to switch") : LocalizedStringKey("Swipe to switch"),
            footerKeys: vocab.sessionKeys
        ) {
            let snapshot = controller.snapshot
            let sessions = snapshot.sessions.values.sorted { $0.id < $1.id }
            VStack(spacing: 2) {
                ForEach(sessions) { session in
                    SheetListRow(
                        icon: vocab.sessionIcon,
                        name: snapshot.sessionDisplayName(session),
                        meta: sessionMeta(session, snapshot: snapshot),
                        isActive: session.id == snapshot.activeSessionId
                    ) {
                        Haptics.select()
                        controller.selectSession(session.id)
                        dismiss()
                    }
                    .contextMenu {
                        Button { renameText = session.name; renaming = session } label: {
                            Label("Rename", systemImage: "pencil")
                        }
                        Button(role: .destructive) { killing = session } label: {
                            Label("\(controller.multiplexer.vocabulary.killVerb) \(controller.multiplexer.vocabulary.session)", systemImage: "trash")
                        }
                    }
                }
            }
        }
        .alert("Rename \(controller.multiplexer.vocabulary.session)", isPresented: Binding(
            get: { renaming != nil }, set: { if !$0 { renaming = nil } }
        )) {
            TextField("Name", text: $renameText)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Button("Rename") {
                if let target = renaming {
                    controller.renameSession(target.id, to: renameText)
                }
                renaming = nil
            }
            Button("Cancel", role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            "\(controller.multiplexer.vocabulary.killVerb) \(controller.multiplexer.vocabulary.session.lowercased()) \(killing.map { controller.snapshot.sessionDisplayName($0) } ?? "")? Everything in it dies.",
            isPresented: Binding(get: { killing != nil }, set: { if !$0 { killing = nil } }),
            titleVisibility: .visible
        ) {
            Button("\(controller.multiplexer.vocabulary.killVerb) \(controller.multiplexer.vocabulary.session)", role: .destructive) {
                if let target = killing { controller.killSession(target.id) }
                killing = nil
            }
        }
        .task { controller.refresh() }
        .alert("New \(controller.multiplexer.vocabulary.session)", isPresented: $promptingName) {
            TextField("Name (optional)", text: $name)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Button("Create") {
                Haptics.tap()
                let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
                controller.newSession(named: trimmed.isEmpty ? nil : trimmed)
                // newSession selects the created session once tmux confirms it;
                // dropping the sheet completes the "create → I'm in it" story.
                dismiss()
            }
            Button("Cancel", role: .cancel) { }
        }
    }

    private func sessionMeta(_ session: SessionInfo, snapshot: TmuxSessionController.Snapshot) -> String {
        if session.id == snapshot.activeSessionId {
            let count = snapshot.sortedWindows.count
            let activeWindow = snapshot.activeWindowId.flatMap { snapshot.windows[$0] }
            let head = "\(count) \(controller.multiplexer.vocabulary.windowPlural.lowercased())"
            if let activeWindow {
                return "\(head) · \(activeWindow.index):\(activeWindow.name)"
            }
            return head
        }
        return session.isAttached ? String(localized: "attached") : String(localized: "detached")
    }
}

// MARK: - Screens 11/12 — Select Pane sheet with layout-aware pane board

struct SelectPaneSheet<C: MultiplexerControlling>: View {
    let controller: C
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        CompactSheet(
            title: "Select Pane",
            onPlus: { Haptics.tap(); controller.newPane() },
            footerHint: LocalizedStringKey("Tap to focus · \(controller.multiplexer.vocabulary.splitHint)"),
            footerKeys: controller.multiplexer.vocabulary.paneKeys
        ) {
            // A plain list, same shape as the Windows and Sessions sheets —
            // deliberately NOT a spatial mirror of the split layout. This
            // used to render the tmux layout tree as a fixed-height board
            // (prototype §3.9's `display-panes` visualizer), which fought the
            // app's own model: Moshpit never renders splits — the active pane
            // fills the phone screen — so on a phone the geometry carried no
            // information, and a four-pane stack physically overflowed the
            // sheet (user report, 2026-08-17). Finding a pane needs its
            // number (`⌃b q <n>`), its command, its agent state, and a tap
            // target — a row each, and the sheet's ScrollView handles any
            // count.
            let snapshot = controller.snapshot
            VStack(spacing: 2) {
                // Stable order: server order changes between polls, and a
                // list that reshuffles under the user's thumb is worse than
                // one that is slightly stale.
                ForEach(snapshot.activePanes.sorted { $0.index < $1.index }) { pane in
                    let hook = controller.agentHooks[pane.id]
                    let signal = AgentSignal(hook?.state)
                    // Agent name first — the same source the Home tree and
                    // the breadcrumb use. `pane_current_command` is the raw
                    // process name, and Claude Code's versioned install makes
                    // that a bare version number ("2.1.227") — the tree said
                    // "claude" while this sheet said digits (report, build
                    // 359). The process name demotes to the meta line so the
                    // version is still findable, not the identity.
                    let agentName = hook?.agent
                    SheetListRow(
                        icon: pane.index <= 50 ? "\(pane.index).square" : "number.square",
                        name: agentName
                            ?? (pane.command.isEmpty
                                ? String(localized: "pane \(pane.index)") : pane.command),
                        meta: agentName != nil ? pane.command : "",
                        isActive: pane.id == snapshot.activePaneId,
                        statusColor: signal?.color,
                        statusLabel: signal?.label
                    ) {
                        Haptics.select()
                        controller.selectPane(pane.id)
                        dismiss()
                    }
                }
            }
        }
        .task { controller.refresh() }
    }
}
