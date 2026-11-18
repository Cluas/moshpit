import Foundation

enum AuthMethod: String, Codable, CaseIterable {
    case password
    case key
}

enum ConnectionProtocol: String, Codable, CaseIterable {
    case ssh
    case mosh

    var label: String {
        switch self {
        case .ssh: return "SSH"
        case .mosh: return "Mosh"
        }
    }

    var icon: String {
        switch self {
        case .ssh: return "lock.fill"
        case .mosh: return "antenna.radiowaves.left.and.right"
        }
    }
}

/// Which multiplexer a connection drives on the remote host.
///
/// tmux and herdr COEXIST rather than replace one another: they are separate
/// servers holding unrelated sessions, so a host missing the chosen one is
/// never silently switched to the other — that would show the user someone
/// else's work and call it theirs. The choice is per connection because one
/// person's hosts rarely all have the same tools installed.
enum Multiplexer: String, Codable, CaseIterable, Sendable {
    /// No multiplexer — one plain shell pane, no session persistence.
    case none
    case tmux
    /// herdr (<https://herdr.dev>) — a multiplexer built for CLI coding
    /// agents. It tracks agent status itself, so the Vibe Island needs no
    /// host-side hooks installed.
    case herdr

    var label: String {
        switch self {
        case .none:  return String(localized: "None")
        case .tmux:  return "tmux"
        case .herdr: return "herdr"
        }
    }

    /// One-line "what am I picking" for the form row. Deliberately concrete —
    /// the user is choosing between two things they may never have compared.
    var subtitle: String {
        switch self {
        case .none:
            return String(localized: "Single shell, no session persistence")
        case .tmux:
            return String(localized: "Mature, already on nearly every host")
        case .herdr:
            return String(localized: "Built for coding agents — agent status needs no hooks")
        }
    }

    /// SF Symbol standing in for this choice, so the picker reads at a glance
    /// instead of asking you to parse two lowercase tool names.
    ///
    /// Each one says what the thing IS rather than decorating it: a single
    /// shell, a split grid, and the agent sparkle the workbench already uses
    /// for every agent row — herdr's whole point is that it tracks agents.
    var symbol: String {
        switch self {
        case .none:  return "terminal"
        case .tmux:  return "rectangle.split.2x2"
        case .herdr: return "sparkles"
        }
    }
}

struct ServerConnection: Identifiable, Codable, Equatable {
    var id: UUID
    var name: String
    var host: String
    var port: Int
    var username: String
    var authMethod: AuthMethod
    var connectionProtocol: ConnectionProtocol
    /// SSH port for mosh bootstrap (default 22)
    var sshPort: Int
    /// Mosh UDP port range start (default 60001)
    var moshPortRangeStart: Int
    /// Mosh UDP port range end (default 60999)
    var moshPortRangeEnd: Int
    /// Custom mosh-server binary path on the remote host (e.g. /usr/local/bin/mosh-server)
    var moshServerPath: String?
    /// For password auth, stored in Keychain (not here).
    /// For key auth, this is the Keychain reference ID.
    var keychainRef: String?
    var createdAt: Date
    var lastConnectedAt: Date?

    // MARK: - Legacy

    // The prototype's "Enhance" page also stored startupCommand,
    // forwardAgent, keepAliveInterval, jumpHost, useTailscale, tmuxAutoAttach
    // and tmuxCreateNew (and compression). Nothing ever read them, so they
    // are gone; lists that still carry them decode fine, since unknown keys
    // are ignored, and the store keeps writing the non-optional ones for
    // builds that still require them (crates/store's `rollback_fields`).

    /// Enable tmux integration.
    ///
    /// LEGACY — superseded by ``multiplexer``. Kept as a stored field so
    /// connections saved before the multiplexer picker existed still decode to
    /// the same behaviour, and so rolling back to an older build doesn't
    /// silently drop a user's tmux setting. `multiplexerRaw` wins when present.
    var useTmux: Bool

    // MARK: - Prototype v2 fields (optional so stored JSON keeps decoding)

    /// Mosh predictive-echo mode ("adaptive" default). Stored raw so the
    /// struct stays Codable-stable.
    var predictModeRaw: String?
    /// Kept as stored, read by nothing (B530): its switch ("Roam on
    /// Cellular") is gone; the key stays so an older build reads it.
    var roamOnCellular: Bool?
    /// Custom tmux binary path on the remote host.
    var tmuxPath: String?
    /// Custom herdr binary path on the remote host.
    var herdrPath: String?
    /// Selected multiplexer, stored raw so the struct stays Codable-stable.
    /// `nil` on anything saved before the picker existed — ``multiplexer``
    /// then falls back to ``useTmux``.
    var multiplexerRaw: String?
    /// When the user picked a managed key from SSH Keys (instead of pasting a
    /// PEM), these reference the `SSHKeyRecord`: id for UI display, raw
    /// algorithm so `SSHService` knows how to interpret the keychain blob
    /// (Secure Enclave keys are a `dataRepresentation`, not PEM).
    var sshKeyId: UUID?
    var sshKeyAlgorithmRaw: String?
    /// Route this connection's SSH dial through a SOCKS5 proxy (Add
    /// Connection → Proxy). Optional, like the rest of this section, so
    /// connections saved before this field existed keep decoding — `nil`
    /// behaves exactly like `false`, dialing `host`/`port` directly.
    var useSOCKSProxy: Bool?
    var socksProxyHost: String?
    /// `nil` ⇒ 1080, the conventional SOCKS5 default.
    var socksProxyPort: Int?
    /// The emoji its mark shows in place of its host key's face (one
    /// emoji, checked by the Rust form model). Optional: connections saved
    /// before it existed decode as `nil`, the face.
    var emoji: String?
    /// The saved connection this one is reached through (a jump host,
    /// `ssh -J`; F1), picked in the NSH form. Optional: `nil` goes straight
    /// to the host. The legacy `jumpHost` string is another key, unread.
    var viaConnectionId: UUID?
    /// Made only to be gone through: Home doesn't list it, and every
    /// "Connect via" picker does. Optional so connections saved before it
    /// existed keep decoding: `nil` is off.
    var jumpHostOnly: Bool?
    /// How mosh's UDP gets to this host through its jump host: `ssh` (the
    /// whole connection over SSH), `direct` (UDP straight to the host) or
    /// `viaJump` (UDP to the jump host, which forwards it). Stored raw, as
    /// the core spells it (`JumpMoshRoute::raw_value`); `nil` and anything
    /// unknown read as `ssh`. Optional so older connections keep decoding.
    var jumpMoshRoute: String?

