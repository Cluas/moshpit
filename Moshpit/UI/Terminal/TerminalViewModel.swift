import Foundation
import Observation

/// What a terminal screen shows of its session's health: the coarse
/// ``Status``, the error card's ``errorMessage``, and the cards a connect
/// asks through (the host key, a key's passphrase). The Rust core dials and
/// redials (``RustSessionDriver``); this only says where it stands
/// (``adoptRustStatus(_:automatic:error:)``).
@Observable
@MainActor
final class TerminalViewModel {
    enum Status: Equatable {
        case idle
        case connecting
        case connected
        case reconnecting   // transport dropped; auto-reconnect in progress
        case disconnected
        case failed(String)
        /// The remote end finished the session for good — its shell exited,
        /// or mosh-server sent its shutdown — with what to say about it. Not
        /// a failure, and nothing redials it: reconnecting is the user's call.
        case ended(String)
    }

    /// A host-key decision the user must make before the connection proceeds.
    /// Set on the main actor while the core's handshake awaits `decide`.
    struct HostKeyPrompt: Identifiable {
        let id = UUID()
        let host: String
        let port: Int
        let fingerprint: String
        /// Non-nil when the host's key CHANGED (potential MITM) — carries the
        /// previously trusted fingerprint.
        let previousFingerprint: String?
        /// `ssh-rsa` when the server offers no better host-key algorithm
        /// (B564): the New Host card says so.
        var olderAlgorithm: String? = nil
        let decide: (Bool) -> Void
    }

    /// The passphrase of a key stored behind one (``StoredKey``), asked for
    /// once: the sign-in converts the key and keeps it that way. Set on the
    /// main actor while the core's sign-in awaits `decide`; nil = cancel.
    struct KeyPassphrasePrompt: Identifiable {
        let id = UUID()
        let host: String
        /// The last passphrase given didn't unlock the key.
        let wrong: Bool
        let decide: (String?) -> Void
    }

    /// What the server asks at sign-in that the saved password doesn't
    /// answer — a one-time code (B563) — in the server's own words. Set on
    /// the main actor while the core's sign-in awaits `decide`; nil =
    /// decline.
    struct SignInQuestion: Identifiable {
        let id = UUID()
        let host: String
        /// The server's words: its instructions, then the prompt.
        let words: String
        /// The answer is shown as typed (false: a secret).
        let echo: Bool
        let decide: (String?) -> Void
    }

    let connection: ServerConnection
    private(set) var status: Status = .idle
    var errorMessage: String?
    /// The last connect ended because the user said no to the server's key
    /// (B151): the new-host card's Cancel, the changed-key card's Disconnect.
    /// Their own decision, not a failure — no error card; the screen leaves
    /// and the hub retires the session the way Disconnect does.
    private(set) var declinedHostKey = false

    /// What every surface showing this session's health should read.
    ///
    /// Lives here because it used to live in three places — the Terminal
    /// screen, the transport pill and the home card's row each derived it, and
    /// each derived it differently, so one reconnect could be blue on one
    /// surface, red on another and amber on a third. `status` alone is not
    /// enough to derive it (see the collapse below), which is exactly why
    /// duplicating the derivation went wrong.
    var connState: TransportConnState {
        // One state for the whole automatic reconnect, however many attempts it
        // takes: `status` flips connecting → failed → connecting on every
        // keepalive tick, and letting that through made the screen alternate
        // between "opening the pit" and "line dropped".
        //
        // It collapses to `.reconnecting` and not `.offline` because the app is
        // dialling — `.offline` means nothing is being attempted. Reporting
        // offline here turned the connecting screen red mid-reconnect, and since
        // this flag and `status` change on their own schedules, a single
        // reconnect crossed from blue to red partway through.
        if isAutoReconnectInFlight { return .reconnecting }
        switch status {
        case .connected: return .live
        case .reconnecting: return .reconnecting
        case .connecting, .idle: return .connecting
        case .failed, .disconnected, .ended: return .offline
        }
    }

    /// The remote end finished this session (``Status/ended(_:)``) — the
    /// terminal names it for what it is, not a dropped line or an error.
    var endedByServer: Bool {
        if case .ended = status { return true }
        return false
    }

    /// True from the moment the app starts re-establishing a transport the user
    /// didn't ask to re-establish, until it's back.
    ///
    /// A failure the user asked for is news — they tapped connect and it didn't
    /// happen, so a modal saying why is right. A failure inside an automatic
    /// retry is not news: the core is already dialling again, and the
    /// connecting screen has copy for exactly this. Surfacing both
    /// meant a dropped line showed a modal error card over the terminal, then
    /// the card vanished and "opening the pit" took its place, then the card
    /// came back — two things saying the same thing, in a loop, neither of them
    /// dismissable in any way that helped.
    ///
    /// Set from the core's state (``adoptRustStatus(_:automatic:error:)``).
    private(set) var isAutoReconnectInFlight = false

    /// Pending TOFU confirmation; the Terminal screen presents it as an alert.
    var hostKeyPrompt: HostKeyPrompt?
    /// Pending key passphrase; presented as a card beside `hostKeyPrompt`.
    var keyPassphrasePrompt: KeyPassphrasePrompt?
    /// Pending question from the server at sign-in; a card like the
    /// passphrase's.
    var signInQuestion: SignInQuestion?

    init(connection: ServerConnection) {
        self.connection = connection
    }
}

// MARK: - A session on the Rust core (W14a)

extension TerminalViewModel {
    /// The Rust core's session state, mirrored where every surface reads a
    /// session's health (``RustSessionDriver``): the core dials and redials,
    /// this only says where it stands. `automatic`: nobody asked for what is
    /// happening (a redial) — the collapse into `.reconnecting` and the no-modal
    /// rule of ``isAutoReconnectInFlight``. `error` raises the modal card.
    func adoptRustStatus(_ status: Status, automatic: Bool, error: String? = nil) {
        self.status = status
        isAutoReconnectInFlight = automatic
        if let error { errorMessage = error }
    }

    /// The person's own no to the server's key, on the Rust core: the
    /// connect ends quietly (B151, B334) — disconnected, not failed, no
    /// card, and ``declinedHostKey`` for the terminal to leave on.
    func adoptRustDeclined() {
        status = .disconnected
        isAutoReconnectInFlight = false
        errorMessage = nil
        declinedHostKey = true
    }
}
