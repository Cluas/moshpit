import WidgetKit
import SwiftUI
import ActivityKit
import MoshpitKit

#if MOSHPIT_TESTS
// Compiled into the unit-test bundle as well as the widget extension, so a test
// can measure LockScreenView against the height iOS clips at. There the shared
// Island types are members of the Moshpit module rather than files in this
// target, so they need importing; in the extension they are neither.
@testable import Moshpit
#endif

// Vibe Island — Dynamic Island / lock-screen presentation of the aggregate
// agent Live Activity (see Moshpit/Services/Island/AgentActivityMonitor.swift). One
// activity lists every active agent; the pill shows the most-urgent one.
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
// Adaptive (R18 D): deep enough to read on a light lock screen, the colours
// above in the Dynamic Island, which is always dark.
private let teal = AgentPalette.workingAdaptive
private let amber = AgentPalette.attentionAdaptive
private let green = AgentPalette.doneAdaptive

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

/// The Live Activity: the Dynamic Island and the lock screen.
///
/// The expanded island, laid out once (Canary 130's review: the state said
/// twice, names that were window titles and versions, rows cut by the
/// island's curves):
///
/// - **leading**: the companion (the pet, the egg before it hatches, the
///   Moshpit mark with the companion off), then the agent's name and, under
///   it, where it is as Home says it ("moshpit · mac");
/// - **trailing**: the one state label with its timer beside it (or the
///   quiet state alone), vertically centred, inset from the corner;
/// - **center**: nothing (it repeated the state);
/// - **bottom**, inset from the curved corners: what it's asking or doing
///   (amber when it asks), then up to two other agents, one line each
///   (dot · name · place … time or state), each a link to its own pane,
///   then "+N more" when there are more, a 44 pt row that opens Home's
///   "Needs you" (B661), where every agent is listed. The whole island
///   opens the headline's pane. (Canary 141: the Switch button that cycled
///   the headline read as nothing and did nothing visible; it's gone.)
///
/// The compact pill (B597): the companion (or the mark) leading; trailing,
/// the headline's state — anyone needing you first, an amber "!" for one,
/// the waiting hand and their count for several, never a bare number;
/// else working its elapsed time, done a check, idle its dot
/// (``AgentActivityAttributes.ContentState.compactSignal``). Minimal: the
/// companion, else the count waiting, else the mark.
struct MoshpitIslandLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentActivityAttributes.self) { context in
            LockScreenCard(state: context.state, isStale: context.isStale)
        } dynamicIsland: { context in
            let headline = context.state.headline
            // The companion's still (PET-DESIGN §7.1), in place of the dot:
            // the island is always dark; asleep once the activity is stale.
            let still = context.state.companionStill(stale: context.isStale)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    HStack(spacing: 8) {
                        CompanionMark(still: still, size: 36, alwaysDark: true)
                        if let headline {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(headline.name)
                                    .font(.system(size: 15, weight: .semibold))
                                    .foregroundStyle(Color.primary)
                                    .lineLimit(1)
                                Text(headline.whereabouts)
                                    .font(.system(size: 11))
                                    .foregroundStyle(Color.primary.opacity(0.55))
                                    .lineLimit(1)
                            }
                        } else {
                            Text("Moshpit")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(Color.primary)
                        }
                    }
                    .padding(.leading, 6)
                    .frame(maxHeight: .infinity, alignment: .center)
                    .dynamicIsland(verticalPlacement: .belowIfTooWide)
                    .environment(\.colorScheme, .dark)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    if let headline {
                        StateAndTime(agent: headline)
                            .padding(.trailing, 6)
                            .frame(maxHeight: .infinity, alignment: .center)
                            .environment(\.colorScheme, .dark)
                    }
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 4) {
                        // What the headline agent is doing / asking; smart-
                        // truncated so a long command stays readable.
                        if let headline, let detail = headline.detail, !detail.isEmpty {
                            Text(smartTruncate(detail))
                                .font(.system(size: 12, design: .monospaced))
                                .foregroundStyle(headline.state == .attention ? amber : Color.primary.opacity(0.7))
                                .lineLimit(1)
                        }
                        OtherAgentRows(state: context.state)
                        let more = context.state.agents.count - 1 - OtherAgentRows.shown
                        if more > 0 {
                            // Every agent is on Home's "Needs you" (B661): a
                            // whole-width row, 44 pt tall, to tap.
                            Link(destination: URL(string: AgentDeepLink.needsYou)!) {
                                HStack(spacing: 4) {
                                    Text(AppLanguage.string("+\(more) more"))
                                        .font(.system(size: 12, weight: .medium))
                                    Image(systemName: "chevron.right")
                                        .font(.system(size: 10, weight: .semibold))
                                    Spacer(minLength: 0)
                                }
                                .foregroundStyle(Color.primary.opacity(0.55))
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                        }
                        if context.isStale {
                            StaleHint()
                        }
                    }
                    // Clear of the island's curved bottom corners.
                    .padding(.horizontal, 14)
                    .padding(.bottom, 2)
                    .environment(\.colorScheme, .dark)
                }
            } compactLeading: {
                CompanionMark(still: still, size: 24, alwaysDark: true)
                    .environment(\.colorScheme, .dark)
            } compactTrailing: {
                CompactTrailing(state: context.state)
                    .environment(\.colorScheme, .dark)
            } minimal: {
                // The one slot beside another app's activity: the companion
                // (its pose says the state); else the count waiting on you;
                // else the mark.
                Group {
                    if still != nil {
                        CompanionMark(still: still, size: 24, alwaysDark: true)
                    } else if context.state.attentionCount > 0 {
                        Text("\(context.state.attentionCount)")
                            .font(.system(size: 12, weight: .heavy))
                            .monospacedDigit()
                            .foregroundStyle(amber)
                    } else {
                        IslandMark(size: 22)
                    }
                }
                .environment(\.colorScheme, .dark)
            }
            .widgetURL(context.state.headlineDeepLink.flatMap(URL.init(string:)))
            .keylineTint(teal)
        }
    }
}

