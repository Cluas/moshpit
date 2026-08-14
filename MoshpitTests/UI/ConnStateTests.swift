import Foundation
import Testing
@testable import Moshpit

/// `TerminalViewModel.connState` — the one derivation every surface that shows a
/// session's health reads.
///
/// It used to be derived independently by the Terminal screen, the transport
/// pill and the home card's row, and the three disagreed: a reconnect showed
/// blue on one, red on another and amber on the third, and the Terminal screen
/// visibly changed hue partway through a single reconnect because the flag and
/// the status it read flip on their own schedules.
@Suite("TransportConnState derivation")
@MainActor
struct ConnStateTests {

    private func makeViewModel() -> TerminalViewModel {
        let connection = ServerConnection(
            name: "test",
            host: "192.0.2.1",   // RFC5737 TEST-NET-1 — guaranteed non-routable
            port: 22,
            username: "tester",
            authMethod: .password)
        return TerminalViewModel(connection: connection)
    }

    @Test("an automatic reconnect reads as reconnecting, never offline")
    func automaticReconnectIsNotOffline() {
        let vm = makeViewModel()
        // `markReconnecting` only fires from a previously-live state — it must
        // never interrupt an initial connect — so the session has to be up
        // before it can drop.
        vm.markConnected()
        #expect(vm.connState == .live)

        vm.markReconnecting()
        #expect(vm.connState == .reconnecting)

        // The retry attempt begins: `beginAttempt(automatic:)` raises the flag,
        // and the attempt itself will push `status` back to `.connecting`. Both
        // of those used to be able to win, which is what made one reconnect
        // cross from blue to red.
        vm.beginAttempt(automatic: true)
        #expect(vm.connState == .reconnecting,
                "an in-flight automatic reconnect must not report offline — the app is dialling")
    }

    @Test("a fresh view model reads as connecting, not offline")
    func idleIsConnecting() {
        // `.idle` is "no transport yet", which on every surface that renders it
        // means a connection is being opened.
        #expect(makeViewModel().connState == .connecting)
    }

    @Test("a user-initiated attempt is connecting, not reconnecting")
    func manualAttemptIsConnecting() {
        let vm = makeViewModel()
        vm.beginAttempt(automatic: false)
        #expect(vm.connState == .connecting)
    }

    @Test("only a state with nothing in flight earns offline — and its own colour")
    func offlineIsReservedForNoAttempt() {
        let vm = makeViewModel()
        vm.fail("host unreachable")
        #expect(vm.connState == .offline)
        // Red belongs to this state alone; the two states that mean "working on
        // it" must not borrow it.
        #expect(TransportConnState.offline.transientTint == Ink.danger)
        #expect(TransportConnState.reconnecting.transientTint != Ink.danger)
        #expect(TransportConnState.connecting.transientTint != Ink.danger)
    }

    @Test("live has no transient tint, so a pill keeps its transport's own colour")
    func liveDefersToTransport() {
        // SSH and mosh read differently when healthy; only the transient states
        // override that.
        #expect(TransportConnState.live.transientTint == nil)
    }

    @Test("connecting and reconnecting are told apart by colour, not just wording")
    func transientStatesAreDistinct() {
        // The whole point of the consolidation: one event, one colour — but two
        // different events still have to look different.
        #expect(TransportConnState.connecting.transientTint
                != TransportConnState.reconnecting.transientTint)
    }
}
