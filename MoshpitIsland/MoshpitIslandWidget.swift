import WidgetKit
import SwiftUI
import ActivityKit
import AppIntents

/// Vibe Island — Dynamic Island / lock-screen presentation of the aggregate
/// agent Live Activity (see Moshpit/Island/AgentActivityMonitor.swift). One
/// activity lists every active agent; the pill shows the most-urgent one.
@main
struct MoshpitIslandBundle: WidgetBundle {
    var body: some Widget {
        MoshpitIslandLiveActivity()
        MoshpitStatusWidget()   // home- / lock-screen agent status (App Group pull)
    }
}

// State colours come from the shared palette so the island can never drift
// from the app's dots again (it did: amber here was more saturated, and the
// app's "working" was the theme accent rather than this teal).
private let teal = AgentPalette.working
private let amber = AgentPalette.attention

struct MoshpitIslandLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentActivityAttributes.self) { context in
            LockScreenView(state: context.state, isStale: context.isStale)
                .activityBackgroundTint(Color.black.opacity(0.85))
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            let headline = context.state.headline
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    HStack(spacing: 6) {
                        StateDot(state: headline?.state ?? .idle)
                        Text(headline.map { displayCommand($0) } ?? "Moshpit")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                    }
                    .padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    if let headline {
                        TrailingStatus(agent: headline).padding(.trailing, 4)
                    }
                }
                DynamicIslandExpandedRegion(.center) {
                    // Prime real estate for the headline's state label + where
                    // it lives — previously this whole region was empty.
                    if let headline {
                        VStack(spacing: 1) {
                            Text(headline.state.label)
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(stateColor(headline.state))
                            Text(headline.location)
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundStyle(.white.opacity(0.55))
                                .lineLimit(1)
                        }
                    }
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 4) {
                        // What the headline agent is doing / asking — the actual
                        // content; give it room instead of a single clipped line.
                        if let headline, let detail = headline.detail, !detail.isEmpty {
                            Text(detail)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(headline.state == .attention ? amber : .white.opacity(0.7))
                                .lineLimit(2)
                        }
                        // The OTHER agents as real rows (dot · name · live timer),
                        // not an opaque "+N more".
                        OtherAgentRows(state: context.state)
                        if context.isStale {
                            StaleHint()
                        }
                        // Control the headline agent inline (answer / stop).
                        if let headline {
                            AgentControls(agent: headline).padding(.top, 4)
                        }
                        // Several agents: let the pill cycle through them.
                        if context.state.agents.count > 1 {
                            SwitchAgentButton().padding(.top, 2)
                        }
                    }
                    .padding(.horizontal, 4)
                }
            } compactLeading: {
                StateDot(state: headline?.state ?? .idle)
            } compactTrailing: {
                CompactTrailing(state: context.state)
            } minimal: {
                // The one slot visible alongside another app's activity — show
                // the strongest signal we have, not just a dot: attention count
                // in amber, else the working dot.
                if context.state.attentionCount > 0 {
                    Text("\(context.state.attentionCount)")
                        .font(.system(size: 12, weight: .heavy, design: .monospaced))
                        .foregroundStyle(amber)
                } else {
                    StateDot(state: headline?.state ?? .idle)
                }
            }
            .widgetURL(context.state.headlineDeepLink.flatMap(URL.init(string:)))
            .keylineTint(teal)
        }
    }
}

// MARK: - Lock screen

private struct LockScreenView: View {
    let state: AgentActivityAttributes.ContentState
    var isStale: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Moshpit")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.white)
                Spacer()
                Text(summary)
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.6))
            }
            // Full controls only for the most-urgent agent — a tall activity
            // gets CLIPPED by iOS, and what fell off the bottom was exactly the
            // second agent's Allow/Deny. Others collapse to a status line.
            ForEach(Array(state.agents.enumerated()), id: \.element.id) { index, agent in
                AgentRow(agent: agent, compact: index > 0)
            }
            if isStale {
                StaleHint()
            }
        }
        // More top and bottom than sides: the system draws this on a
        // translucent card whose edge sits right against the text, and the
        // vertical crowding is what reads as unfinished. Affordable only
        // because the trailing agent rows above collapsed to one line each —
        // padding added on its own would have pushed more content under the
        // clip instead of framing it.
        .padding(.horizontal, 14)
        .padding(.vertical, 18)
    }

    private var summary: String {
        var parts: [String] = []
        if state.attentionCount > 0 { parts.append("\(state.attentionCount) needs you") }
        if state.workingCount > 0 { parts.append("\(state.workingCount) working") }
        let done = state.agents.filter { $0.state == .done }.count
        if done > 0 { parts.append("\(done) done") }
        return parts.isEmpty ? "" : parts.joined(separator: " · ")
    }
}

