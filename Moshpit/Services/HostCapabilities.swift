import Foundation

/// What a target host can actually do — probed once per connection over the
/// first SSH channel, then cached. Drives the degrade matrix in ``SessionHub``
/// (no tmux → plain SSH, no mosh-server → SSH fallback) and the Install Assist
/// sheet's per-distro command.
///
/// Degrading is never blocking: a missing dependency switches the session to a
/// lesser-but-working transport and raises a dismissible banner — it does not
/// fail the connection.
struct HostCapabilities: Codable, Equatable, Sendable {
    var hasTmux: Bool
    var hasMoshServer: Bool
    /// `uname -s` output, e.g. "Linux", "Darwin". Empty when unknown.
    var os: String
    /// First package manager found on PATH (the probe walks a fixed list);
    /// `nil` when none matched — Install Assist then shows generic guidance.
    var packageManager: PackageManager?
    /// herdr on PATH — the alternative multiplexer (see ``Multiplexer``).
    var hasHerdr: Bool
    /// python3 on PATH. It gated the Swift herdr client's push pump (gone,
    /// E1); the Rust core probes for its own. Kept in the probe and the
    /// cache's stored shape.
    var hasPython3: Bool

    init(hasTmux: Bool, hasMoshServer: Bool, os: String,
         packageManager: PackageManager?, hasHerdr: Bool = false,
         hasPython3: Bool = false) {
        self.hasTmux = hasTmux
        self.hasMoshServer = hasMoshServer
        self.os = os
        self.packageManager = packageManager
        self.hasHerdr = hasHerdr
        self.hasPython3 = hasPython3
    }

    /// "Everything present" — the optimistic default used before the first
    /// probe completes, so a cold session behaves exactly as it does today and
    /// only degrades if the probe actually finds something missing.
    static let unknown = HostCapabilities(
        hasTmux: true, hasMoshServer: true, os: "", packageManager: nil, hasHerdr: true,
        hasPython3: true)

    /// The directories added to a host's `PATH` for the probe and the
    /// core's commands (`moshpit_install::capabilities::EXTRA_PATH_DIRS`): a
    /// non-login exec shell sources no profile, so Homebrew's, `/usr/local`'s
    /// and `$HOME/.local/bin` (herdr's installer) would look missing.
    static let extraPathDirs = hostExtraPathDirs()

    /// The probe a connection runs on its host, the core's
    /// (`moshpit_install::capabilities::probe_command`): Install Assist's
    /// re-check asks exactly what a connect asks.
    static let probeCommand = hostCapabilitiesProbeCommand()

    /// What the probe's output says the host has, read by the core
    /// (`HostCapabilities::parse`).
    static func parse(_ output: String) -> HostCapabilities {
        let found = hostCapabilitiesParse(output: output)
        return HostCapabilities(
            hasTmux: found.hasTmux, hasMoshServer: found.hasMoshServer, os: found.os,
            packageManager: found.packageManager.flatMap(PackageManager.init(rawValue:)),
            hasHerdr: found.hasHerdr, hasPython3: found.hasPython3)
    }
}

/// The remote host's package manager, by the raw id the core's Install
/// Assist plan takes (`installAssistPlan(packageManager:)`).
enum PackageManager: String, Codable, Equatable, Sendable, CaseIterable {
    case aptGet     // Debian / Ubuntu
    case dnf        // Fedora / RHEL 8+
    case yum        // CentOS / RHEL 7
    case pacman     // Arch
    case apk        // Alpine
    case brew       // macOS / Homebrew

}

/// Process-wide cache of probed capabilities, keyed by `connection.id`.
///
/// In-memory is enough for the degrade decision (it only matters within a
/// session). We also mirror into `UserDefaults` so a returning user's first
/// connection can show the right banner immediately while a fresh probe runs
/// in the background and corrects the entry.
@MainActor
final class HostCapabilityCache {
    static let shared = HostCapabilityCache()

    private var memory: [UUID: HostCapabilities] = [:]
    private let defaults: UserDefaults
    private let keyPrefix = "hostcaps."

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func capabilities(for id: UUID) -> HostCapabilities? {
        if let cached = memory[id] { return cached }
        guard let data = defaults.data(forKey: keyPrefix + id.uuidString),
              let decoded = try? JSONDecoder().decode(HostCapabilities.self, from: data)
        else { return nil }
        memory[id] = decoded
        return decoded
    }

    /// What `UserDefaults` holds for `id` now, read again: the Rust core
    /// probes and writes the same `hostcaps.<UUID>` behind this cache
    /// (crates/store's `session_memory`), so the copy held in memory may be
    /// older than the core's (B336).
    @discardableResult
    func reload(for id: UUID) -> HostCapabilities? {
        memory[id] = nil
        return capabilities(for: id)
    }

    /// Forget what this process holds for `id` — its connection now points
    /// at another box (B431); the stored copy goes with the store's update.
    func forget(_ id: UUID) {
        memory[id] = nil
    }

    func store(_ caps: HostCapabilities, for id: UUID) {
        memory[id] = caps
        if let data = try? JSONEncoder().encode(caps) {
            defaults.set(data, forKey: keyPrefix + id.uuidString)
        }
    }

    /// Probe over the given SSH session and cache the result. Returns `nil`
    /// when the probe could not run (closed channel, transient hiccup, non-UTF8
    /// output) so the caller can KEEP what it already knew instead of assuming
    /// everything is present. Clobbering a known-degraded host with the
    /// optimistic default would retry `tmux -CC attach` on a tmux-less box and
    /// stall — a `nil` means "no new information", not "all present".
    func probe(over ssh: any HostConnection, for id: UUID) async -> HostCapabilities? {
        guard let output = try? await ssh.executeCommand(HostCapabilities.probeCommand),
              let text = String(data: output, encoding: .utf8)
        else { return nil }
        let caps = HostCapabilities.parse(text)
        store(caps, for: id)
        return caps
    }
}
