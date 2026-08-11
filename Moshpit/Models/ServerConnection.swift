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

    /// Binary probed on PATH and used to boot the session. `nil` for `.none`.
    var binaryName: String? {
        switch self {
        case .none:  return nil
        case .tmux:  return "tmux"
        case .herdr: return "herdr"
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

    // MARK: - Enhance page fields

    /// Startup command to run after connection (e.g. "cd ~/project")
    var startupCommand: String?
    /// Forward SSH agent to remote host
    var forwardAgent: Bool
    /// Enable SSH compression
    var compression: Bool
    /// Keep-alive interval in seconds (0 = disabled)
    var keepAliveInterval: Int
    /// Jump host for ProxyJump (e.g. "bastion.example.com")
    var jumpHost: String?
    /// Route connection through Tailscale VPN
    var useTailscale: Bool
    /// Auto-attach to tmux session if only one exists
    var tmuxAutoAttach: Bool
    /// Offer to create a new tmux session
    var tmuxCreateNew: Bool
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
    /// Reconnect over 5G/LTE when Wi-Fi drops (prototype "Roam on Cellular").
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

    var sshKeyAlgorithm: SSHKeyAlgorithm? {
        sshKeyAlgorithmRaw.flatMap(SSHKeyAlgorithm.init(rawValue:))
    }

    var predictMode: PredictMode {
        get { predictModeRaw.flatMap(PredictMode.init(rawValue:)) ?? .adaptive }
        set { predictModeRaw = newValue.rawValue }
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

    /// Custom binary path for the SELECTED multiplexer, or nil to use PATH.
    /// A non-nil path means the user vouches for the binary, which skips the
    /// capability probe's degrade check (the probe walks PATH, not custom
    /// locations).
    var multiplexerPath: String? {
        switch multiplexer {
        case .none:  return nil
        case .tmux:  return tmuxPath
        case .herdr: return herdrPath
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
        startupCommand: String? = nil,
        forwardAgent: Bool = false,
        compression: Bool = false,
        keepAliveInterval: Int = 60,
        jumpHost: String? = nil,
        useTailscale: Bool = false,
        tmuxAutoAttach: Bool = true,
        tmuxCreateNew: Bool = true,
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
        self.startupCommand = startupCommand
        self.forwardAgent = forwardAgent
        self.compression = compression
        self.keepAliveInterval = keepAliveInterval
        self.jumpHost = jumpHost
        self.useTailscale = useTailscale
        self.tmuxAutoAttach = tmuxAutoAttach
        self.tmuxCreateNew = tmuxCreateNew
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
