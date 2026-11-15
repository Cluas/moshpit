import Foundation
import SwiftUI
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

    // Every state below is set the way ``RustSessionDriver`` sets it from the
    // core's session state (``TerminalViewModel/adoptRustStatus(_:automatic:error:)``).

    @Test("an automatic redial reads as reconnecting, never offline")
    func automaticRedialIsNotOffline() {
        let vm = makeViewModel()
        vm.adoptRustStatus(.connected, automatic: false)
        #expect(vm.connState == .live)

        // The link dropped and the core redials on its own.
        vm.adoptRustStatus(.reconnecting, automatic: true)
        #expect(vm.connState == .reconnecting)

        // The redial's own attempt reports connecting; the flag keeps it
        // reconnecting. Both used to be able to win, which is what made one
        // reconnect cross from blue to red.
        vm.adoptRustStatus(.connecting, automatic: true)
        #expect(vm.connState == .reconnecting,
                "an in-flight automatic redial must not report offline: the core is dialling")
    }

    @Test("a fresh view model reads as connecting, not offline")
    func idleIsConnecting() {
        // `.idle` is "no transport yet", which on every surface that renders it
        // means a connection is being opened.
        #expect(makeViewModel().connState == .connecting)
    }

    @Test("a connect the person asked for is connecting, not reconnecting")
    func manualAttemptIsConnecting() {
        let vm = makeViewModel()
        vm.adoptRustStatus(.connecting, automatic: false)
        #expect(vm.connState == .connecting)
    }

    @Test("only a state with nothing in flight earns offline")
    func offlineIsReservedForNoAttempt() {
        let failed = makeViewModel()
        failed.adoptRustStatus(.failed("host unreachable"), automatic: false, error: "host unreachable")
        #expect(failed.connState == .offline)
        #expect(failed.errorMessage == "host unreachable")

        let down = makeViewModel()
        down.adoptRustStatus(.disconnected, automatic: false)
        #expect(down.connState == .offline)
        #expect(down.errorMessage == nil, "a plain disconnect raises no card")
    }

    // MARK: A session the server ended

    /// `exit` in the remote shell, or mosh-server's shutdown. It came through
    /// as a failure — "Connection Error" over a "LINE DROPPED" veil — though
    /// nothing failed and nothing dropped.
    @Test("a session the server ended is its own state: offline, said, and not a failure")
    func endedIsItsOwnState() {
        let ended = "The session ended: the remote shell exited."
        let vm = makeViewModel()
        vm.adoptRustStatus(.connected, automatic: false)

        vm.adoptRustStatus(.ended(ended), automatic: false, error: ended)

        #expect(vm.status == .ended(ended))
        #expect(vm.endedByServer)
        #expect(vm.connState == .offline, "nothing is being attempted")
        #expect(vm.errorMessage == ended)

        // A failure is still a failure.
        let failed = makeViewModel()
        failed.adoptRustStatus(.failed("host unreachable"), automatic: false, error: "host unreachable")
        #expect(!failed.endedByServer)
    }

    /// The end is said even when the last attempt was an automatic one —
    /// unlike a failure inside an automatic retry, nothing retries an end.
    @Test("an end is said even after an automatic redial")
    func endIsAlwaysSaid() {
        let ended = "The session ended: the remote shell exited."
        let vm = makeViewModel()
        vm.adoptRustStatus(.reconnecting, automatic: true)
        vm.adoptRustStatus(.connected, automatic: false)
        vm.adoptRustStatus(.reconnecting, automatic: true)

        vm.adoptRustStatus(.ended(ended), automatic: false, error: ended)

        #expect(vm.errorMessage == ended)
        #expect(!vm.isAutoReconnectInFlight)
        #expect(vm.connState == .offline)
    }

    /// B323: on the Rust core herdr's veil waits for herdr's first frame,
    /// as tmux's does — on the grace it lifted 500 ms after transport-up,
    /// over a stale frame whose input herdr still dropped.
    @Test("herdr's reconnect veil waits for its first frame, as tmux's does")
    func herdrWaitsForItsFirstFrame() {
        let waits = TerminalScreen.waitsForFirstFrame
        #expect(waits(.ssh, .herdr, false, true))
        #expect(waits(.ssh, .tmux, false, true))
        #expect(waits(.ssh, .tmux, false, false))
        // The Swift path's herdr reports no first frame: the grace.
        #expect(!waits(.ssh, .herdr, false, false))
        // mosh, a plain shell and a degraded connect paint into the
        // persistent terminal.
        #expect(!waits(.mosh, .herdr, false, true))
        #expect(!waits(.mosh, .tmux, false, true))
        #expect(!waits(.ssh, .none, false, true))
        #expect(!waits(.ssh, .tmux, true, true))
        #expect(!waits(.ssh, .herdr, true, true))
    }

    /// B324: a redial the person barely saw buzzed twice within a second;
    /// the tap on the way back is for a drop they waited through.
    @Test("the way back taps only after a drop the person waited through")
    func theWayBackTapsOnlyAfterAVisibleDrop() {
        let feedback = TerminalScreen.connectionFeedback
        let dropped = Date(timeIntervalSinceReferenceDate: 800_000_000)
        #expect(feedback(.live, .reconnecting, nil, dropped) == .warning)
        #expect(feedback(.live, .offline, nil, dropped) == .warning)
        #expect(feedback(.reconnecting, .live, dropped, dropped.addingTimeInterval(0.6)) == nil)
        #expect(feedback(.reconnecting, .live, dropped, dropped.addingTimeInterval(0.999)) == nil)
        #expect(feedback(.reconnecting, .live, dropped, dropped.addingTimeInterval(1.0)) == .success)
        #expect(feedback(.reconnecting, .live, dropped, dropped.addingTimeInterval(7)) == .success)
        // No drop on record: not seen, no tap. (Entering mid-reconnect
        // starts the clock too, so this is only a screen that never saw one.)
        #expect(feedback(.reconnecting, .live, nil, dropped) == nil)
        // A first connect always taps; nothing else does.
        #expect(feedback(.connecting, .live, nil, dropped) == .success)
        #expect(feedback(.reconnecting, .offline, dropped, dropped) == nil)
        #expect(feedback(.connecting, .reconnecting, nil, dropped) == nil)
    }
}