/// The state's one label and its timer beside it ("WORKING 1:23"), or the
/// quiet state alone ("done").
private struct StateAndTime: View {
    let agent: AgentActivityAttributes.Agent

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(agent.state.label.uppercased())
                .font(.system(size: 10, weight: .bold))
                .kerning(0.4)
                .foregroundStyle(stateColor(agent.state).opacity(0.85))
                .lineLimit(1)
                .fixedSize()
            if agent.state == .working || agent.state == .attention {
                Text(agent.startedAt, style: .timer)
                    .font(.system(size: 14, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(stateColor(agent.state))
                    .frame(maxWidth: 56, alignment: .trailing)
                    .multilineTextAlignment(.trailing)
            }
        }
    }
}

// MARK: - Lock screen

/// The lock-screen card in the lock screen's appearance (R18 D): dark on a
/// dark one as it always was, light on a light one. Its words and dots
/// follow (`Color.primary`, the adaptive state colours).
struct LockScreenCard: View {
    let state: AgentActivityAttributes.ContentState
    var isStale: Bool = false
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        LockScreenView(state: state, isStale: isStale)
            .activityBackgroundTint(scheme == .dark ? Color.black.opacity(0.85) : Color.white.opacity(0.85))
            .activitySystemActionForegroundColor(scheme == .dark ? .white : .black)
    }
}

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
    /// `Tests/MoshpitTests/Island/LockScreenHeightTests.swift` fails if any shape
    /// exceeds the budget, so raising this will be caught rather than shipped.
    static let maxTrailingRows = 2

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Moshpit")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(Color.primary)
                Spacer()
                // A phrase ("1 needs you · 2 working"), not a code fragment —
                // default font, not the mono the rest of the old header ran.
                Text(summary)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Color.primary.opacity(0.6))
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
                // The headline's dot is the companion's still, at 32 pt: no
                // taller than the two lines beside it, so the card keeps its
                // height (LockScreenHeightTests).
                AgentRow(agent: agent, compact: index > 0,
                         companion: index == 0 ? state.companionStill(stale: isStale) : nil)
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

    private var summary: String { Self.summary(for: state) }

    /// The header's phrase ("1 needs you · 2 working"), in the language
    /// Moshpit shows (``AppLanguage``) — not this extension's, which is the
    /// system's — from `bundle`'s catalog.
    static func summary(for state: AgentActivityAttributes.ContentState,
                        language: String = AppLanguage.current(),
                        in bundle: Bundle = .main) -> String {
        var parts: [String] = []
        if state.attentionCount > 0 {
            parts.append(AppLanguage.string("\(state.attentionCount) needs you", language: language, in: bundle))
        }
        if state.workingCount > 0 {
            parts.append(AppLanguage.string("\(state.workingCount) working", language: language, in: bundle))
        }
        let done = state.agents.filter { $0.state == .done }.count
        if done > 0 { parts.append(AppLanguage.string("\(done) done", language: language, in: bundle)) }
        return parts.joined(separator: " · ")
    }
}

