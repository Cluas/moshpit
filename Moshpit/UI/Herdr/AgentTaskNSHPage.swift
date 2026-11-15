import SwiftUI

/// New Agent Task drawn by NSH (`ui/src/agent_task.rs`) in its sheet: the
/// form is the page's; the host has the host — the repositories (the
/// panes' at once, then the scan's), the agents herdr knows, the start and
/// how it went — and posts them as they change (`{"op": "agentTask", …}`).
/// If its screen can't run, the sheet says so.
struct AgentTaskNSHPage: View {
    /// The directories the panes already sit in: offered at once, while
    /// the fuller scan runs.
    let initialRepos: [String]
    /// Looks for repositories on the host (~1 s) and asks herdr which
    /// agents it knows. Runs when the sheet appears, not before.
    let load: () async -> (repos: [String], agents: [String])
    /// Runs the task and says how it went.
    let start: (AgentTaskRequest) async -> AgentTaskOutcome

    @Environment(\.dismiss) private var dismiss
    @State private var model = AgentTaskNSHModel()
    @State private var failed = false
    @State private var repos: [String]?
    @State private var agents: [String] = []
    /// Until the scan is back: the page picks the default agent after.
    @State private var scanning = true
    /// Big repositories take a while to check out: Start waits, or it gets
    /// pressed again.
    @State private var starting = false
    /// The start in flight, so Cancel can call it off: the create already
    /// sent can't be taken back, but no agent is typed into its pane after.
    @State private var startTask: Task<Void, Never>?
    @State private var failure: String?
    /// The worktree exists but the run came back incomplete: Done takes
    /// Start's place.
    @State private var taskExists = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            let state = message
            NSHPage(
                screen: .agentTask,
                attach: { model.attach($0, current: state) },
                onMessage: handle,
                onFailure: { failed = true })
                .ignoresSafeArea(.all)
                .onAppear { model.update(state) }
                .onChange(of: state) { _, new in model.update(new) }
                .background(Ink.pageBG)
                .task {
                    let found = await load()
                    scanning = false
                    if !found.repos.isEmpty { repos = found.repos }
                    agents = found.agents
                }
        }
    }

    /// The page's `{"op": "agentTask", …}` now.
    private var message: String {
        Self.message(repos: repos ?? initialRepos, agents: agents, scanning: scanning,
                     starting: starting, failure: failure, taskExists: taskExists)
    }

    nonisolated static func message(repos: [String], agents: [String], scanning: Bool, starting: Bool,
                                    failure: String?, taskExists: Bool) -> String {
        var fields: [String: Any] = [
            "op": "agentTask", "repos": repos, "agents": agents, "scanning": scanning,
            "starting": starting, "taskExists": taskExists,
        ]
        if let failure { fields["failure"] = failure }
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    /// The request in the page's `start`.
    nonisolated static func request(_ message: [String: Any]) -> AgentTaskRequest? {
        guard message["ev"] as? String == "start",
              let repo = message["repo"] as? String,
              let branch = message["branch"] as? String,
              let agent = message["agent"] as? String else { return nil }
        return AgentTaskRequest(repoPath: repo, branch: branch, agent: agent,
                                prompt: message["prompt"] as? String ?? "")
    }

    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "start":
            guard !starting, let request = Self.request(message) else { return }
            run(request)
        case "cancel":
            startTask?.cancel()
            dismiss()
        case "done":
            dismiss()
        default:
            break
        }
    }

    private func run(_ request: AgentTaskRequest) {
        failure = nil
        starting = true
        startTask = Task {
            let outcome = await start(request)
            starting = false
            // Cancelled: the form is gone, and there is no one to tell.
            guard !Task.isCancelled else { return }
            switch outcome {
            case .started:
                // The host focuses the new workspace, so the terminal is
                // already showing it by the time this closes.
                dismiss()
            case .failed(let message):
                failure = message
            case .incomplete(let message):
                // Stay open: what didn't land has to be said, and a first
                // message that never arrived is still in its field to copy.
                failure = message
                taskExists = true
            }
        }
    }
}

/// What the NSH New Agent Task was last told, so a surface made again
/// starts from it.
@MainActor
final class AgentTaskNSHModel {
    private weak var surface: NSHSurface?
    private var message: String?

    /// The surface, told `current` (the page's state as its body has it
    /// now) unless the model already has a newer one.
    func attach(_ surface: NSHSurface, current: String) {
        if message == nil { message = current }
        self.surface = surface
        if let message { surface.post(json: message) }
    }

    func update(_ message: String) {
        guard message != self.message else { return }
        self.message = message
        surface?.post(json: message)
    }
}
