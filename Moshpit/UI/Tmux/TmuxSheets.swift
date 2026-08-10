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
            let snapshot = controller.snapshot
            let layout = snapshot.activeWindowId
                .flatMap { snapshot.windows[$0]?.layout }
                .flatMap(TmuxLayoutParser.parse)

            PaneBoard(
                layout: layout,
                panes: snapshot.activePanes,
                activePaneId: snapshot.activePaneId
            ) { paneId in
                Haptics.select()
                controller.selectPane(paneId)
                dismiss()
            }
            .padding(.vertical, 4)
        }
        .task { controller.refresh() }
    }
}

/// Prototype §3.9 — tmux `display-panes` visualizer. Fixed-height board that
/// renders the real layout tree; single pane fills the board, splits divide it
/// proportionally.
struct PaneBoard: View {
    let layout: TmuxLayoutNode?
    let panes: [PaneInfo]
    let activePaneId: String?
    let onSelect: (String) -> Void

    var body: some View {
        Group {
            if let layout, panes.count > 1 {
                BoardNodeView(
                    node: layout,
                    panes: panesById,
                    activePaneId: activePaneId,
                    tag: tag(for: layout),
                    onSelect: onSelect)
            } else if panes.count > 1 {
                // No layout tree, more than one pane — herdr. It ships no tmux
                // layout string (`WindowInfo.layout` is ""), so the parser
                // returns nil and this used to fall through to "show the first
                // pane, call it SINGLE, call it active". Every other pane in
                // the tab was then unreachable from the only sheet that
                // switches panes.
                //
                // herdr does report geometry — each pane's rect lands in
                // `width`/`height` — so split the board the way the server
                // says it is split, by comparing the panes' own dimensions.
                FallbackBoard(
                    panes: panes,
                    activePaneId: activePaneId,
                    onSelect: onSelect)
            } else if let pane = panes.first {
                BoardCell(
                    pane: pane,
                    isActive: true,
                    tag: "SINGLE",
                    onSelect: onSelect)
            } else {
                Text("no panes")
                    .font(Face.mono(11))
                    .foregroundStyle(Ink.meta)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(height: Metrics.paneBoardHeight)
        .padding(6)
        .background(Ink.groupRaised, in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                .strokeBorder(Ink.groupBorder, lineWidth: 1))
    }

    private var panesById: [Int: PaneInfo] {
        Dictionary(uniqueKeysWithValues: panes.compactMap { pane in
            Int(pane.id.dropFirst()).map { ($0, pane) }
        })
    }

    /// Tag the main (largest) pane in a main-vertical style layout.
    private func tag(for node: TmuxLayoutNode) -> Int? {
        if case .row(let kids, let w, _) = node,
           let first = kids.first,
           case .pane(let id, let pw, _) = first,
           Double(pw) / Double(max(w, 1)) > 0.55 {
            return id
        }
        return nil
    }
}

private struct BoardNodeView: View {
    let node: TmuxLayoutNode
    let panes: [Int: PaneInfo]
    let activePaneId: String?
    let tag: Int?
    let onSelect: (String) -> Void

    var body: some View {
        switch node {
        case .pane(let id, _, _):
            BoardCell(
                pane: panes[id] ?? PaneInfo(id: "%\(id)", windowId: ""),
                isActive: activePaneId == "%\(id)",
                tag: tag == id ? "MAIN" : nil,
                onSelect: onSelect)
        case .row(let kids, let w, _):
            HStack(spacing: 6) {
                ForEach(Array(kids.enumerated()), id: \.offset) { _, kid in
                    BoardNodeView(node: kid, panes: panes, activePaneId: activePaneId, tag: tag, onSelect: onSelect)
                        .frame(maxWidth: .infinity)
                        .layoutPriority(Double(kid.width) / Double(max(w, 1)))
                }
            }
        case .column(let kids, _, let h):
            VStack(spacing: 6) {
                ForEach(Array(kids.enumerated()), id: \.offset) { _, kid in
                    BoardNodeView(node: kid, panes: panes, activePaneId: activePaneId, tag: tag, onSelect: onSelect)
                        .frame(maxHeight: .infinity)
                        .layoutPriority(Double(kid.height) / Double(max(h, 1)))
                }
            }
        }
    }
}

/// A pane board for a multiplexer that reports no layout tree.
///
/// herdr is the case: it has real splits but no tmux layout string. Rather
/// than invent a tree, this reads the panes' own reported geometry — all the
/// same width means they are stacked, all the same height means they sit side
/// by side — and falls back to a grid when it is neither. The point is not a
/// pixel-perfect mirror of the server; it is that every pane is on screen and
/// tappable, with the real active one marked.
private struct FallbackBoard: View {
    let panes: [PaneInfo]
    let activePaneId: String?
    let onSelect: (String) -> Void

    /// Panes in a stable order — server order changes between polls, and a
    /// board that reshuffles itself under the user's thumb is worse than one
    /// that is slightly wrong.
    private var ordered: [PaneInfo] { panes.sorted { $0.index < $1.index } }

    private var isColumn: Bool {
        let widths = Set(panes.map(\.width))
        let heights = Set(panes.map(\.height))
        // Equal widths, differing heights → stacked vertically.
        return widths.count == 1 && heights.count > 1
    }

    private var isRow: Bool {
        let widths = Set(panes.map(\.width))
        let heights = Set(panes.map(\.height))
        return heights.count == 1 && widths.count > 1
    }

    var body: some View {
        if isColumn {
            VStack(spacing: 4) { cells }
        } else if isRow {
            HStack(spacing: 4) { cells }
        } else {
            // Mixed or missing geometry (herdr 0.7.3 reports none at all):
            // a two-column grid keeps every pane reachable.
            let columns = [GridItem(.flexible(), spacing: 4), GridItem(.flexible(), spacing: 4)]
            LazyVGrid(columns: columns, spacing: 4) { cells }
        }
    }

    @ViewBuilder
    private var cells: some View {
        ForEach(ordered) { pane in
            BoardCell(
                pane: pane,
                isActive: pane.id == activePaneId || (activePaneId == nil && pane.isActive),
                tag: nil,
                onSelect: onSelect)
        }
    }
}

private struct BoardCell: View {
    let pane: PaneInfo
    let isActive: Bool
    let tag: String?
    let onSelect: (String) -> Void

    /// The window-local pane index — what `display-panes` shows and what
    /// `⌃b q <n>` jumps to. NOT the global "%41" pane id.
    private var number: String {
        String(pane.index)
    }

    var body: some View {
        Button {
            onSelect(pane.id)
        } label: {
            ZStack(alignment: .topTrailing) {
                VStack(spacing: 4) {
                    Text(number)
                        .font(Face.display(34, .semibold))
                        .foregroundStyle(isActive ? Ink.accent : Ink.termMuted)
                    if !pane.command.isEmpty {
                        Text(pane.command)
                            .font(Face.mono(11))
                            .foregroundStyle(isActive ? Ink.primary : Ink.termMuted)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(EdgeInsets(top: 14, leading: 10, bottom: 14, trailing: 10))

                if let tag {
                    Text(tag)
                        .font(Face.mono(9))
                        .kerning(1.26)
                        .foregroundStyle(isActive ? Ink.paneActiveTag : Ink.meta)
                        .padding(EdgeInsets(top: 6, leading: 0, bottom: 0, trailing: 8))
                }
            }
            .background(
                isActive ? Ink.paneActiveBG : Ink.group,
                in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                    .strokeBorder(
                        isActive ? Ink.paneActiveRing : Ink.groupBorder,
                        lineWidth: isActive ? 1.5 : 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
