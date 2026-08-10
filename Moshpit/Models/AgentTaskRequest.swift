import Foundation

/// What the New agent task form collects: a repo, a branch to isolate the work
/// on, an agent to run, and an optional opening prompt.
///
/// Validation lives here rather than in the view so the rules are testable and
/// so a bad branch name is caught before a round trip — typing on a phone,
/// finding out from a remote error what `git` would have told you instantly is
/// the difference between one taps and three.
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

    /// Human-readable reason this can't be submitted, or nil when it can.
    var validationError: String? {
        if repoPath.trimmingCharacters(in: .whitespaces).isEmpty {
            return String(localized: "Pick a repository")
        }
        if agent.trimmingCharacters(in: .whitespaces).isEmpty {
            return String(localized: "Pick an agent")
        }
        return Self.branchError(branch)
    }

    var isValid: Bool { validationError == nil }

    /// The subset of `git check-ref-format` worth enforcing on a phone
    /// keyboard. Not a reimplementation of git — just the mistakes that are
    /// easy to make and annoying to discover remotely.
    static func branchError(_ raw: String) -> String? {
        let branch = raw.trimmingCharacters(in: .whitespaces)
        if branch.isEmpty { return String(localized: "Name the branch") }
        if branch.contains(" ") { return String(localized: "No spaces in a branch name") }
        if branch.hasPrefix("-") || branch.hasPrefix("/") || branch.hasSuffix("/") {
            return String(localized: "Can't start with “-” or “/”, or end with “/”")
        }
        if branch.contains("..") { return String(localized: "No “..” in a branch name") }
        if branch.hasSuffix(".lock") { return String(localized: "Can't end with “.lock”") }
        let forbidden = CharacterSet(charactersIn: "~^:?*[\\\u{7f}")
        if branch.rangeOfCharacter(from: forbidden) != nil {
            return String(localized: "No ~ ^ : ? * [ \\ in a branch name")
        }
        if branch.unicodeScalars.contains(where: { $0.value < 0x20 }) {
            return String(localized: "No control characters in a branch name")
        }
        return nil
    }

    /// Last path component of the repo, for display. `~/code/moshi` → `moshi`.
    var repoName: String {
        (repoPath as NSString).lastPathComponent
    }

    /// `/Users/cluas/code/moshi` → `~/code/moshi`, for menu subtitles — a
    /// list of nine repos otherwise spells the account prefix nine times and
    /// wraps the part that differs. Only the home patterns git hosts actually
    /// use; anything else passes through untouched.
    static func abbreviatePath(_ path: String) -> String {
        for prefix in ["/Users/", "/home/"] where path.hasPrefix(prefix) {
            let rest = path.dropFirst(prefix.count)
            guard let slash = rest.firstIndex(of: "/") else { return "~" }
            return "~" + rest[slash...]
        }
        if path == "/root" { return "~" }
        if path.hasPrefix("/root/") { return "~" + path.dropFirst("/root".count) }
        return path
    }
}
