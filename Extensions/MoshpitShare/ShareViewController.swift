import SwiftUI
import UIKit
import UniformTypeIdentifiers
import MoshpitKit

/// The share-sheet face of attach-image: pick which agent gets the picture,
/// queue it, done. This process has no SSH and a hard memory ceiling, so it
/// deliberately does only two things — read the shared images, and write
/// them plus a target into the App Group queue that the app drains the next
/// time the matching session is live (immediately, if Moshpit is running).
///
/// Targets come from the same App Group snapshot the lock-screen widget
/// reads: every agent pane the app knew about on its last sync, each with a
/// deep link carrying (connection, pane).
final class ShareViewController: UIViewController, UIAdaptivePresentationControllerDelegate {
    /// This sheet's staged images, discarded however the sheet goes (B344).
    private let staging = ShareStaging()

    override func viewDidLoad() {
        super.viewDidLoad()
        let host = UIHostingController(
            rootView: ShareTargetView(
                context: extensionContext,
                staging: staging,
                onDone: { [weak self] in
                    self?.staging.end()
                    self?.extensionContext?.completeRequest(returningItems: nil)
                },
                onCancel: { [weak self] in
                    self?.staging.end()
                    self?.extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
                }))
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // A swipe down takes the sheet away without Cancel (B344).
        presentationController?.delegate = self
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        staging.end()
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        // Whatever took the sheet away. Harmless after a queued share: its
        // images are named by its entry and stay.
        staging.end()
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
    let staging: ShareStaging
    let onDone: () -> Void
    let onCancel: () -> Void

    /// The shared images, already written into the queue's directory one
    /// at a time (B315: never all of them in memory at once); a target's
    /// tap gives them an entry, Cancel drops them.
    @State private var staged: [String] = []
    /// Until the shared images are written, no target is offered: a big
    /// photo (or one iCloud has to fetch) takes seconds, and a tap meanwhile
    /// queued nothing and said it had.
    @State private var loading = true
    @State private var targets: [ShareTarget] = []
    @State private var snapshotAge: TimeInterval = 0
    @State private var queuedTo: String?
    @State private var failed = false

    var body: some View {
        NavigationStack {
            Group {
                if let queuedTo {
                    queuedConfirmation(queuedTo)
                } else if loading {
                    ProgressView()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
                    Button("Cancel") {
                        onCancel()
                    }
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
                if staged.count == 1 {
                    Text("1 image — pick the agent that should see it")
                } else {
                    Text("\(staged.count) images — pick the agent that should see them")
                }
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
            Group {
                if failed {
                    Text("Couldn't queue the images")
                } else {
                    Text("Queued for \(name)")
                }
            }
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
        // Once: a second tap before the sheet changed queued the same images
        // again (B341).
        guard !loading, !staged.isEmpty, queuedTo == nil, !staging.ended else { return }
        do {
            try ShareQueue.append(
                target: (target.connectionId, target.paneId),
                agentName: target.agentName,
                label: target.label,
                files: staged)
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
            guard let target = item.target else { return nil }
            return ShareTarget(id: item.id,
                               connectionId: target.connectionId,
                               paneId: target.paneId,
                               agentName: item.command,
                               label: item.location,
                               stateLabel: AgentActivityAttributes.AgentState(rawValue: item.state)?.label
                                   ?? item.state)
        }

        // Images: every attachment that is one, written into the queue one
        // at a time as it is shared (the app scales and strips on drain).
        let providers = (context?.inputItems.compactMap { $0 as? NSExtensionItem } ?? [])
            .flatMap { $0.attachments ?? [] }
            .filter { $0.hasItemConformingToTypeIdentifier(UTType.image.identifier) }
        staged = await ShareQueue.stage(count: providers.count) { try await providers[$0].sharedImage() }
        staging.didStage(staged)
        loading = false
        if staged.isEmpty { onCancel() }
    }
}

private extension NSItemProvider {
    /// The image whichever way the sharing app hands it over: a file URL
    /// (Photos, Files), copied while the URL is good and never read into
    /// memory, or in-memory data or a UIImage (screenshots, web).
    func sharedImage() async throws -> SharedImage? {
        try await withCheckedThrowingContinuation { continuation in
            loadItem(forTypeIdentifier: UTType.image.identifier, options: nil) { item, error in
                if let error { continuation.resume(throwing: error); return }
                switch item {
                case let url as URL:
                    let copy = FileManager.default.temporaryDirectory
                        .appendingPathComponent(UUID().uuidString)
                    let copied = (try? FileManager.default.copyItem(at: url, to: copy)) != nil
                    continuation.resume(returning: copied ? .temporaryFile(copy) : nil)
                case let data as Data:
                    continuation.resume(returning: .data(data))
                case let image as UIImage:
                    continuation.resume(returning: image.jpegData(compressionQuality: 0.95).map(SharedImage.data))
                default:
                    continuation.resume(returning: nil)
                }
            }
        }
    }
}
