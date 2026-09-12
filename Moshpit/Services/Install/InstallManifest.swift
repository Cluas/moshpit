import Foundation

/// What Moshpit has installed on a host, as recorded on that host.
///
/// `~/.moshpit/manifest.json`. It exists because the install flow it replaces
/// had no memory at all: it could not say what was installed, could not tell a
/// current copy from a stale one, and offered no way to remove anything. The
/// concrete cost of that showed up the day the stamp script gained a push
/// hand-off — every already-installed copy silently stopped being current, and
/// nothing anywhere could notice.
///
/// The manifest is read, merged and written by the APP, never by shell on the
/// host. That is the point: JSON editing in POSIX `sh` is what forced the old
/// installer to carry a jq program AND a python fallback AND a heredoc to hold
/// them, which is most of why it was 6.5 KB of paste. Swift has a JSON parser;
/// the host only needs `cat` and `base64 -d`.
struct InstallManifest: Codable, Equatable {

    /// Bumped only if this file's SHAPE changes. An unknown schema is treated as
    /// "nothing installed" rather than misread — a wrong read here would offer
    /// to uninstall files it cannot name.
    static let currentSchema = 1

    var schema: Int
    var components: [String: Component]

    struct Component: Codable, Equatable {
        /// SHA-256 of the file's content as the app wrote it. Absent for
        /// components that are not a single file (a hook registration).
        var digest: String?
        var installedAt: Date
        /// For `hooks.<agent>`: which config was edited. Recorded so uninstall
        /// edits the same file the install did, even if the agent's default path
        /// changes later.
        var configPath: String?
        /// For `pairing`: which connection and relay it belongs to. NEVER the
        /// secret — that lives only in `push.conf`, mode 0600.
        var connectionId: String?
        var relayURL: String?

        init(digest: String? = nil, installedAt: Date, configPath: String? = nil,
             connectionId: String? = nil, relayURL: String? = nil) {
            self.digest = digest
            self.installedAt = installedAt
            self.configPath = configPath
            self.connectionId = connectionId
            self.relayURL = relayURL
        }
    }

    static let empty = InstallManifest(schema: currentSchema, components: [:])

    static let path = "\(HostCommands.dir)/manifest.json"

    subscript(_ component: InstallComponent) -> Component? {
        get { components[component.key] }
        set { components[component.key] = newValue }
    }

    /// This DEVICE's pairing entry, wherever an install of any age put it:
    /// under the per-device key, or under the legacy shared "pairing" key when
    /// that record names the same connection. A legacy entry naming a DIFFERENT
    /// connection belongs to another device and is nobody else's business.
    func pairingEntry(conn: String) -> Component? {
        if let mine = components["pairing.\(conn)"] { return mine }
        if let legacy = components["pairing"], legacy.connectionId == conn { return legacy }
        return nil
    }

    /// Every device pairing recorded on this host, for surfaces that show the
    /// host's state rather than one device's.
    var pairingEntries: [Component] {
        components.compactMap { key, value in
            key == "pairing" || key.hasPrefix("pairing.") ? value : nil
        }
    }

    mutating func removePairing(conn: String) {
        components["pairing.\(conn)"] = nil
        if components["pairing"]?.connectionId == conn { components["pairing"] = nil }
    }

    /// Read the manifest off a host. An absent, empty, corrupt or
    /// future-schema file all yield `.empty` — every caller's next move is the
    /// same (offer a fresh install), and guessing at a file we cannot parse is
    /// how an uninstall deletes the wrong thing.
    static func read(from channel: HostChannel) async throws -> InstallManifest {
        let text = try await channel.runText(HostCommands.readFile(path: path))
        guard !text.isEmpty, let data = text.data(using: .utf8),
              let manifest = try? JSONDecoder.moshpitInstall.decode(InstallManifest.self, from: data),
              manifest.schema == currentSchema
        else { return .empty }
        return manifest
    }

    /// Write the manifest back. Not 0600: it holds no secret, and a
    /// world-readable manifest is one a user can read to see what we did.
    func write(to channel: HostChannel) async throws {
        let data = try JSONEncoder.moshpitInstall.encode(self)
        _ = try await channel.run(HostCommands.writeFile(
            path: Self.path, mode: "644",
            base64: data.base64EncodedString()))
    }
}