private struct AgentRow: View {
    let agent: AgentActivityAttributes.Agent
    /// Header line only — no detail, no control buttons (height budget).
    var compact: Bool = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                StateDot(state: agent.state)
                if compact {
                    // Name and location share ONE line for the trailing agents.
                    // Stacking them cost ~16pt each, and this activity is
                    // already at the height iOS starts clipping (see
                    // LockScreenView) — that budget buys the top and bottom
                    // padding instead, which every row benefits from.
                    Text(displayCommand(agent))
                        .font(.system(size: 13, weight: .semibold, design: .monospaced))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    Text(agent.location)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.white.opacity(0.45))
                        .lineLimit(1)
                        .layoutPriority(-1)
                } else {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(displayCommand(agent))
                            .font(.system(size: 14, weight: .semibold, design: .monospaced))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        Text(agent.location)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.white.opacity(0.55))
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 4)
                TrailingStatus(agent: agent)
            }
            // What the agent is doing / asking (hook @moshpit_title) — amber when
            // it's the thing you're being asked to approve.
            if !compact, let detail = agent.detail, !detail.isEmpty {
                Text(detail)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(agent.state == .attention ? amber : .white.opacity(0.6))
                    .lineLimit(2)
                    .padding(.leading, 18)
            }
            // T1 control surface — answer the prompt (Allow/Deny/quick-reply) or
            // stop a running agent, from the lock screen, without opening the app.
            if !compact {
                AgentControls(agent: agent).padding(.leading, 18)
            }
        }
    }
}

/// Preset quick-reply templates offered under Allow/Deny — one tap sends the
/// text + Enter to the agent's pane (see AgentAction.reply).
private let quickReplies = ["yes", "continue"]

/// Allow / Deny + quick replies straight from the Live Activity. The tap runs
/// `AgentApprovalIntent` in the app process, which sends the keystroke to the
/// agent's tmux pane.
private struct ApprovalButtons: View {
    let agent: AgentActivityAttributes.Agent

    var body: some View {
        VStack(spacing: 6) {
            HStack(spacing: 8) {
                Button(intent: AgentApprovalIntent(action: .allow,
                                                   connectionId: agent.connectionId,
                                                   paneId: agent.paneId)) {
                    Label("Allow", systemImage: "checkmark")
                        .font(.system(size: 12, weight: .semibold))
                        .frame(maxWidth: .infinity)
                }
                .tint(teal)
                Button(intent: AgentApprovalIntent(action: .deny,
                                                   connectionId: agent.connectionId,
                                                   paneId: agent.paneId)) {
                    Label("Deny", systemImage: "xmark")
                        .font(.system(size: 12, weight: .semibold))
                        .frame(maxWidth: .infinity)
                }
                .tint(amber)
            }
            HStack(spacing: 6) {
                ForEach(quickReplies, id: \.self) { reply in
                    Button(intent: AgentApprovalIntent(action: .reply,
                                                       connectionId: agent.connectionId,
                                                       paneId: agent.paneId,
                                                       text: reply)) {
                        Text(reply)
                            .font(.system(size: 11, weight: .medium))
                            .frame(maxWidth: .infinity)
                    }
                    .tint(.white.opacity(0.22))
                }
            }
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.small)
        .foregroundStyle(.white)
    }
}

/// Stop a running agent (Ctrl-C) from the Live Activity.
private struct InterruptButton: View {
    let agent: AgentActivityAttributes.Agent

    var body: some View {
        Button(intent: AgentApprovalIntent(action: .interrupt,
                                           connectionId: agent.connectionId,
                                           paneId: agent.paneId)) {
            Label("Stop", systemImage: "stop.fill")
                .font(.system(size: 12, weight: .semibold))
                .frame(maxWidth: .infinity)
        }
        .tint(amber)
        .buttonStyle(.borderedProminent)
        .controlSize(.small)
        .foregroundStyle(.white)
    }
}

/// Cycle which agent the Dynamic Island pill shows (when several are active).
private struct SwitchAgentButton: View {
    var body: some View {
        Button(intent: AgentCycleIntent()) {
            Label("Switch", systemImage: "arrow.left.arrow.right")
                .font(.system(size: 11, weight: .medium))
        }
        .tint(.white.opacity(0.22))
        .buttonStyle(.borderedProminent)
        .controlSize(.small)
        .foregroundStyle(.white)
    }
}

/// The control row for an agent given its state: answer when it needs you,
/// stop it while it's working.
private struct AgentControls: View {
    let agent: AgentActivityAttributes.Agent

    var body: some View {
        switch agent.state {
        case .attention: ApprovalButtons(agent: agent)
        case .working:   InterruptButton(agent: agent)
        case .done, .idle: EmptyView()
        }
    }
}

// MARK: - Shared bits