    /// Whether a connect to this goes to another box than one to `before`:
    /// another host (case aside, as DNS compares names), port or SSH port
    /// (`ServerConnection::points_elsewhere_than` in crates/store).
    func pointsElsewhere(than before: ServerConnection) -> Bool {
        host.caseInsensitiveCompare(before.host) != .orderedSame
            || port != before.port || sshPort != before.sshPort
    }

    var sshKeyAlgorithm: SSHKeyAlgorithm? {
        sshKeyAlgorithmRaw.flatMap(SSHKeyAlgorithm.init(rawValue:))
    }

    var predictMode: PredictMode {
        get { predictModeRaw.flatMap(PredictMode.init(rawValue:)) ?? .adaptive }
        set { predictModeRaw = newValue.rawValue }
    }

    /// The connection's own predictive echo; nil: as Settings says (B531,
    /// the store's `ServerConnection::predict`). "adaptive" has always meant
    /// that, as no value and one this build doesn't know do; a connection
    /// pinning Adaptive itself is "pinnedAdaptive".
    var predictOverride: PredictMode? {
        switch predictModeRaw {
        case nil, "adaptive"?: nil
        case "pinnedAdaptive"?: .adaptive
        case let raw?: PredictMode(rawValue: raw)
        }
    }

    /// Which multiplexer to drive. Migration: stored JSON written before the
    /// picker existed has no `multiplexerRaw`, and `useTmux` was hardcoded
    /// `true` by the form — so those connections land on `.tmux` and behave
    /// exactly as they did.
    var multiplexer: Multiplexer {
        get { multiplexerRaw.flatMap(Multiplexer.init(rawValue:)) ?? (useTmux ? .tmux : .none) }
        set {
            multiplexerRaw = newValue.rawValue
            // Keep the legacy flag consistent with the new one so an older
            // build reading the same store makes the same decision.
            useTmux = newValue == .tmux
        }
    }

    init(
        id: UUID = UUID(),
        name: String = "",
        host: String = "",
        port: Int = 22,
        username: String = "",
        authMethod: AuthMethod = .password,
        connectionProtocol: ConnectionProtocol = .ssh,
        sshPort: Int = 22,
        moshPortRangeStart: Int = 60001,
        moshPortRangeEnd: Int = 60999,
        moshServerPath: String? = nil,
        keychainRef: String? = nil,
        createdAt: Date = Date(),
        lastConnectedAt: Date? = nil,
        useTmux: Bool = false,
        predictModeRaw: String? = nil,
        roamOnCellular: Bool? = nil,
        tmuxPath: String? = nil,
        herdrPath: String? = nil,
        multiplexerRaw: String? = nil
    ) {
        self.id = id
        self.name = name
        self.host = host
        self.port = port
        self.username = username
        self.authMethod = authMethod
        self.connectionProtocol = connectionProtocol
        self.sshPort = sshPort
        self.moshPortRangeStart = moshPortRangeStart
        self.moshPortRangeEnd = moshPortRangeEnd
        self.moshServerPath = moshServerPath
        self.keychainRef = keychainRef
        self.createdAt = createdAt
        self.lastConnectedAt = lastConnectedAt
        self.useTmux = useTmux
        self.predictModeRaw = predictModeRaw
        self.roamOnCellular = roamOnCellular
        self.tmuxPath = tmuxPath
        self.herdrPath = herdrPath
        self.multiplexerRaw = multiplexerRaw
    }

    var displayName: String {
        name.isEmpty ? "\(username)@\(host)" : name
    }
}

extension ServerConnection {
    /// How the jump route reads: unknown or missing is `ssh`, as the core
    /// reads it.
    var jumpRoute: JumpMoshRoute { jumpMoshRoute.flatMap(JumpMoshRoute.init(rawValue:)) ?? .ssh }

    /// Whether the core dials this connection over mosh. It is the core's
    /// own rule (`ConnectionSpec::dialled_protocol`, through the FFI's
    /// `dialsMosh`), never a second copy of it: the surface must follow what
    /// was dialled, not the setting, or an SSH herdr session is read as a
    /// mosh one and lands on the tmux empty state. Through a jump host that
    /// is mosh only on a route that isn't SSH.
    var dialsMosh: Bool {
        coreDialsMosh(mosh: connectionProtocol == .mosh, throughJump: viaConnectionId != nil,
                          jumpMoshRoute: jumpMoshRoute)
    }

    /// The connection goes through a jump host, and its mosh UDP takes a
    /// route of its own (not SSH).
    var dialsMoshThroughJump: Bool {
        dialsMosh && viaConnectionId != nil
    }
}

/// How mosh's UDP gets through a jump host (``ServerConnection/jumpMoshRoute``).
enum JumpMoshRoute: String {
    case ssh, direct, viaJump
}