/// One thing the installer can put on a host.
enum InstallComponent: Hashable {

    /// Where a pre-multi-device install kept its single pairing. Read (and
    /// removed on unpair) for compatibility; never written any more.
    static let legacyPairingPath = "\(HostCommands.dir)/push.conf"

    /// `~/.moshpit/moshpit-stamp.sh` — what an agent hook calls.
    case stamp
    /// `~/.moshpit/moshpit-push.sh` — what turns a status into a push.
    case sender
    /// `~/.moshpit/moshpit-await.sh` — what collects the phone's answer to a
    /// prompt and presses the key, since the phone cannot deliver it itself.
    /// A hook registration inside one agent's own config.
    case hooks(agent: String)
    /// `~/.moshpit/push.d/<conn>.conf` — the relay address and one paired
    /// DEVICE's secrets, keyed by that device's connection id. One file per
    /// device: a single shared `push.conf` quietly meant one phone per host —
    /// the second device's pairing overwrote the first's secret and its
    /// notifications just stopped, with nothing anywhere saying why. (The
    /// legacy single file is still read by the sender and still uninstalled by
    /// ``legacyPairingPath``.)
    case pairing(conn: String)

    var key: String {
        switch self {
        case .stamp:            return "stamp"
        case .sender:           return "sender"
        case .hooks(let agent): return "hooks.\(agent)"
        case .pairing(let conn): return "pairing.\(conn)"
        }
    }

    /// Absolute path for the components that are a file.
    var path: String? {
        switch self {
        case .stamp:   return "\(HostCommands.dir)/moshpit-stamp.sh"
        case .sender:  return "\(HostCommands.dir)/moshpit-push.sh"
        case .pairing(let conn): return "\(HostCommands.dir)/push.d/\(conn).conf"
        case .hooks:   return nil
        }
    }

    /// Mode a fresh copy is written with. The sender and stamp are executable;
    /// `push.conf` holds two secrets and is readable only by its owner.
    var mode: String {
        switch self {
        case .stamp, .sender: return "755"
        case .pairing:        return "600"
        case .hooks:          return "644"
        }
    }
}

/// What a host can and cannot do, in one round trip.
///
/// Answers gathered BEFORE anything is written, so a host missing a tool is told
/// so instead of installing something that will fail quietly at 3am. The old
/// flow checked none of this: on a host without `jq` or `python3` its config
/// merge simply did not happen, and the sheet still said the same thing it
/// always said.
struct HostFacts: Equatable {
    var home: String = ""
    var shell: String = ""
    var uname: String = ""
    var tools: [String: Bool] = [:]

    var hasOpenSSL: Bool { tools["openssl"] == true }
    var hasCurl: Bool { tools["curl"] == true }
    var hasBase64: Bool { tools["base64"] == true }
    var hasTmux: Bool { tools["tmux"] == true }
    /// Optional: without it the stamp script still reports state, just no title.
    var hasJq: Bool { tools["jq"] == true }

    /// Everything the push path needs. `base64` is in here because it is how the
    /// installer lands files at all — a host without it cannot be installed to
    /// by this engine, and saying that up front beats a confusing write failure.
    var canPush: Bool { hasOpenSSL && hasCurl && hasBase64 }

    /// Human-readable list of what is missing for pushes to work.
    var missingForPush: [String] {
        var missing: [String] = []
        if !hasOpenSSL { missing.append("openssl") }
        if !hasCurl { missing.append("curl") }
        if !hasBase64 { missing.append("base64") }
        return missing
    }

    static func parse(_ output: String) -> HostFacts {
        var facts = HostFacts()
        for line in output.split(separator: "\n", omittingEmptySubsequences: true) {
            guard let eq = line.firstIndex(of: "=") else { continue }
            let key = String(line[line.startIndex..<eq])
            let value = String(line[line.index(after: eq)...])
                .trimmingCharacters(in: .whitespaces)
            switch key {
            case "home":  facts.home = value
            case "shell": facts.shell = value
            case "uname": facts.uname = value
            default:      facts.tools[key] = (value == "yes")
            }
        }
        return facts
    }
}

extension JSONDecoder {
    /// ISO-8601 dates so a human reading `manifest.json` on their own server
    /// sees a date, not a float since 2001.
    static let moshpitInstall: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}

extension JSONEncoder {
    static let moshpitInstall: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }()
}
