import Foundation
import Testing

@testable import Moshpit

/// A protocol switch swaps a fresh session in for the one on screen. The
/// fresh one was never made the visible session: its tmux leg attached as
/// if the terminal were closed — pins handed back, the window's output
/// never painted — and after mosh gave way to SSH the pane froze.
@MainActor
@Suite("A session swapped in by a manual reconnect")
struct SessionHubReplaceTests {
    private func hub() -> SessionHub {
        SessionHub(metrics: SessionMetricsRegistry())
    }

    @Test("takes the screen from the session it replaces")
    func onScreen() async {
        let hub = hub()
        let connection = ServerConnection(host: "box", username: "u")
        let old = hub.prepare(connection)
        hub.visibleSession = old
        let fresh = await hub.replace(connection)
        #expect(fresh !== old)
        #expect(hub.visibleSession === fresh)
    }

    @Test("stays off screen when the one it replaces was off screen")
    func offScreen() async {
        let hub = hub()
        let shown = hub.prepare(ServerConnection(host: "other", username: "u"))
        hub.visibleSession = shown
        let connection = ServerConnection(host: "box", username: "u")
        let old = hub.prepare(connection)
        let fresh = await hub.replace(connection)
        #expect(fresh !== old)
        #expect(hub.visibleSession === shown)
    }
}
