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
    /// python3 on PATH — gates the herdr push pump (`HerdrPushBoot`), which
    /// bridges to herdr's socket through a python one-liner. Missing just
    /// means the control plane stays on polling; nothing degrades visibly.
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

    /// Hand-written so a cache entry written by an older build — which had no
    /// `hasHerdr` key — still decodes instead of throwing. Synthesized
    /// `Codable` does NOT fall back to a property's default value on a missing
    /// key, and a throw here would silently discard the whole cached probe.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        hasTmux = try c.decodeIfPresent(Bool.self, forKey: .hasTmux) ?? false
        hasMoshServer = try c.decodeIfPresent(Bool.self, forKey: .hasMoshServer) ?? false
        os = try c.decodeIfPresent(String.self, forKey: .os) ?? ""
        packageManager = try c.decodeIfPresent(PackageManager.self, forKey: .packageManager)
        // Absent (old cache) → false. A stale "missing" only costs one banner
        // that the in-session re-probe clears; a stale "present" would boot
        // herdr on a host without it.
        hasHerdr = try c.decodeIfPresent(Bool.self, forKey: .hasHerdr) ?? false
        // Same fallback, cheaper stakes: a stale "missing" just delays push
        // mode until the re-probe lands; polling covers the gap.
        hasPython3 = try c.decodeIfPresent(Bool.self, forKey: .hasPython3) ?? false
    }

    /// "Everything present" — the optimistic default used before the first
    /// probe completes, so a cold session behaves exactly as it does today and
    /// only degrades if the probe actually finds something missing.
    static let unknown = HostCapabilities(
        hasTmux: true, hasMoshServer: true, os: "", packageManager: nil, hasHerdr: true,
        hasPython3: true)

    /// Whether the host can run the multiplexer this connection picked.
    /// `.none` needs nothing, so it is always satisfied.
    func has(_ multiplexer: Multiplexer) -> Bool {
        switch multiplexer {
        case .none:  return true
        case .tmux:  return hasTmux
        case .herdr: return hasHerdr
        }
    }

    /// Extra PATH entries to check alongside whatever the SSH channel already
    /// inherited. `ssh host command` typically execs a NON-login,
    /// non-interactive shell — it does not source `.zprofile`/`.bash_profile`,
    /// so PATH additions those files make (most commonly Homebrew's `brew
    /// shellenv`, at `/opt/homebrew` on Apple Silicon or `/usr/local` on Intel
    /// Mac and many Linux self-installs) are invisible here even though the
    /// SAME binary works fine from an interactive login shell. Without this,
    /// a real tmux install can probe as "missing" and silently degrade a
    /// session to plain SSH for no reason the user can see.
    /// Shared with `MoshBootstrap`: the bootstrap's `mosh-server new` runs over
    /// the same non-interactive exec channel, so it must search the same dirs
    /// the probe did — otherwise a host can pass the probe and still fail the
    /// launch with "command not found" (seen live on a macOS/Homebrew host).
    /// `$HOME/.local/bin` is here for herdr specifically: its official
    /// installer (`curl -fsSL https://herdr.dev/install.sh | sh`) drops the
    /// binary there and, by design, does NOT touch any shell rc file — it only
    /// prints a "not in your PATH" warning. Without this entry the very
    /// install command Install Assist hands the user produces a herdr that
    /// still probes as missing. `$HOME` expands remotely: the dirs are
    /// interpolated inside a double-quoted `PATH="$PATH:…"` assignment.
    static let extraPathDirs =
        "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin"

    /// The single shell command whose stdout ``parse(_:)`` consumes. Kept here
    /// (not inlined in the hub) so the unit test exercises the exact string the
    /// app runs.
    ///
    /// `command -v` prints a path per found name and exits non-zero if any are
    /// missing — harmless, we only read stdout. The trailing marker line is
    /// unambiguous (`::os::pkgPath`) so parsing never confuses it with a tool
    /// path. `2>/dev/null` keeps a noisy shell from polluting the marker.
    static let probeCommand =
        "PATH=\"$PATH:\(extraPathDirs)\" command -v tmux mosh-server herdr python3; " +
        "echo \"::$(uname -s)::$(PATH=\"$PATH:\(extraPathDirs)\" command -v apt-get dnf yum pacman apk brew 2>/dev/null | head -1)\""

    /// Parse the probe stdout. Lines before the `::os::pkg` marker are the
    /// resolved paths of whatever `command -v tmux mosh-server` found; a line
    /// ending in `/tmux` (or just `tmux`) means tmux is on PATH, same for
    /// `mosh-server`. The marker line carries `uname -s` and the package
    /// manager's path.
    static func parse(_ output: String) -> HostCapabilities {
        var hasTmux = false
        var hasMoshServer = false
        var hasHerdr = false
        var hasPython3 = false
        var os = ""
        var packageManager: PackageManager?

        for rawLine in output.split(whereSeparator: \.isNewline) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("::") {
                // Marker: "::<uname>::<pkgPath>"
                let fields = line.split(separator: ":", omittingEmptySubsequences: false)
                // ["", "", "<uname>", "", "<pkgPath>"] — `::` yields two empty
                // leading fields; the body fields land at index 2 and 4.
                if fields.count > 2 {
                    os = String(fields[2]).trimmingCharacters(in: .whitespaces)
                }
                if fields.count > 4 {
                    let pkgPath = String(fields[4]).trimmingCharacters(in: .whitespaces)
                    packageManager = PackageManager(binaryPath: pkgPath)
                }
                continue
            }
            // A resolved tool path (or a bare name from a shell builtin form).
            let name = line.split(separator: "/").last.map(String.init) ?? line
            if name == "tmux" { hasTmux = true }
            if name == "mosh-server" { hasMoshServer = true }
            if name == "herdr" { hasHerdr = true }
            if name == "python3" { hasPython3 = true }
        }

        return HostCapabilities(
            hasTmux: hasTmux, hasMoshServer: hasMoshServer,
            os: os, packageManager: packageManager, hasHerdr: hasHerdr,
            hasPython3: hasPython3)
    }
}

