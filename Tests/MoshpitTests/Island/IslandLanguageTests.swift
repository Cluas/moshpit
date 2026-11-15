import Foundation
import Testing
@testable import Moshpit
import MoshpitKit

/// The Live Activity and the widget are drawn by the MoshpitIsland
/// extension, whose own languages are the system's. They used to be what it
/// wrote in: Moshpit set to English on a phone in Chinese had an English app
/// and a Chinese island. The app records the language it shows in the App
/// Group (AppLanguage), and the island writes in it. Run on a device in
/// English: Moshpit set to Chinese must give a Chinese island.
@Suite("Island language", .serialized, .enabled(if: AppGroupContainer.isAvailable, AppGroupContainer.skipReason))
@MainActor
struct IslandLanguageTests {

    private func withAppLanguage(_ language: String, _ body: () throws -> Void) rethrows {
        let saved = AppLanguage.stored()
        defer {
            if let saved { AppLanguage.record(saved) }
            else { UserDefaults(suiteName: PushPrefs.suite)?.removeObject(forKey: AppLanguage.key) }
        }
        AppLanguage.record(language)
        try body()
    }

    /// The island's own catalog, as it ships inside the app.
    private func islandBundle() throws -> Bundle {
        let url = try #require(Bundle.main.builtInPlugInsURL?.appendingPathComponent("MoshpitIsland.appex"))
        return try #require(Bundle(url: url), "no MoshpitIsland.appex in the test host")
    }

    @Test("a row's state on the island reads in the language Moshpit shows")
    func stateFollowsTheAppsLanguage() throws {
        try withAppLanguage("zh-Hans") {
            #expect(AgentActivityAttributes.AgentState.done.label == "已完成",
                    "the system's language, not Moshpit's: \(AgentActivityAttributes.AgentState.done.label)")
            #expect(AgentActivityAttributes.AgentState.attention.label == "在等你")
            #expect(AgentActivityAttributes.AgentState.working.label == "工作中")
        }
    }

    @Test("the lock screen's header reads in the language Moshpit shows, from the island's catalog")
    func headerFollowsTheAppsLanguage() throws {
        let bundle = try islandBundle()
        let agents: [AgentActivityAttributes.Agent] = [
            .init(id: "c:%1", connectionId: "c", paneId: "%1", command: "claude", location: "devbox",
                  state: .attention, startedAt: Date()),
            .init(id: "c:%2", connectionId: "c", paneId: "%2", command: "codex", location: "devbox",
                  state: .done, startedAt: Date()),
        ]
        let state = AgentActivityAttributes.ContentState(agents: agents, workingCount: 0, attentionCount: 1)
        try withAppLanguage("zh-Hans") {
            #expect(LockScreenView.summary(for: state, in: bundle) == "1 个在等你 · 1 个已完成")
        }
        try withAppLanguage("en") {
            #expect(LockScreenView.summary(for: state, in: bundle) == "1 needs you · 1 done")
        }
    }

    /// B662: a dropped connection's agents are counted apart in the
    /// header, not as working or done.
    @Test("the lock screen's header counts a dropped connection's agents as offline")
    func headerCountsOffline() throws {
        let bundle = try islandBundle()
        let dropped = Date()
        let agents: [AgentActivityAttributes.Agent] = [
            .init(id: "c:%1", connectionId: "c", paneId: "%1", command: "claude", location: "devbox",
                  state: .working, startedAt: Date()),
            .init(id: "d:%1", connectionId: "d", paneId: "%1", command: "codex", location: "mac",
                  state: .working, startedAt: Date(), pausedAt: dropped),
            .init(id: "d:%2", connectionId: "d", paneId: "%2", command: "claude", location: "mac",
                  state: .done, startedAt: Date(), pausedAt: dropped),
        ]
        let state = AgentActivityAttributes.ContentState(agents: agents, workingCount: 1, attentionCount: 0)
        try withAppLanguage("zh-Hans") {
            #expect(LockScreenView.summary(for: state, in: bundle) == "1 个工作中 · 2 个离线")
        }
        try withAppLanguage("en") {
            #expect(LockScreenView.summary(for: state, in: bundle) == "1 working · 2 offline")
        }
    }
}
