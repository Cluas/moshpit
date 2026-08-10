import SwiftUI

/// Start an agent on its own git worktree, from the phone.
///
/// Deliberately built from the same pieces as Add Connection — `FormGroup`,
/// `FieldRow`, a `Menu` row with `MiniChevron` — so it reads as part of the
/// app rather than a herdr appendage. The only new idea on screen is the work
/// it does, not how it looks.
struct NewAgentTaskSheet: View {
    /// What we can offer immediately — the directories the panes are already
    /// sitting in. Shown while the fuller scan runs so the form is usable the
    /// instant it opens.
    let initialRepos: [String]
    /// Looks for repositories on the host (~1s) and asks herdr which agents it
    /// knows. Runs when the sheet appears, not before it.
    let load: () async -> (repos: [String], agents: [String])
    /// Runs the task; returns nil on success or a message to show.
    let start: (AgentTaskRequest) async -> String?

    @Environment(\.dismiss) private var dismiss
    @State private var request = AgentTaskRequest()
    @State private var repos: [String] = []
    @State private var agents: [String] = []
    @State private var scanning = false
    @State private var customRepo = ""
    @State private var showCustomRepo = false
    /// Big repos take a while to check out. Without this the Start button
    /// looks broken and gets tapped again — the same trap the Create Session
    /// button fell into.
    @State private var starting = false
    @State private var failure: String?

    private var repoValue: String {
        request.repoPath.isEmpty ? String(localized: "Choose") : request.repoName
    }

    var body: some View {
        NavigationStack {
            ZStack {
                MoshpitBackground()
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        FormGroup(
                            title: "TASK",
                            footer: "Creates a git worktree on the host, then starts the agent inside it. Your working tree is untouched."
                        ) {
                            repoRow
                            if showCustomRepo {
                                FieldRow(placeholder: "Repository path", text: $customRepo, mono: true)
                            }
                            // Field and its complaint share one row so the
                            // group's divider logic doesn't treat the hint as
                            // a row of its own.
                            VStack(alignment: .leading, spacing: 0) {
                                FieldRow(placeholder: "Branch", text: $request.branch, mono: true)
                                if let hint = branchHint {
                                    Text(hint)
                                        .font(Face.mono(11))
                                        .foregroundStyle(Ink.warn)
                                        .padding(.bottom, 8)
                                }
                            }
                            agentRow
                        }

                        FormGroup(
                            title: "FIRST MESSAGE",
                            footer: "Optional. Sent to the agent once it's running — leave blank to type it yourself."
                        ) {
                            FieldRow(placeholder: "Prompt", text: $request.prompt, multiline: true)
                        }

                        if let failure {
                            Label {
                                Text(failure)
                                    .font(Face.text(12))
                                    .fixedSize(horizontal: false, vertical: true)
                            } icon: {
                                Image(systemName: "exclamationmark.circle.fill")
                                    .foregroundStyle(Ink.warn)
                            }
                            .foregroundStyle(Ink.secondary)
                            .padding(.top, 14)
                        }
                    }
                    .padding(.horizontal, Metrics.pageHPad)
                    .padding(.bottom, 32)
                }
            }
            .navigationTitle("New Agent Task")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .font(Face.text(15))
                        .foregroundStyle(Ink.accent)
                        .disabled(starting)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(starting ? "Starting…" : "Start") { run() }
                        .font(Face.text(15, .semibold))
                        .foregroundStyle(canStart ? Ink.accent : Ink.disabledNav)
                        .disabled(!canStart)
                        .accessibilityIdentifier("start-agent-task")
                }
            }
        }
        .preferredColorScheme(.dark)
        .onAppear {
            if repos.isEmpty { repos = initialRepos }
            if request.repoPath.isEmpty, let first = repos.first { request.repoPath = first }
        }
        .task {
            scanning = true
            let found = await load()
            scanning = false
            if !found.repos.isEmpty { repos = found.repos }
            agents = found.agents
            // Only move the selection if the user hasn't picked yet, or if
            // what they're looking at was just a placeholder from the panes.
            if request.repoPath.isEmpty || !repos.contains(request.repoPath) {
                request.repoPath = repos.first ?? request.repoPath
            }
            if request.agent.isEmpty { request.agent = Self.defaultAgent(from: agents) }
        }
    }

    /// herdr lists every agent it can detect, alphabetically — which made the
    /// default whatever sorts first, not what anyone is likely to want. Prefer
    /// the ones this app is actually built around, then fall back.
    static func defaultAgent(from agents: [String]) -> String {
        for preferred in ["claude", "codex"] where agents.contains(preferred) {
            return preferred
        }
        return agents.first ?? "claude"
    }

    private var canStart: Bool {
        !starting && resolved().isValid
    }

    /// Why Start is greyed out, said next to the field that caused it — but
    /// only once the user has actually typed a branch. A disabled button with
    /// no stated reason just reads as broken.
    private var branchHint: String? {
        guard !request.branch.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return AgentTaskRequest.branchError(request.branch)
    }

    /// The custom path wins when the user opened that row — otherwise a stale
    /// pick from the menu would silently override what they typed.
    private func resolved() -> AgentTaskRequest {
        var value = request
        if showCustomRepo {
            value.repoPath = customRepo.trimmingCharacters(in: .whitespaces)
        }
        return value
    }

    private var repoRow: some View {
        Menu {
            ForEach(repos, id: \.self) { repo in
                Button {
                    showCustomRepo = false
                    request.repoPath = repo
                } label: {
                    Text(verbatim: (repo as NSString).lastPathComponent)
                    // "~/code/moshi", not the full home prefix nine times over.
                    Text(verbatim: AgentTaskRequest.abbreviatePath(repo))
                }
            }
            if scanning {
                Text("Looking for repositories…")
            } else if repos.isEmpty {
                // Say WHY the list is bare, or an empty menu reads as broken.
                // (Seen on a slow host: the scan missed its budget, and the
                // menu was just "Other…" with no explanation.)
                Text("None found — no panes in repos, and nothing under ~")
            }
            Button("Other…") { showCustomRepo = true }
        } label: {
            HStack(spacing: 10) {
                Text("Repo").font(Face.text(14)).foregroundStyle(Ink.primary)
                Spacer()
                if scanning && repos.isEmpty {
                    ProgressView().controlSize(.mini).tint(Ink.meta)
                }
                Text(showCustomRepo ? String(localized: "Custom") : repoValue)
                    .font(Face.text(14)).foregroundStyle(Ink.meta).lineLimit(1)
                MiniChevron()
            }
            .frame(minHeight: Metrics.cellMinHeight)
            .contentShape(Rectangle())
        }
        .accessibilityIdentifier("agent-task-repo")
    }

    private var agentRow: some View {
        Menu {
            ForEach(agents, id: \.self) { agent in
                Button(agent) { request.agent = agent }
            }
        } label: {
            HStack(spacing: 10) {
                Text("Agent").font(Face.text(14)).foregroundStyle(Ink.primary)
                Spacer()
                Text(request.agent.isEmpty ? String(localized: "Choose") : request.agent)
                    .font(Face.text(14)).foregroundStyle(Ink.meta)
                MiniChevron()
            }
            .frame(minHeight: Metrics.cellMinHeight)
            .contentShape(Rectangle())
        }
        .accessibilityIdentifier("agent-task-agent")
    }

    private func run() {
        let value = resolved()
        failure = nil
        starting = true
        Task {
            let error = await start(value)
            starting = false
            if let error {
                failure = error
            } else {
                // The host focuses the new workspace, so the terminal is
                // already showing it by the time this closes.
                dismiss()
            }
        }
    }
}
