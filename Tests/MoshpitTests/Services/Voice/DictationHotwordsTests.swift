import Foundation
import Testing
@testable import Moshpit

/// The pane's words a recognizer is told to expect (`DictationHotwords`).
@Suite("Dictation hotwords")
struct DictationHotwordsTests {

    /// A Claude Code pane in a tmux window, the way the eval's fake context
    /// is built: a prompt, a diff summary naming files, a branch.
    private let claudePane = DictationPaneFacts(
        agent: "claude",
        command: "claude",
        path: "/Users/dev/code/moshpit",
        sessionName: "work",
        windowName: "voice",
        screen: [
            "~/code/moshpit on  feature/login",
            "$ git status",
            "  modified:   Moshpit/Services/Voice/DictationEngines.swift",
            "  modified:   Moshpit/Models/AppSettings.swift",
            "⏺ Updated AppSettings.swift: max_tokens now reads from the store",
            "  useEffect still runs twice in DictationEngines.swift",
            "> 把 timeout 改成 30 秒",
        ])

    @Test("The agent, the known agents, the project and the tmux names lead")
    func paneFactsLead() {
        let words = DictationHotwords.harvest(claudePane)
        #expect(Array(words.prefix(5)) == ["Claude Code", "Codex", "moshpit", "work", "voice"])
    }

    @Test("Files, branches and identifiers on screen are kept as written")
    func screenWords() {
        let words = DictationHotwords.harvest(claudePane)
        for expected in ["DictationEngines.swift", "AppSettings.swift", "feature/login",
                         "max_tokens", "useEffect", "git"] {
            #expect(words.contains(expected), "missing \(expected) in \(words)")
        }
    }

    @Test("A file seen twice outranks one seen once")
    func frequencyRanks() {
        let words = DictationHotwords.screenWords(claudePane.screen)
        let engines = words.firstIndex(of: "DictationEngines.swift")
        let effect = words.firstIndex(of: "useEffect")
        #expect(engines != nil && effect != nil && engines! < effect!)
    }

    @Test("Plain words, hashes, numbers and shells are not hotwords")
    func noise() {
        let words = DictationHotwords.harvest(DictationPaneFacts(
            command: "zsh",
            sessionName: "0",
            windowName: "bash",
            screen: ["commit 3f9a2c1d7e Merge branch into the main line", "error: 404 not found at https"]))
        #expect(words == DictationHotwords.agentNames)
    }

    @Test("A path names its file, not its directories")
    func pathReducesToFile() {
        #expect(DictationHotwords.reduce("src/net/client_pool.rs") == "client_pool.rs")
        #expect(DictationHotwords.reduce("origin/main") == "origin/main")
        #expect(DictationHotwords.reduce("README.md") == "README.md")
        #expect(DictationHotwords.reduce("deadbeefcafe") == nil)
        #expect(DictationHotwords.reduce("the") == nil)
    }

    @Test("A command typed at a prompt counts, prose in an agent's input box doesn't")
    func promptCommand() {
        #expect(DictationHotwords.candidates(in: "dev@box:~/srv$ kubectl get pods").first == "kubectl")
        #expect(DictationHotwords.candidates(in: "> fix the flaky test").isEmpty)
        #expect(DictationHotwords.candidates(in: "# Install").isEmpty)
    }

    @Test("Duplicates differing only in case keep the first spelling")
    func caseInsensitiveDedupe() {
        let words = DictationHotwords.harvest(DictationPaneFacts(
            agent: "codex", screen: ["CODEX ran", "see AppSettings.swift and appsettings.swift"]))
        #expect(words.filter { $0.lowercased() == "codex" } == ["Codex"])
        #expect(words.filter { $0.lowercased() == "appsettings.swift" }.count == 1)
    }

    @Test("The limit holds")
    func limit() {
        let screen = (0..<300).map { "file_\($0).swift" }
        #expect(DictationHotwords.harvest(DictationPaneFacts(screen: screen)).count == DictationHotwords.appleLimit)
        #expect(DictationHotwords.harvest(DictationPaneFacts(screen: screen), limit: 12).count == 12)
    }

    @Test("The prompt line joins words with commas and never splits one")
    func csv() {
        #expect(DictationHotwords.csv(["Claude Code", "a,b", "main"]) == "Claude Code,a b,main")
    }
}
