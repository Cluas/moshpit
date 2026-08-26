import WidgetKit
import SwiftUI
import ActivityKit
import AppIntents

#if MOSHPIT_TESTS
// Compiled into the unit-test bundle as well as the widget extension, so a test
// can measure LockScreenView against the height iOS clips at. There the shared
// Island types are members of the Moshpit module rather than files in this
// target, so they need importing; in the extension they are neither.
@testable import Moshpit
#endif

/// Vibe Island — Dynamic Island / lock-screen presentation of the aggregate
/// agent Live Activity (see Moshpit/Island/AgentActivityMonitor.swift). One
/// activity lists every active agent; the pill shows the most-urgent one.
// Not compiled into the test bundle: a test bundle has no @main, and the other
// widget in it lives in a file the tests have no reason to pull in. The views
// below are the point.
#if !MOSHPIT_TESTS
@main
struct MoshpitIslandBundle: WidgetBundle {
    var body: some Widget {
        MoshpitIslandLiveActivity()
        MoshpitStatusWidget()   // home- / lock-screen agent status (App Group pull)
    }
}
#endif

// State colours come from the shared palette so the island can never drift
// from the app's dots again (it did: amber here was more saturated, and the
// app's "working" was the theme accent rather than this teal).
private let teal = AgentPalette.working
private let amber = AgentPalette.attention
private let green = AgentPalette.done

// MARK: - Typography
//
// One rule, applied everywhere below: the agent's NAME and its STATE are the
// thing you glance at the lock screen for, so they get the system's default
// font at real weight/size. Everything else — a location breadcrumb, a raw
// command line, a hook's detail string — is auxiliary and stays small,
// monospaced and muted, the same voice the in-app pane rows already use for
// that kind of content. Before this split, the card ran one flavour of
// cramped monospace top to bottom and nothing told the eye where to land.
// Ticking numbers (the live timers) get `.monospacedDigit()` instead of a
// full mono font — that keeps digits from jittering sideways without
// dragging the rest of the type family along with them.

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
                        StatusBadge(agent: headline, compact: true).padding(.trailing, 4)
                    }
                }
                DynamicIslandExpandedRegion(.center) {
                    // Prime real estate for the headline's state label + where
                    // it lives — previously this whole region was empty.
                    if let headline {
                        VStack(spacing: 2) {
                            Text(headline.state.label.uppercased())
                                .font(.system(size: 10, weight: .bold))
                                .kerning(0.5)
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
                        // Smart-truncated: a raw "claude --resume <uuid>" hook
                        // title used to just get chopped mid-UUID by lineLimit.
                        if let headline, let detail = headline.detail, !detail.isEmpty {
                            Text(smartTruncate(detail))
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(headline.state == .attention ? amber : .white.opacity(0.7))
                                .lineLimit(1)
                        }
                        // The OTHER agents as real rows (dot · name · live timer),
                        // not an opaque "+N more".
                        OtherAgentRows(state: context.state)
                        if context.isStale {
                            StaleHint()
                        }
                        // Control the headline agent inline (answer / stop).
                        if let headline {
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
                        .font(.system(size: 12, weight: .heavy))
                        .monospacedDigit()
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

struct LockScreenView: View {
    let state: AgentActivityAttributes.ContentState
    var isStale: Bool = false

    /// How many agents beyond the headline get a line of their own.
    ///
    /// Chosen from measurements, not taste. iOS clips this card at 160pt and
    /// reports nothing, so the number comes from rendering the real view:
    /// an `attention` headline is ~90pt and each collapsed row ~24pt, which
    /// leaves room for two. Three measured 180pt for four agents — over, and
    /// silently sliced.
    ///
    /// `MoshpitTests/Island/LockScreenHeightTests.swift` fails if any shape
    /// exceeds the budget, so raising this will be caught rather than shipped.
    static let maxTrailingRows = 2

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Moshpit")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.white)
                Spacer()
                // A phrase ("1 needs you · 2 working"), not a code fragment —
                // default font, not the mono the rest of the old header ran.
                Text(summary)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.white.opacity(0.6))
            }
            // Full controls only for the most-urgent agent; the rest collapse to
            // one line each — and CRUCIALLY, only as many of them as fit.
            //
            // iOS clips this card at 160pt and reports nothing, so an unbounded
            // ForEach here meant the card grew with the number of agents and got
            // sliced: 138pt for one, 171 for two, 435 for ten. Clipped at both
            // ends, because the system centres what it cannot fit — which is how
            // a real phone came back with the header gone off the top AND an
            // agent row cut off the bottom.
            //
            // `maxTrailingRows` is small because the headline agent's own block
            // is most of the budget. What the held-back rows would have said,
            // the header summary already says as a count.
            ForEach(Array(state.agents.prefix(1 + Self.maxTrailingRows).enumerated()),
                    id: \.element.id) { index, agent in
                AgentRow(agent: agent, compact: index > 0)
            }
            if isStale {
                StaleHint()
            }
        }
        // Still more top and bottom than sides — the system draws this on a
        // translucent card whose edge sits right against the text, and the
        // vertical crowding is what reads as unfinished. 18 was the earlier
        // value, chosen when the card's height was believed rather than
        // measured; it was being paid for with content that fell off the end.
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
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
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                StateDot(state: agent.state)
                if compact {
                    // Name and location share ONE line for the trailing agents.
                    // Stacking them cost ~16pt each, and this activity is
                    // already at the height iOS starts clipping (see
                    // LockScreenView) — that budget buys the top and bottom
                    // padding instead, which every row benefits from.
                    Text(displayCommand(agent))
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    Text(agent.location)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.white.opacity(0.4))
                        .lineLimit(1)
                        .layoutPriority(-1)
                } else {
                    VStack(alignment: .leading, spacing: 2) {
                        // The name — bigger and NOT monospaced, so it reads as
                        // the headline of the row rather than one more line of
                        // code alongside the location below it.
                        //
                        // This was briefly merged onto one line to claw back
                        // ~18pt for the Allow/Deny row. With the buttons gone the
                        // card has the height again, and the hierarchy is worth
                        // more than the two lines cost: telling you which agent
                        // wants what IS the card's whole job now.
                        Text(displayCommand(agent))
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        Text(agent.location)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.white.opacity(0.45))
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 6)
                StatusBadge(agent: agent, compact: compact, singleLine: compact)
            }
            // What the agent is doing / asking (hook @moshpit_title) — amber
            // when it's the thing you're being asked to approve. Smart-
            // truncated so a long shell command survives as something
            // readable instead of getting chopped mid-token by lineLimit.
            if !compact, let detail = agent.detail, !detail.isEmpty {
                Text(smartTruncate(detail))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(agent.state == .attention ? amber : .white.opacity(0.6))
                    .lineLimit(1)
                    .padding(.leading, 18)
            }
        }
    }
}

