import Foundation
import Testing
@testable import Moshpit

/// `AgentActivityMonitor` is a `@MainActor` state machine whose interesting
/// logic (output-heuristic promotion, the idle/done/linger sweep) is `private`
/// and driven exclusively by live `TmuxSessionController` callbacks and timers.
///
/// One decision has been lifted out of that: whether to ANNOUNCE an attention
/// episode. It used to be three terms inline in `applyAgentHooks`, and this
/// header used to note that it could not be reached "without a live tmux fixture
/// or source changes" — which was true, and which is why it shipped wrong and
/// re-notified on every reconnect. Untestable is not a property of the code; it
/// is a decision about where to put a boundary.
@Suite("AgentActivityMonitor")
@MainActor
struct AgentActivityMonitorTests {

    private func makeMonitor() -> (AgentActivityMonitor, UserDefaults, String) {
        let name = "test.agentmonitor.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        let settings = AppSettings(defaults: defaults)
        return (AgentActivityMonitor(settings: settings), defaults, name)
    }

    // MARK: - Announcing an attention episode

    // The reported bug: a reconnect re-notified for a prompt already on screen,
    // once per reconnect, for as long as the agent kept waiting.
    //
    // The decision used to be `!(wasAttention && wasHookOwned)` inline in
    // `applyAgentHooks`, where `wasHookOwned` is a FIVE-SECOND liveness window.
    // Any longer gap in hook polling — a reconnect, time in the background, a
    // slow host — made the sweep drop `hookOwned` while `.attention` stuck by
    // design, so the next poll read an unchanged prompt as a new one.
    //
    // This suite exists as much for its reachability as its assertions. The
    // comment at the top of this file used to say the hook path could not be
    // tested "without a live tmux fixture or source changes", and that was true
    // and is exactly why the bug shipped. The rule is now a static function.

    @Test("a standing prompt is announced once, not once per poll")
    func announcedOncePerEpisode() {
        let episode = Date(timeIntervalSince1970: 1_756_000_000)
        #expect(AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: episode, lastAnnounced: nil))
        // Every subsequent poll of the same standing prompt.
        #expect(!AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: episode, lastAnnounced: episode))
    }