private struct TrailingStatus: View {
    let agent: AgentActivityAttributes.Agent

    var body: some View {
        switch agent.state {
        case .working, .attention:
            // Live elapsed timer, counting up since the state began.
            Text(agent.startedAt, style: .timer)
                .font(.system(size: 12, weight: .medium, design: .monospaced))
                .foregroundStyle(stateColor(agent.state))
                .monospacedDigit()
                .frame(minWidth: 44, alignment: .trailing)
        case .done, .idle:
            Text(agent.state.label)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundStyle(stateColor(agent.state))
        }
    }
}

private struct CompactTrailing: View {
    let state: AgentActivityAttributes.ContentState

    var body: some View {
        if state.attentionCount > 0 {
            // Needs-you owns the pill: a single "!" for one, the count for several.
            Group {
                if state.attentionCount == 1 {
                    Image(systemName: "exclamationmark").font(.system(size: 13, weight: .heavy))
                } else {
                    Text("\(state.attentionCount)")
                        .font(.system(size: 13, weight: .bold, design: .monospaced))
                }
            }
            .foregroundStyle(amber)
        } else if let headline = state.headline, headline.state == .working,
                  state.workingCount == 1 {
            // A LIVE ticking timer — the pill actually shows something happening
            // (the old 4-char command prefix told you nothing you didn't know).
            Text(headline.startedAt, style: .timer)
                .font(.system(size: 12, weight: .semibold, design: .monospaced))
                .foregroundStyle(teal)
                .monospacedDigit()
                .frame(maxWidth: 48)
                .multilineTextAlignment(.trailing)
        } else if state.workingCount > 1 {
            Text("\(state.workingCount)")
                .font(.system(size: 13, weight: .bold, design: .monospaced))
                .foregroundStyle(.white)
        } else if let headline = state.headline, headline.state == .done {
            Image(systemName: "checkmark")
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(stateColor(.done))
        } else if let headline = state.headline {
            Text(String(headline.command.prefix(4)))
                .font(.system(size: 12, weight: .bold, design: .monospaced))
                .foregroundStyle(.white)
        }
    }
}

/// The non-headline agents as compact live rows in the expanded island —
/// each with its own state dot, name, window and ticking timer. Replaces the
/// old "+N more", which hid everything useful.
private struct OtherAgentRows: View {
    let state: AgentActivityAttributes.ContentState

    var body: some View {
        let headlineId = state.headline?.id
        let others = state.agents.filter { $0.id != headlineId }
        ForEach(others.prefix(2)) { agent in
            HStack(spacing: 6) {
                StateDot(state: agent.state)
                Text(displayCommand(agent))
                    .font(.system(size: 11, weight: .semibold, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.85))
                    .lineLimit(1)
                Text(agent.location)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.45))
                    .lineLimit(1)
                Spacer(minLength: 4)
                switch agent.state {
                case .working, .attention:
                    Text(agent.startedAt, style: .timer)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundStyle(stateColor(agent.state))
                        .monospacedDigit()
                        .frame(maxWidth: 44)
                        .multilineTextAlignment(.trailing)
                case .done, .idle:
                    Text(agent.state.label)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundStyle(stateColor(agent.state))
                }
            }
        }
        if others.count > 2 {
            Text("+\(others.count - 2) more")
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.white.opacity(0.4))
        }
    }
}

/// Honest staleness: our 2s poll pushes island updates only while iOS keeps the
/// app alive — once suspended, the data freezes. iOS flips `context.isStale`
/// after the content's staleDate; tell the user instead of showing a frozen
/// "working" forever.
private struct StaleHint: View {
    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "pause.circle")
                .font(.system(size: 10))
            Text("paused — open Moshpit to refresh")
                .font(.system(size: 10, design: .monospaced))
        }
        .foregroundStyle(.white.opacity(0.45))
    }
}

private struct StateDot: View {
    let state: AgentActivityAttributes.AgentState

    var body: some View {
        // Live Activity / widget views render as static snapshots (no
        // repeatForever animation), so "alive" is conveyed by a stronger glow on
        // the active states rather than a pulse that wouldn't run.
        let active = state == .working || state == .attention
        Circle()
            .fill(stateColor(state))
            .frame(width: 8, height: 8)
            .shadow(color: stateColor(state).opacity(active ? 0.9 : 0.5),
                    radius: active ? 5 : 2)
    }
}

private func stateColor(_ state: AgentActivityAttributes.AgentState) -> Color {
    switch state {
    case .working:   return teal
    case .attention: return amber
    case .done:      return AgentPalette.done
    case .idle:      return Color.white.opacity(0.45)
    }
}

private func displayCommand(_ agent: AgentActivityAttributes.Agent) -> String {
    agent.command.isEmpty ? "shell" : agent.command
}

