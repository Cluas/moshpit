import WidgetKit
import SwiftUI

/// Home- and lock-screen Widget showing live Ringdown agent status. Unlike the Live
/// Activity (push-updated), this PULLs from the App Group snapshot the app writes
/// (see AgentWidgetStore); the app nudges WidgetCenter on every change. Tapping
/// opens the headline agent's pane.
private let wTeal = Color(red: 0.37, green: 0.89, blue: 0.85)
private let wAmber = Color(red: 1.0, green: 0.62, blue: 0.04)
private let wGreen = Color(red: 0.55, green: 0.85, blue: 0.55)

private func color(forState raw: String) -> Color {
    switch raw {
    case "working":   return wTeal
    case "attention": return wAmber
    case "done":      return wGreen
    default:          return .white.opacity(0.45)
    }
}

struct AgentStatusEntry: TimelineEntry {
    let date: Date
    let state: AgentWidgetState
    /// Snapshot is older than the app's live-update horizon — the app has been
    /// suspended and every state/timer shown would be a lie.
    var isStale: Bool = false
}

struct AgentStatusProvider: TimelineProvider {
    /// How old a snapshot may be before the widget stops claiming it's live.
    static let staleAfter: TimeInterval = 180

    func placeholder(in context: Context) -> AgentStatusEntry {
        AgentStatusEntry(date: Date(), state: .empty)
    }
    func getSnapshot(in context: Context, completion: @escaping (AgentStatusEntry) -> Void) {
        let state = AgentWidgetStore.read()
        completion(AgentStatusEntry(date: Date(), state: state,
                                    isStale: Date().timeIntervalSince(state.updatedAt) > Self.staleAfter))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<AgentStatusEntry>) -> Void) {
        // The app reloads timelines on every state change; this longer fallback
        // just keeps the widget from drifting stale if the app isn't running.
        // Two entries: the fresh view now, and — WITHOUT any new push — the
        // stale variant once the snapshot passes the horizon, so a suspended
        // app can't leave "2 working" frozen on the home screen forever.
        let state = AgentWidgetStore.read()
        let alreadyStale = Date().timeIntervalSince(state.updatedAt) > Self.staleAfter
        var entries = [AgentStatusEntry(date: Date(), state: state, isStale: alreadyStale)]
        if !alreadyStale {
            entries.append(AgentStatusEntry(
                date: state.updatedAt.addingTimeInterval(Self.staleAfter),
                state: state, isStale: true))
        }
        completion(Timeline(entries: entries, policy: .after(Date().addingTimeInterval(900))))
    }
}

struct RingdownStatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "RingdownAgentStatus", provider: AgentStatusProvider()) { entry in
            AgentStatusWidgetView(state: entry.state, isStale: entry.isStale)
                .widgetURL(entry.state.headlineDeepLink.flatMap(URL.init(string:)))
        }
        .configurationDisplayName("Agent Status")
        .description("Live status of your Ringdown agents.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular,
                            .accessoryCircular, .accessoryInline])
    }
}