    @Test("a reconnect does not re-announce a prompt that has not changed")
    func reconnectDoesNotReAnnounce() {
        // The exact sequence: announce, lose the connection for longer than
        // hookGrace, come back, poll. The pane is still in the same episode —
        // the host's `@moshpit_since` has not moved, because nothing happened.
        let episode = Date(timeIntervalSince1970: 1_756_000_000)
        var announced: Date?

        if AgentActivityMonitor.shouldAnnounce(state: .attention, episode: episode,
                                               lastAnnounced: announced) {
            announced = episode
        }
        // …disconnect, any length at all, then the first poll after reconnect.
        #expect(!AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: episode, lastAnnounced: announced),
                "a reconnect re-announced a prompt the user is already looking at")
        // And a second reconnect, because the old bug fired on every one.
        #expect(!AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: episode, lastAnnounced: announced))
    }

    @Test("a genuinely new prompt is announced, even on the same pane")
    func newEpisodeAnnouncesAgain() {
        // The other half, and the reason suppression is keyed on the episode
        // rather than just "have I ever announced this pane": the agent asked,
        // was answered, worked, and is now asking something else. That is a new
        // question and has to ring.
        let first = Date(timeIntervalSince1970: 1_756_000_000)
        let second = first.addingTimeInterval(90)
        #expect(AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: second, lastAnnounced: first))
    }

    @Test("only attention announces")
    func otherStatesAreSilent() {
        let episode = Date(timeIntervalSince1970: 1_756_000_000)
        for state in [AgentActivityAttributes.AgentState.working, .done, .idle] {
            #expect(!AgentActivityMonitor.shouldAnnounce(
                state: state, episode: episode, lastAnnounced: nil),
                    "\(state) asked for an attention notification")
        }
    }

    @Test("the record survives a relaunch, so a fresh launch does not re-announce")
    func announcedRecordRoundTrips() {
        // A device log caught FOUR notifications added within one millisecond of
        // a fresh app launch — every pane still standing in attention, announced
        // again, one of them re-added two seconds after being withdrawn. An
        // in-memory record fixes the reconnect and leaves that untouched.
        let episode = Date(timeIntervalSince1970: 1_756_000_000)
        let encoded = AgentActivityMonitor.encodeAnnounced(["conn|%3": episode])
        let back = AgentActivityMonitor.decodeAnnounced(
            encoded, now: episode.addingTimeInterval(300), ttl: 86_400)
        #expect(back["conn|%3"] == episode)
        #expect(!AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: episode, lastAnnounced: back["conn|%3"]))
    }

    @Test("an ancient record is dropped rather than suppressing forever")
    func staleRecordExpires() {
        let old = Date(timeIntervalSince1970: 1_756_000_000)
        let encoded = AgentActivityMonitor.encodeAnnounced(["conn|%3": old])
        let back = AgentActivityMonitor.decodeAnnounced(
            encoded, now: old.addingTimeInterval(86_400 + 60), ttl: 86_400)
        #expect(back.isEmpty)
        // …and with the record gone, a standing prompt can ring again.
        #expect(AgentActivityMonitor.shouldAnnounce(
            state: .attention, episode: old, lastAnnounced: back["conn|%3"]))
    }

    @Test("a record from the future is discarded, not trusted")
    func futureRecordDiscarded() {
        // A clock that jumped — timezone change, NTP correction, a restored
        // backup — must not be able to silence notifications until real time
        // catches up to it.
        let now = Date(timeIntervalSince1970: 1_756_000_000)
        let encoded = AgentActivityMonitor.encodeAnnounced(
            ["conn|%3": now.addingTimeInterval(7 * 86_400)])
        #expect(AgentActivityMonitor.decodeAnnounced(encoded, now: now, ttl: 86_400).isEmpty)
    }

    // MARK: - Reclassifying stamps the host got wrong

    // Found as "NEEDS YOU 我明明已经点开看了，那个状态一直都在": two panes on a
    // real phone frozen in attention for 16 and 24 HOURS. Older stamp scripts
    // recorded Claude's idle reminder as attention; the current one refuses
    // idle nags — which also means it never overwrites the fossils, and nothing
    // else ever would have.

    @Test("an idle-reminder attention is a parked agent, not a question")
    func idleTitleHeals() {
        #expect(AgentActivityMonitor.reclassify(
            state: .attention, title: "Claude is waiting for your input",
            foregroundCommand: "claude") == .done)
        // A real question keeps its urgency.
        #expect(AgentActivityMonitor.reclassify(
            state: .attention, title: "Bash: rm -rf build",
            foregroundCommand: "claude") == .attention)
        #expect(AgentActivityMonitor.reclassify(
            state: .attention, title: nil,
            foregroundCommand: "claude") == .attention)
    }

    @Test("a stamp whose agent is dead heals, whatever it says")
    func shellForegroundHeals() {
        // Hooks only ever write the options; a killed agent fires no final hook,
        // so its last state freezes at the moment of death. The pane's actual
        // foreground is the only truth available.
        for state in [AgentActivityAttributes.AgentState.attention, .working] {
            #expect(AgentActivityMonitor.reclassify(
                state: state, title: "Bash: make", foregroundCommand: "zsh") == .done,
                    "\(state) survived its agent")
        }
        // No foreground info (older controller data) changes nothing.
        #expect(AgentActivityMonitor.reclassify(
            state: .attention, title: "Bash: make", foregroundCommand: nil) == .attention)
    }

    @Test("attentionState is nil for a pane the monitor has no record of")
    func attentionStateNilWhenUntracked() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        // nil (not false) is the documented contract: callers must not block on
        // an absence of data, only on an explicit "no longer attention".
        #expect(monitor.attentionState(connectionId: UUID(), paneId: "%0") == nil)
    }

    @Test("untracking an unknown connection is a harmless no-op")
    func untrackUnknownIsSafe() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        monitor.untrack(connectionId: UUID())
        #expect(monitor.attentionState(connectionId: UUID(), paneId: "%1") == nil)
    }

    @Test("cycleHeadline is safe to call with no agents present")
    func cycleHeadlineOnEmptyStateIsSafe() {
        let (monitor, defaults, name) = makeMonitor()
        defer { defaults.removePersistentDomain(forName: name) }

        // No tracked panes → buildState() yields an empty agent list; cycling the
        // headline offset must wrap harmlessly rather than divide/index by zero.
        monitor.cycleHeadline()
        monitor.cycleHeadline()
        #expect(monitor.attentionState(connectionId: UUID(), paneId: "%0") == nil)
    }
}
