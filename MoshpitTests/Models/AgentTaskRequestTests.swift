import Foundation
import Testing
@testable import Moshpit

/// Branch-name rules are enforced locally so a phone-keyboard typo is caught
/// before a round trip, instead of coming back as a remote git error.
@Suite("Agent task request")
struct AgentTaskRequestTests {

    private func valid() -> AgentTaskRequest {
        AgentTaskRequest(repoPath: "/Users/cluas/code/moshi",
                         branch: "fix-scroll-jump",
                         agent: "claude")
    }

    @Test("A complete request passes")
    func complete() {
        #expect(valid().isValid)
        #expect(valid().validationError == nil)
    }

    @Test("Repo and agent are both required")
    func requiredFields() {
        var noRepo = valid(); noRepo.repoPath = "  "
        #expect(noRepo.validationError == "Pick a repository")

        var noAgent = valid(); noAgent.agent = ""
        #expect(noAgent.validationError == "Pick an agent")
    }

    @Test("Branch names git would reject are caught here",
          arguments: [
            "", "  ",
            "two words",
            "-leading-dash",
            "/leading-slash",
            "trailing-slash/",
            "double..dot",
            "ends.lock",
            "star*", "question?", "colon:", "tilde~", "caret^", "bracket[", "back\\slash",
          ])
    func rejectedBranches(_ branch: String) {
        var request = valid(); request.branch = branch
        #expect(request.validationError != nil, "\(branch) should be rejected")
    }

    @Test("Branch names git accepts pass through",
          arguments: ["fix-scroll-jump", "feature/thing", "v2", "a_b.c", "用户-分支"])
    func acceptedBranches(_ branch: String) {
        var request = valid(); request.branch = branch
        #expect(request.validationError == nil, "\(branch) should be accepted")
    }

    @Test("Surrounding whitespace doesn't make a name invalid")
    func trimsWhitespace() {
        var request = valid(); request.branch = "  fix-scroll  "
        #expect(request.isValid)
    }

    @Test("Repo name is the last path component, for display")
    func repoName() {
        #expect(valid().repoName == "moshi")
    }

    @Test("Menu subtitles collapse the home prefix to ~")
    func pathAbbreviation() {
        #expect(AgentTaskRequest.abbreviatePath("/Users/cluas/code/moshi") == "~/code/moshi")
        #expect(AgentTaskRequest.abbreviatePath("/home/deploy/rugisland") == "~/rugisland")
        #expect(AgentTaskRequest.abbreviatePath("/root/gotools") == "~/gotools")
        #expect(AgentTaskRequest.abbreviatePath("/Users/cluas") == "~")
        // Not a home path — leave it alone rather than guess.
        #expect(AgentTaskRequest.abbreviatePath("/srv/repos/api") == "/srv/repos/api")
    }
}
