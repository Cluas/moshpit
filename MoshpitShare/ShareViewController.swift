import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The share-sheet face of attach-image: pick which agent gets the picture,
/// queue it, done. This process has no SSH and a hard memory ceiling, so it
/// deliberately does only two things — read the shared images, and write
/// them plus a target into the App Group queue that the app drains the next
/// time the matching session is live (immediately, if Moshpit is running).
///
/// Targets come from the same App Group snapshot the lock-screen widget
/// reads: every agent pane the app knew about on its last sync, each with a
/// deep link carrying (connection, pane).
final class ShareViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        let host = UIHostingController(
            rootView: ShareTargetView(
                context: extensionContext,
                onDone: { [weak self] in
                    self?.extensionContext?.completeRequest(returningItems: nil)
                },
                onCancel: { [weak self] in
                    self?.extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
                }))
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
    }
}

private struct ShareTarget: Identifiable {
    let id: String
    let connectionId: UUID
    let paneId: String
    let agentName: String
    let label: String
    let stateLabel: String
}

private struct ShareTargetView: View {
    let context: NSExtensionContext?
    let onDone: () -> Void
    let onCancel: () -> Void

    @State private var images: [Data] = []
    @State private var targets: [ShareTarget] = []
    @State private var snapshotAge: TimeInterval = 0
    @State private var queuedTo: String?
    @State private var failed = false

    var body: some View {
        NavigationStack {
            Group {
                if let queuedTo {
                    queuedConfirmation(queuedTo)
                } else if targets.isEmpty {
                    emptyState
                } else {
                    targetList
                }
            }
            .navigationTitle("Send to agent")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                }
            }
        }
        .task { await load() }
    }

    private var targetList: some View {
        List {
            Section {
                ForEach(targets) { target in
                    Button {
                        queue(to: target)
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "sparkle")
                                .foregroundStyle(.tint)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(target.agentName).font(.body.weight(.semibold))
                                Text(target.label)
                                    .font(.caption.monospaced())
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Text(target.stateLabel)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            } header: {
                Text(images.count == 1
                     ? "1 image — pick the agent that should see it"
                     : "\(images.count) images — pick the agent that should see them")
            } footer: {
                if snapshotAge > 300 {
                    Text("Agent list is from \(Int(snapshotAge / 60)) min ago — targets may have moved on. Queued images are delivered when the session is next live.")
                } else {
                    Text("Delivered as an uploaded path the moment the session is live — instantly, if Moshpit is connected right now.")
                }
            }
        }
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label("No agents to send to", systemImage: "sparkle")
        } description: {
            Text("Open Moshpit and connect to a session with a running agent first — the share sheet lists every agent Moshpit is watching.")
        }
    }

    private func queuedConfirmation(_ name: String) -> some View {
        VStack(spacing: 12) {
            Image(systemName: failed ? "exclamationmark.triangle" : "checkmark.circle.fill")
                .font(.system(size: 40))
                .foregroundStyle(failed ? .orange : .green)
            Text(failed ? "Couldn't queue the images"
                        : "Queued for \(name)")
                .font(.headline)
            if !failed {
                Text("Moshpit uploads and hands over the path the moment that session is live.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task {
            try? await Task.sleep(for: .seconds(1.1))
            onDone()
        }
    }

    private func queue(to target: ShareTarget) {
        do {
            try ShareQueue.append(
                target: (target.connectionId, target.paneId),
                agentName: target.agentName,
                label: target.label,
                images: images)
            queuedTo = target.agentName
        } catch {
            failed = true
            queuedTo = target.agentName
        }
    }

    private func load() async {
        // Targets: the widget snapshot, deep links parsed back into ids.
        let state = AgentWidgetStore.read()
        snapshotAge = Date().timeIntervalSince(state.updatedAt)
        targets = state.items.compactMap { item in
            guard let link = item.deepLink,
                  let url = URL(string: link),
                  let connectionId = UUID(uuidString: url.host() ?? ""),
                  let pane = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                      .queryItems?.first(where: { $0.name == "pane" })?.value
            else { return nil }
            return ShareTarget(id: item.id,
                               connectionId: connectionId,
                               paneId: pane,
                               agentName: item.command,
                               label: item.location,
                               stateLabel: item.state)
        }

        // Images: every attachment that can load as image data.
        var loaded: [Data] = []
        for item in context?.inputItems.compactMap({ $0 as? NSExtensionItem }) ?? [] {
            for provider in item.attachments ?? [] where
                provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                if let data = try? await provider.loadImageData() {
                    loaded.append(data)
                }
            }
        }
        images = loaded
        if images.isEmpty { onCancel() }
    }
}

private extension NSItemProvider {
    /// Image bytes whichever way the sharing app hands them over — a file
    /// URL (Photos, Files) or in-memory data/UIImage (screenshots, web).
    func loadImageData() async throws -> Data? {
        try await withCheckedThrowingContinuation { continuation in
            loadItem(forTypeIdentifier: UTType.image.identifier, options: nil) { item, error in
                if let error { continuation.resume(throwing: error); return }
                switch item {
                case let url as URL:
                    continuation.resume(returning: try? Data(contentsOf: url))
                case let data as Data:
                    continuation.resume(returning: data)
                case let image as UIImage:
                    continuation.resume(returning: image.jpegData(compressionQuality: 0.95))
                default:
                    continuation.resume(returning: nil)
                }
            }
        }
    }
}