struct AgentStatusWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let state: AgentWidgetState
    var isStale: Bool = false

    var body: some View {
        content
            .containerBackground(for: .widget) {
                if family == .systemSmall || family == .systemMedium {
                    Color.black.opacity(0.92)
                } else {
                    Color.clear
                }
            }
    }

    @ViewBuilder private var content: some View {
        switch family {
        case .accessoryCircular:    circular
        case .accessoryRectangular: rectangular
        case .accessoryInline:      inline
        case .systemMedium:         medium
        default:                    small
        }
    }

    /// State dot colour, dimmed uniformly when the snapshot is stale —
    /// a suspended app's "working" teal would be a lie.
    private func dotColor(_ raw: String) -> Color {
        isStale ? Color.white.opacity(0.35) : color(forState: raw)
    }

    private var total: Int { state.items.count }
    private var headline: AgentWidgetState.Item? { state.items.first }

    /// Prefer WHAT the agent is doing (the hook detail) over where it lives —
    /// "Bash: npm install" beats "work · 2:rednote" at a glance. Falls back to
    /// the location when no hook detail is present.
    private func secondary(_ item: AgentWidgetState.Item) -> String {
        if let d = item.detail, !d.isEmpty { return d }
        return item.location
    }

    private var summaryLine: String {
        var parts: [String] = []
        if state.attentionCount > 0 { parts.append("\(state.attentionCount) needs you") }
        if state.workingCount > 0 { parts.append("\(state.workingCount) working") }
        if isStale { return "paused — open Ringdown" }
        return parts.isEmpty ? "no active agents" : parts.joined(separator: " · ")
    }

    // MARK: families

    private var small: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                Circle().fill(wTeal).frame(width: 7, height: 7)
                Text("Ringdown").font(.system(size: 12, weight: .bold)).foregroundStyle(.white)
                Spacer()
                if state.attentionCount > 0 {
                    Text("\(state.attentionCount)")
                        .font(.system(size: 13, weight: .bold, design: .monospaced))
                        .foregroundStyle(wAmber)
                }
            }
            Spacer(minLength: 0)
            if let headline {
                HStack(spacing: 6) {
                    Circle().fill(dotColor(headline.state)).frame(width: 8, height: 8)
                    Text(headline.command.isEmpty ? "shell" : headline.command)
                        .font(.system(size: 15, weight: .semibold, design: .monospaced))
                        .foregroundStyle(.white).lineLimit(1)
                }
                Text(secondary(headline))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.55)).lineLimit(1)
            }
            Spacer(minLength: 0)
            Text(summaryLine)
                .font(.system(size: 10, weight: .medium, design: .monospaced))
                .foregroundStyle(.white.opacity(0.6)).lineLimit(1)
        }
    }

    private var medium: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 5) {
                Circle().fill(wTeal).frame(width: 7, height: 7)
                Text("Ringdown").font(.system(size: 12, weight: .bold)).foregroundStyle(.white)
                Spacer()
                Text(summaryLine)
                    .font(.system(size: 10, weight: .medium, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.6)).lineLimit(1)
            }
            if state.items.isEmpty {
                Spacer()
                Text("no active agents")
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.4))
                Spacer()
            } else {
                ForEach(state.items.prefix(3)) { item in
                    HStack(spacing: 8) {
                        Circle().fill(dotColor(item.state)).frame(width: 7, height: 7)
                        Text(item.command.isEmpty ? "shell" : item.command)
                            .font(.system(size: 13, weight: .semibold, design: .monospaced))
                            .foregroundStyle(.white).lineLimit(1)
                        Text(secondary(item))
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(item.state == "attention" ? wAmber : .white.opacity(0.5))
                            .lineLimit(1)
                        Spacer()
                    }
                }
            }
            Spacer(minLength: 0)
        }
    }

    private var rectangular: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 5) {
                Circle().fill(dotColor(headline?.state ?? "idle")).frame(width: 7, height: 7)
                Text(headline.map { $0.command.isEmpty ? "shell" : $0.command } ?? "Ringdown")
                    .font(.system(size: 14, weight: .semibold)).lineLimit(1)
            }
            Text(summaryLine).font(.system(size: 12)).lineLimit(1)
        }
    }

    private var circular: some View {
        ZStack {
            AccessoryWidgetBackground()
            VStack(spacing: 0) {
                Text("\(state.attentionCount > 0 ? state.attentionCount : total)")
                    .font(.system(size: 18, weight: .bold, design: .rounded))
                Text(state.attentionCount > 0 ? "wait" : "agt")
                    .font(.system(size: 8, weight: .medium))
            }
        }
    }

    /// Lock-screen inline slot (above the clock). The system renders it monochrome
    /// — an SF Symbol + one line of text, leading with needs-you.
    @ViewBuilder private var inline: some View {
        if state.attentionCount > 0 {
            Label("\(state.attentionCount) needs you", systemImage: "exclamationmark.bubble.fill")
        } else if let headline {
            Label(headline.command.isEmpty ? "shell" : headline.command, systemImage: "circle.fill")
        } else {
            Label("Ringdown · no agents", systemImage: "circle")
        }
    }
}
