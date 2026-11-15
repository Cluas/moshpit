import Foundation
import Testing
@testable import Moshpit

/// E6c: New Agent Task and Import Theme, off SwiftUI: what their hosts post
/// to the NSH pages (`ui/tests/agent_task.rs`, `ui/tests/theme_import.rs`
/// hold what the pages do with it) and read back.
@Suite("NSH sheet hosts")
struct SheetHostsNSHTests {
    private func fields(_ json: String) throws -> [String: Any] {
        try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    @Test("New Agent Task is told the host's repositories, agents and how the start went")
    func agentTaskMessage() throws {
        let message = try fields(AgentTaskNSHPage.message(
            repos: ["/Users/dev/code/moshpit"], agents: ["claude", "codex"], scanning: true,
            starting: false, failure: nil, taskExists: false))
        #expect(message["op"] as? String == "agentTask")
        #expect(message["repos"] as? [String] == ["/Users/dev/code/moshpit"])
        #expect(message["agents"] as? [String] == ["claude", "codex"])
        #expect(message["scanning"] as? Bool == true)
        #expect(message["failure"] == nil)
        let failed = try fields(AgentTaskNSHPage.message(
            repos: [], agents: [], scanning: false, starting: false,
            failure: "Creating the worktree failed", taskExists: true))
        #expect(failed["failure"] as? String == "Creating the worktree failed")
        #expect(failed["taskExists"] as? Bool == true)
    }

    @Test("Start's request is the page's, word for word")
    func agentTaskRequest() {
        let request = AgentTaskNSHPage.request([
            "ev": "start", "repo": "/srv/web", "branch": "fix-scroll", "agent": "codex", "prompt": "Go",
        ])
        #expect(request == AgentTaskRequest(repoPath: "/srv/web", branch: "fix-scroll", agent: "codex",
                                            prompt: "Go"))
        #expect(AgentTaskNSHPage.request(["ev": "start", "repo": "/srv/web"]) == nil)
        #expect(AgentTaskNSHPage.request(["ev": "cancel"]) == nil)
    }

    @Test("Import Theme is told why an import didn't take, and only then")
    func themeImportMessage() throws {
        let fine = try fields(ThemeImportNSHPage.message(error: nil))
        #expect(fine["op"] as? String == "themeImport")
        #expect(fine["error"] == nil)
        let refused = try fields(ThemeImportNSHPage.message(error: "No theme in it."))
        #expect(refused["error"] as? String == "No theme in it.")
    }

    @Test("the two pages are NSH's")
    func screens() {
        #expect(NSHScreen.agentTask.name == "agent-task")
        #expect(NSHScreen.themeImport.name == "theme-import")
    }
}