/// The remote host's package manager, used to synthesize the right install
/// command in the Install Assist sheet.
enum PackageManager: String, Codable, Equatable, Sendable, CaseIterable {
    case aptGet     // Debian / Ubuntu
    case dnf        // Fedora / RHEL 8+
    case yum        // CentOS / RHEL 7
    case pacman     // Arch
    case apk        // Alpine
    case brew       // macOS / Homebrew

    /// Match the last path component of a `command -v` hit (e.g.
    /// `/usr/bin/apt-get`) to a case. Unknown → nil.
    init?(binaryPath: String) {
        let name = binaryPath.split(separator: "/").last.map(String.init) ?? binaryPath
        switch name {
        case "apt-get": self = .aptGet
        case "dnf": self = .dnf
        case "yum": self = .yum
        case "pacman": self = .pacman
        case "apk": self = .apk
        case "brew": self = .brew
        default: return nil
        }
    }

    /// Install command for the given package names, e.g.
    /// `sudo apt-get install -y tmux mosh`. brew runs unprivileged.
    func installCommand(for packages: [String]) -> String {
        let pkgs = packages.joined(separator: " ")
        switch self {
        case .aptGet: return "sudo apt-get install -y \(pkgs)"
        case .dnf:    return "sudo dnf install -y \(pkgs)"
        case .yum:    return "sudo yum install -y \(pkgs)"
        case .pacman: return "sudo pacman -S --noconfirm \(pkgs)"
        case .apk:    return "sudo apk add \(pkgs)"
        case .brew:   return "brew install \(pkgs)"
        }
    }
}

extension Multiplexer {
    /// The install command Install Assist offers for this multiplexer, given
    /// the host's detected package manager. `nil` when there is nothing
    /// useful to hand the user.
    func installCommand(using manager: PackageManager?) -> String? {
        switch self {
        case .none:
            return nil
        case .tmux:
            // Install mosh alongside: a host missing tmux is usually missing
            // mosh too, and one command beats two round trips.
            return manager?.installCommand(for: ["tmux", "mosh"])
        case .herdr:
            // herdr is in NO distro repository — only Homebrew and its own
            // installer script. Routing it through `PackageManager` would
            // synthesize `sudo apt-get install -y herdr`, which fails with
            // "unable to locate package" and reads as our bug. The installer
            // script needs no package manager at all, so unlike tmux there is
            // always something to offer.
            if manager == .brew { return "brew install herdr" }
            return "curl -fsSL https://herdr.dev/install.sh | sh"
        }
    }
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
    func probe(over ssh: SSHSession, for id: UUID) async -> HostCapabilities? {
        guard let output = try? await ssh.executeCommand(HostCapabilities.probeCommand),
              let text = String(data: output, encoding: .utf8)
        else { return nil }
        let caps = HostCapabilities.parse(text)
        store(caps, for: id)
        return caps
    }
}