/// Preset quick-reply templates offered under Allow/Deny — one tap sends the
/// text + Enter to the agent's pane (see AgentAction.reply).
/// Cycle which agent the Dynamic Island pill shows (when several are active).
private struct SwitchAgentButton: View {
    var body: some View {
        Button(intent: AgentCycleIntent()) {
            HStack(spacing: 4) {
                Image(systemName: "arrow.left.arrow.right")
                    .font(.system(size: 9, weight: .semibold))
                Text(String(localized: "Switch"))
                    .font(.system(size: 11, weight: .medium))
            }
            .foregroundStyle(.white.opacity(0.75))
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .overlay(Capsule().strokeBorder(.white.opacity(0.22), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Shared bits

/// The name row's trailing signal. For the two live states this is a
/// labelled, live-updating timer — a small uppercase caption ABOVE the
/// number so a system-rendered timer (which iOS sizes generously for
/// legibility, bigger than the surrounding text) reads as a deliberate
/// stopwatch instead of a stray oversized numeral with nothing around it.
/// For the two quiet states it's a plain tonal badge.
private struct StatusBadge: View {
    let agent: AgentActivityAttributes.Agent
    /// Smaller type. Used by the Dynamic Island's expanded trailing region,
    /// where space is tight sideways but the two-line badge still fits.
    var compact: Bool = false
    /// Drop the state caption and show the timer alone.
    ///
    /// Separate from `compact` because the two constraints are different: the
    /// island is short on WIDTH, the lock-screen card is short on HEIGHT. Folding
    /// them into one flag took the caption off the island too, where nothing was
    /// asking for it back.
    var singleLine: Bool = false

    var body: some View {
        switch agent.state {
        case .working, .attention:
            if singleLine {
                // Timer only. The label above it cost the row its second line —
                // ~10pt of a 160pt card, per trailing agent — and it was the
                // least of three places already saying the same thing: the dot
                // at the head of the row carries the state as colour, and the
                // header counts them ("2 needs you · 1 working").
                Text(agent.startedAt, style: .timer)
                    .font(.system(size: 11, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(stateColor(agent.state))
            } else {
                VStack(alignment: .trailing, spacing: 0) {
                    Text(agent.state.label.uppercased())
                        .font(.system(size: compact ? 7 : 8, weight: .bold))
                        .kerning(0.4)
                        .foregroundStyle(stateColor(agent.state).opacity(0.8))
                    Text(agent.startedAt, style: .timer)
                        .font(.system(size: compact ? 11 : 13, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(stateColor(agent.state))
                }
            }
        case .done, .idle:
            Text(agent.state.label)
                .font(.system(size: compact ? 10 : 11, weight: .semibold))
                .foregroundStyle(stateColor(agent.state))
                .padding(.horizontal, 7)
                .padding(.vertical, 3)
                .background(stateColor(agent.state).opacity(0.14), in: Capsule())
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
                        .font(.system(size: 13, weight: .bold))
                        .monospacedDigit()
                }
            }
            .foregroundStyle(amber)
        } else if let headline = state.headline, headline.state == .working,
                  state.workingCount == 1 {
            // A LIVE ticking timer — the pill actually shows something happening
            // (the old 4-char command prefix told you nothing you didn't know).
            Text(headline.startedAt, style: .timer)
                .font(.system(size: 12, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(teal)
                .frame(maxWidth: 48)
                .multilineTextAlignment(.trailing)
        } else if state.workingCount > 1 {
            Text("\(state.workingCount)")
                .font(.system(size: 13, weight: .bold))
                .monospacedDigit()
                .foregroundStyle(.white)
        } else if let headline = state.headline, headline.state == .done {
            Image(systemName: "checkmark")
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(stateColor(.done))
        } else if let headline = state.headline {
            // The pill's smallest slot — a bare 4-char fragment of the raw
            // command is the one place a code-flavoured mono clip is actually
            // the right call, since there's no room for anything else.
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
                    .font(.system(size: 11, weight: .semibold))
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
                        .font(.system(size: 10, weight: .medium))
                        .monospacedDigit()
                        .foregroundStyle(stateColor(agent.state))
                        .frame(maxWidth: 44)
                        .multilineTextAlignment(.trailing)
                case .done, .idle:
                    Text(agent.state.label)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(stateColor(agent.state))
                }
            }
        }
        if others.count > 2 {
            Text("+\(others.count - 2) more")
                .font(.system(size: 10, weight: .medium))
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
            // A sentence, not a code fragment — default font, matching the
            // header summary this sits directly under.
            Text("paused — open Moshpit to refresh")
                .font(.system(size: 10, weight: .medium))
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
            .overlay(
                // Attention gets a ring on top of the glow — the one state
                // that's actually waiting on a human shouldn't lean on the
                // same "dot + colour" language as a dot that's merely running.
                Circle()
                    .strokeBorder(amber.opacity(state == .attention ? 0.55 : 0), lineWidth: 1.5)
                    .frame(width: 14, height: 14)
            )
    }
}

private func stateColor(_ state: AgentActivityAttributes.AgentState) -> Color {
    switch state {
    case .working:   return teal
    case .attention: return amber
    case .done:      return green
    case .idle:      return Color.white.opacity(0.45)
    }
}

private func displayCommand(_ agent: AgentActivityAttributes.Agent) -> String {
    agent.command.isEmpty ? "shell" : agent.command
}

/// Shortens long opaque tokens — session ids, hashes, absolute paths —
/// inside a command/detail string instead of letting `.lineLimit` chop
/// wherever the text happened to run out of room. That's exactly how
/// "claude --resume e7463a11-2235-4ac4-930f-dff3202cd64e" turned into a
/// truncated UUID with nothing readable in front of it on the lock screen:
/// the short, meaningful words (the command, its flags) got pushed off by
/// one long unbroken token. Short words pass through untouched; only words
/// past the budget get shortened, head + tail, since the middle of a UUID or
/// hash carries no information anyone reads at a glance.
private func smartTruncate(_ text: String, tokenBudget: Int = 14) -> String {
    guard text.count > tokenBudget else { return text }
    return text
        .split(separator: " ", omittingEmptySubsequences: false)
        .map { word -> String in
            guard word.count > tokenBudget else { return String(word) }
            if word.contains("/") {
                // Path-like: the filename at the end is what identifies it —
                // the directories in front of it usually aren't.
                let name = word.split(separator: "/").last.map(String.init) ?? String(word)
                return name.count < word.count ? "…/\(name)" : name
            }
            return "\(word.prefix(6))…\(word.suffix(4))"
        }
        .joined(separator: " ")
}

// MARK: - Previews
//
// Xcode Previews for a Live Activity render this exact widget against a
// fabricated attributes/content-state pair — no device, no real tmux session
// needed to see whether a layout change holds up. `previewAttention` is the
// bug report itself: a long "claude --resume <uuid>" hook title, which is
// what `smartTruncate` above exists to keep readable.
#if DEBUG
private extension AgentActivityAttributes {
    static let preview = AgentActivityAttributes()
}

private extension AgentActivityAttributes.Agent {
    static func preview(state: AgentActivityAttributes.AgentState,
                         command: String = "claude",
                         detail: String? = nil,
                         location: String = "herdr · ~ · wQ · Tab 1",
                         secondsAgo: TimeInterval = 90) -> Self {
        .init(id: UUID().uuidString, connectionId: UUID().uuidString, paneId: "%1",
              command: command, location: location, detail: detail, state: state,
              startedAt: Date().addingTimeInterval(-secondsAgo))
    }
}

private extension AgentActivityAttributes.ContentState {
    static let previewAttention = AgentActivityAttributes.ContentState(
        agents: [.preview(state: .attention, command: "claude",
                          detail: "claude --resume e7463a11-2235-4ac4-930f-dff3202cd64e",
                          secondsAgo: 313)],
        workingCount: 0, attentionCount: 1, headlineDeepLink: nil)

    static let previewWorking = AgentActivityAttributes.ContentState(
        agents: [.preview(state: .working, command: "cargo",
                          detail: "Bash: cargo test --workspace", secondsAgo: 47)],
        workingCount: 1, attentionCount: 0, headlineDeepLink: nil)

    static let previewMulti = AgentActivityAttributes.ContentState(
        agents: [
            .preview(state: .attention, command: "claude", detail: "Edit: Package.swift", secondsAgo: 20),
            .preview(state: .working, command: "codex", location: "work · 2:rednote", secondsAgo: 340),
            .preview(state: .done, command: "node", location: "home · ~/api", secondsAgo: 900),
        ],
        workingCount: 1, attentionCount: 1, headlineDeepLink: nil)
}

#Preview("Lock Screen", as: .content, using: AgentActivityAttributes.preview) {
    MoshpitIslandLiveActivity()
} contentStates: {
    AgentActivityAttributes.ContentState.previewAttention
    AgentActivityAttributes.ContentState.previewWorking
    AgentActivityAttributes.ContentState.previewMulti
}

#Preview("Dynamic Island Compact", as: .dynamicIsland(.compact), using: AgentActivityAttributes.preview) {
    MoshpitIslandLiveActivity()
} contentStates: {
    AgentActivityAttributes.ContentState.previewAttention
    AgentActivityAttributes.ContentState.previewWorking
}

#Preview("Dynamic Island Minimal", as: .dynamicIsland(.minimal), using: AgentActivityAttributes.preview) {
    MoshpitIslandLiveActivity()
} contentStates: {
    AgentActivityAttributes.ContentState.previewAttention
}

#Preview("Dynamic Island Expanded", as: .dynamicIsland(.expanded), using: AgentActivityAttributes.preview) {
    MoshpitIslandLiveActivity()
} contentStates: {
    AgentActivityAttributes.ContentState.previewMulti
}
#endif
