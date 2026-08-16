import AppIntents
import Foundation

/// "Send this image to <agent>" as a Shortcuts action — the third door into
/// attach-image (terminal chip, share sheet, and now automations: "when I
/// screenshot, send it to claude on devbox"). Feeds the SAME App Group queue
/// the share extension writes; delivery is the drainer's job, so the intent
/// works whether or not the session is live right now.
struct AttachImageIntent: AppIntent {
    static let title: LocalizedStringResource = "Send Image to Agent"
    static let description = IntentDescription(
        "Uploads images to the agent's server and hands it the file paths.",
        categoryName: "Agents")
    // Runs in-process without foregrounding: queueing is filesystem work,
    // and the drain happens on the app's own schedule.
    static let openAppWhenRun = false

    @Parameter(title: "Images", supportedContentTypes: [.image])
    var images: [IntentFile]

    @Parameter(title: "Agent")
    var target: AgentTargetEntity

    static var parameterSummary: some ParameterSummary {
        Summary("Send \(\.$images) to \(\.$target)")
    }

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        // File-backed when Shortcuts hands over a file, inline otherwise.
        let payloads: [Data] = images.map { file in
            if let url = file.fileURL, let onDisk = try? Data(contentsOf: url) {
                return onDisk
            }
            return file.data
        }.filter { !$0.isEmpty }
        guard !payloads.isEmpty else {
            return .result(dialog: "No readable images were provided.")
        }
        try ShareQueue.append(
            target: (target.connectionId, target.paneId),
            agentName: target.agentName,
            label: target.label,
            images: payloads)
        // Deliver immediately when the session is live in this process —
        // the bridge closure is set at app launch, same as the notification
        // and Live Activity hooks.
        await AgentControlBridge.shared.drainShareQueue?()
        let pending = ShareQueue.load().contains { $0.paneId == target.paneId }
        return .result(dialog: pending
            ? IntentDialog("Queued for \(target.agentName) — delivers when the session is next live.")
            : IntentDialog("Sent to \(target.agentName)."))
    }
}

/// A queued-image destination: one agent pane Moshpit is watching, as the
/// widget snapshot last saw it.
struct AgentTargetEntity: AppEntity, Identifiable {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Agent"
    static let defaultQuery = AgentTargetQuery()

    var id: String
    var connectionId: UUID
    var paneId: String
    var agentName: String
    var label: String

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(agentName)", subtitle: "\(label)")
    }

    static func fromSnapshot() -> [AgentTargetEntity] {
        AgentWidgetStore.read().items.compactMap { item in
            guard let link = item.deepLink,
                  let url = URL(string: link),
                  let connectionId = UUID(uuidString: url.host() ?? ""),
                  let pane = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                      .queryItems?.first(where: { $0.name == "pane" })?.value
            else { return nil }
            return AgentTargetEntity(id: item.id, connectionId: connectionId,
                                     paneId: pane, agentName: item.command,
                                     label: item.location)
        }
    }
}

struct AgentTargetQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [AgentTargetEntity] {
        AgentTargetEntity.fromSnapshot().filter { identifiers.contains($0.id) }
    }

    func suggestedEntities() async throws -> [AgentTargetEntity] {
        AgentTargetEntity.fromSnapshot()
    }
}
