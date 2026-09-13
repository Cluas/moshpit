import Foundation
import Testing
import UserNotifications
@testable import Moshpit
import MoshpitKit

/// The words on an agent card, pinned once for both copies of it: the local
/// notification the app posts and the pushed one the extension rewrites both
/// come from this renderer, so a pushed finish and a local one read alike.
@Suite("Agent notification copy")
struct AgentNotificationCopyTests {

    static let en = Locale(identifier: "en_US")

    // MARK: - Detail

    @Test("a long prompt is cut by character, with an ellipsis")
    func longPromptIsClamped() {
        let prompt = String(repeating: "x", count: 200)
        let shown = AgentNotificationCopy.detail(prompt)
        #expect(shown?.count == AgentNotificationCopy.detailLimit)
        #expect(shown?.hasSuffix("…") == true)
        // Characters, not bytes: a CJK prompt is cut at the same count.
        let cjk = String(repeating: "长", count: 200)
        #expect(AgentNotificationCopy.detail(cjk)?.count == AgentNotificationCopy.detailLimit)
    }

    @Test("a prompt that fits is shown whole")
    func shortPromptIsWhole() {
        #expect(AgentNotificationCopy.detail("Upload the build with the new Xcode")
                == "Upload the build with the new Xcode")
        let exact = String(repeating: "y", count: AgentNotificationCopy.detailLimit)
        #expect(AgentNotificationCopy.detail(exact) == exact)
    }

    @Test("newlines and whitespace runs collapse — a multi-line prompt is one line on a card")
    func whitespaceCollapses() {
        #expect(AgentNotificationCopy.detail("  fix the\n\n  scroll  jitter\t please \n")
                == "fix the scroll jitter please")
    }

    @Test("nothing left is nil, not an empty line")
    func emptyDetailIsNil() {
        #expect(AgentNotificationCopy.detail(nil) == nil)
        #expect(AgentNotificationCopy.detail("") == nil)
        #expect(AgentNotificationCopy.detail(" \n\t ") == nil)
    }

    // MARK: - Place

    @Test("the place is the connection's name and the session, nothing more")
    func placeIsLabelAndSession() {
        #expect(AgentNotificationCopy.place(label: "mac-mini", session: "work") == "mac-mini · work")
        #expect(AgentNotificationCopy.place(label: "mac-mini", session: nil) == "mac-mini")
        #expect(AgentNotificationCopy.place(label: "mac-mini", session: "  ") == "mac-mini")
    }

    @Test("tmux's numeric default session name is not a place")
    func numericSessionDropped() {
        // "mac-mini.lan · 0" on a real lock screen read as a broken counter.
        #expect(AgentNotificationCopy.place(label: "mac-mini", session: "0") == "mac-mini")
        #expect(AgentNotificationCopy.place(label: "mac-mini", session: "12") == "mac-mini")
        #expect(AgentNotificationCopy.place(label: "mac-mini", session: "api-2") == "mac-mini · api-2")
    }

    // MARK: - Duration

    @Test("a turn under a minute shows no duration")
    func shortTurnHasNoDuration() {
        #expect(AgentNotificationCopy.duration(0, locale: Self.en) == nil)
        #expect(AgentNotificationCopy.duration(59, locale: Self.en) == nil)
    }

    @Test("minutes and hours read as Foundation spells them, whole minutes only")
    func durationSpelling() {
        #expect(AgentNotificationCopy.duration(60, locale: Self.en) == "1 min")
        #expect(AgentNotificationCopy.duration(12 * 60 + 40, locale: Self.en) == "12 min")
        let long = AgentNotificationCopy.duration(65 * 60, locale: Self.en) ?? ""
        #expect(long.contains("1 hr") && long.contains("5 min"), "got \(long)")
    }

    // MARK: - Cards

    @Test("a finished card: what finished and how long it ran, over where")
    func doneCard() {
        let content = UNMutableNotificationContent()
        AgentNotificationCopy.done(agent: "claude", detail: "Upload the build with the new Xcode",
                                   place: "mac-mini · work", duration: 12 * 60,
                                   sound: true, threshold: 180, into: content)
        #expect(content.title.hasPrefix("✓ claude finished · "))
        #expect(content.body == "Upload the build with the new Xcode — mac-mini · work")
        // Twelve minutes is a walked-away build: worth the chime.
        #expect(content.sound != nil)
        #expect(content.interruptionLevel == .active)
    }

    @Test("a short finish is information, not an interruption")
    func shortDoneIsPassive() {
        let content = UNMutableNotificationContent()
        AgentNotificationCopy.done(agent: "claude", detail: nil, place: "mac-mini",
                                   duration: 20, sound: true, threshold: 180, into: content)
        #expect(content.title == "✓ claude finished")
        #expect(content.body == "mac-mini")
        #expect(content.sound == nil)
        #expect(content.interruptionLevel == .passive)
    }

    @Test("a needs-you card carries the question, and the count of further prompts on the name")
    func attentionCard() {
        let content = UNMutableNotificationContent()
        AgentNotificationCopy.attention(agent: "claude", standingCount: 1,
                                        detail: "Bash: rm -rf build", place: "mac-mini · work",
                                        edge: true, sound: true, into: content)
        #expect(content.title == "claude needs you")
        #expect(content.body == "Bash: rm -rf build — mac-mini · work")
        #expect(content.sound != nil)
        #expect(content.interruptionLevel == .timeSensitive)

        let more = UNMutableNotificationContent()
        AgentNotificationCopy.attention(agent: "claude", standingCount: 3,
                                        detail: nil, place: "mac-mini",
                                        edge: false, sound: true, into: more)
        #expect(more.title == "claude +2 needs you")
        #expect(more.body == "mac-mini")
        #expect(more.sound == nil)
        #expect(more.interruptionLevel == .passive)
    }
}