private struct AgentRow: View {
    let agent: AgentActivityAttributes.Agent
    /// Header line only — no detail, no control buttons (height budget).
    var compact: Bool = false
    /// The companion's still, for the headline's row.
    var companion: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                // The headline: the companion (or, switched off, the mark);
                // the rest: their state's dot.
                if compact {
                    StateDot(state: agent.state)
                } else {
                    CompanionMark(still: companion, size: 32)
                }
                if compact {
                    // Name and location share ONE line for the trailing agents.
                    // Stacking them cost ~16pt each, and this activity is
                    // already at the height iOS starts clipping (see
                    // LockScreenView) — that budget buys the top and bottom
                    // padding instead, which every row benefits from.
                    Text(agent.name)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Color.primary)
                        .lineLimit(1)
                    Text(agent.whereabouts)
                        .font(.system(size: 11))
                        .foregroundStyle(Color.primary.opacity(0.45))
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
                        Text(agent.name)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(Color.primary)
                            .lineLimit(1)
                        Text(agent.whereabouts)
                            .font(.system(size: 12))
                            .foregroundStyle(Color.primary.opacity(0.5))
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
                    .foregroundStyle(agent.state == .attention ? amber : Color.primary.opacity(0.6))
                    .lineLimit(1)
                    .padding(.leading, 18)
            }
        }
    }
}

/// Preset quick-reply templates offered under Allow/Deny — one tap sends the
/// text + Enter to the agent's pane (see AgentAction.reply).
// MARK: - Shared bits

/// The companion's still where there is one (a ``PixelSprite``: the pet in
/// the agents' pose, or the egg before it hatches), else (the companion off)
/// Moshpit's mark.
private struct CompanionMark: View {
    let still: String?
    let size: CGFloat
    var alwaysDark = false

    var body: some View {
        if let still, PixelSprite(encoded: still) != nil {
            SpriteView(encoded: still, size: size, alwaysDark: alwaysDark)
        } else {
            IslandMark(size: size * 0.85)
                .frame(width: size, height: size)
        }
    }
}

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
        switch state.compactSignal {
        case .waiting(let count) where count > 1:
            // Several waiting: the waiting hand and their count.
            HStack(spacing: 2) {
                Image(systemName: "hand.raised.fill").font(.system(size: 11, weight: .bold))
                Text("\(count)").font(.system(size: 13, weight: .bold)).monospacedDigit()
            }
            .foregroundStyle(amber)
        case .waiting:
            Image(systemName: "exclamationmark")
                .font(.system(size: 13, weight: .heavy))
                .foregroundStyle(amber)
        case .working(let since):
            Text(since, style: .timer)
                .font(.system(size: 12, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(teal)
                .frame(maxWidth: 48)
                .multilineTextAlignment(.trailing)
        case .done:
            Image(systemName: "checkmark")
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(stateColor(.done))
        case .quiet:
            StateDot(state: .idle)
        case nil:
            EmptyView()
        }
    }
}

/// The non-headline agents in the expanded island, one tidy line each:
/// dot · name · where it is (as Home says it) … the timer or the state, each
/// a link to its own pane. At most ``shown``; "+N more" counts the rest.
private struct OtherAgentRows: View {
    let state: AgentActivityAttributes.ContentState
    static let shown = 2

    var body: some View {
        let headlineId = state.headline?.id
        let others = state.agents.filter { $0.id != headlineId }
        ForEach(others.prefix(Self.shown)) { agent in
            Link(destination: URL(string: AgentDeepLink.string(connectionId: agent.connectionId,
                                                                paneId: agent.paneId))
                 ?? URL(string: "moshpit://")!) {
            HStack(spacing: 6) {
                StateDot(state: agent.state)
                Text(agent.name)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Color.primary.opacity(0.85))
                    .lineLimit(1)
                    .fixedSize()
                Text(agent.whereabouts)
                    .font(.system(size: 11))
                    .foregroundStyle(Color.primary.opacity(0.5))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: 6)
                switch agent.state {
                case .working, .attention:
                    Text(agent.startedAt, style: .timer)
                        .font(.system(size: 11, weight: .medium))
                        .monospacedDigit()
                        .foregroundStyle(stateColor(agent.state))
                        .frame(maxWidth: 48, alignment: .trailing)
                        .multilineTextAlignment(.trailing)
                case .done, .idle:
                    Text(agent.state.label)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(stateColor(agent.state))
                }
            }
            .contentShape(Rectangle())
            }
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
            Text(AppLanguage.string("paused — open Moshpit to refresh"))
                .font(.system(size: 10, weight: .medium))
        }
        .foregroundStyle(Color.primary.opacity(0.45))
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
    case .idle:      return Color.primary.opacity(0.45)
    }
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
