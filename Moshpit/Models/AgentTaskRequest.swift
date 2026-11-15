import Foundation

/// What New Agent Task asks for (`ui/src/agent_task.rs`): a repo, a branch
/// to isolate the work on, an agent to run, and an optional opening
/// prompt. The core checks it before anything runs
/// (`moshpit_herdr::control::AgentTaskRequest::validation_error`), the
/// page before Start can be pressed.
struct AgentTaskRequest: Equatable {
    /// Absolute path to the repository root on the host.
    var repoPath: String = ""
    /// Branch the worktree gets. Also used as the workspace label, so the
    /// Agents section and the tree read the same word the user typed.
    var branch: String = ""
    /// Agent command to run in the new pane, e.g. `claude`.
    var agent: String = ""
    /// Sent to the agent once it's up. Empty means don't send anything.
    var prompt: String = ""
}

/// How a New agent task went — three ways, because the form has to act on
/// the difference.
enum AgentTaskOutcome: Equatable {
    /// The worktree, its agent, and the first message if there was one.
    case started
    /// Nothing was made. The form shows why and stays open for another try.
    case failed(String)
    /// The worktree exists, but what came after it didn't land: the agent
    /// didn't start, or the first message never reached it. Start again
    /// would only collide with the branch the worktree now has, so the form
    /// says what happened and offers Done — with the message still there
    /// to copy into the pane.
    case incomplete(String)
}
