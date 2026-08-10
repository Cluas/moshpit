import Foundation
import Testing
@testable import Moshpit

@Suite("HostCapabilities probe parsing")
struct HostCapabilitiesTests {

    // MARK: - Both present

    @Test("Linux host with tmux + mosh-server + apt-get parses fully")
    func fullDebianHost() {
        let output = """
        /usr/bin/tmux
        /usr/bin/mosh-server
        ::Linux::/usr/bin/apt-get
        """
        let caps = HostCapabilities.parse(output)
        #expect(caps.hasTmux)
        #expect(caps.hasMoshServer)
        #expect(caps.os == "Linux")
        #expect(caps.packageManager == .aptGet)
    }

    // MARK: - tmux missing

    @Test("Host missing tmux: only mosh-server path appears before the marker")
    func tmuxMissing() {
        let output = """
        /usr/bin/mosh-server
        ::Linux::/usr/bin/dnf
        """
        let caps = HostCapabilities.parse(output)
        #expect(!caps.hasTmux)
        #expect(caps.hasMoshServer)
        #expect(caps.packageManager == .dnf)
    }

    // MARK: - mosh-server missing

    @Test("Host missing mosh-server: only tmux path appears")
    func moshServerMissing() {
        let output = """
        /usr/bin/tmux
        ::Linux::/usr/bin/pacman
        """
        let caps = HostCapabilities.parse(output)
        #expect(caps.hasTmux)
        #expect(!caps.hasMoshServer)
        #expect(caps.packageManager == .pacman)
    }

    // MARK: - Both missing, no package manager

    @Test("Bare host: no tool paths, empty package-manager field → nil pkg mgr")
    func bareHost() {
        let output = "::Linux::\n"
        let caps = HostCapabilities.parse(output)
        #expect(!caps.hasTmux)
        #expect(!caps.hasMoshServer)
        #expect(caps.os == "Linux")
        #expect(caps.packageManager == nil)
    }

    // MARK: - macOS / brew

    @Test("macOS host with brew parses Darwin + brew")
    func macHost() {
        let output = """
        /opt/homebrew/bin/tmux
        /opt/homebrew/bin/mosh-server
        ::Darwin::/opt/homebrew/bin/brew
        """
        let caps = HostCapabilities.parse(output)
        #expect(caps.os == "Darwin")
        #expect(caps.packageManager == .brew)
        #expect(caps.hasTmux)
        #expect(caps.hasMoshServer)
    }

    // MARK: - Other package managers

    @Test("apk (Alpine) and yum (CentOS) map correctly", arguments: [
        ("/sbin/apk", PackageManager.apk),
        ("/usr/bin/yum", PackageManager.yum),
    ])
    func packageManagerVariants(path: String, expected: PackageManager) {
        let caps = HostCapabilities.parse("::Linux::\(path)\n")
        #expect(caps.packageManager == expected)
    }

    @Test("Unknown package manager binary → nil")
    func unknownPackageManager() {
        let caps = HostCapabilities.parse("::Linux::/usr/bin/zypper\n")
        #expect(caps.packageManager == nil)
    }

    // MARK: - Install command synthesis

    @Test("Install commands include sudo (except brew) and both packages")
    func installCommands() {
        let pkgs = ["tmux", "mosh"]
        #expect(PackageManager.aptGet.installCommand(for: pkgs) == "sudo apt-get install -y tmux mosh")
        #expect(PackageManager.dnf.installCommand(for: pkgs) == "sudo dnf install -y tmux mosh")
        #expect(PackageManager.yum.installCommand(for: pkgs) == "sudo yum install -y tmux mosh")
        #expect(PackageManager.pacman.installCommand(for: pkgs) == "sudo pacman -S --noconfirm tmux mosh")
        #expect(PackageManager.apk.installCommand(for: pkgs) == "sudo apk add tmux mosh")
        #expect(PackageManager.brew.installCommand(for: pkgs) == "brew install tmux mosh")
    }

    // MARK: - Tolerance

    @Test("Noisy / empty output never crashes and degrades to all-missing")
    func noisyOutput() {
        let caps = HostCapabilities.parse("")
        #expect(!caps.hasTmux)
        #expect(!caps.hasMoshServer)
        #expect(caps.packageManager == nil)
    }

    @Test("CRLF line endings parse identically to LF")
    func crlf() {
        let output = "/usr/bin/tmux\r\n/usr/bin/mosh-server\r\n::Linux::/usr/bin/apt-get\r\n"
        let caps = HostCapabilities.parse(output)
        #expect(caps.hasTmux)
        #expect(caps.hasMoshServer)
        #expect(caps.packageManager == .aptGet)
    }

    @Test("`.unknown` default is optimistic (everything present)")
    func unknownIsOptimistic() {
        #expect(HostCapabilities.unknown.hasTmux)
        #expect(HostCapabilities.unknown.hasMoshServer)
        #expect(HostCapabilities.unknown.hasHerdr)
    }

    // MARK: - herdr

    @Test("herdr on PATH is detected alongside the others")
    func herdrPresent() {
        let output = """
        /usr/bin/tmux
        /usr/bin/mosh-server
        /home/cluas/.local/bin/herdr
        ::Linux::/usr/bin/apt-get
        """
        let caps = HostCapabilities.parse(output)
        #expect(caps.hasHerdr)
        #expect(caps.hasTmux)
    }

    @Test("A tmux-only host reads as herdr-missing")
    func herdrMissing() {
        let caps = HostCapabilities.parse("/usr/bin/tmux\n::Linux::/usr/bin/apt-get\n")
        #expect(!caps.hasHerdr)
        #expect(caps.hasTmux)
    }

    @Test("The probe asks for herdr and searches ~/.local/bin (where its installer lands)")
    func probeCommandCoversHerdr() {
        #expect(HostCapabilities.probeCommand.contains("command -v tmux mosh-server herdr"))
        #expect(HostCapabilities.extraPathDirs.contains("$HOME/.local/bin"))
    }

    @Test("has(_:) maps each multiplexer to its own probe result")
    func hasMultiplexer() {
        let herdrOnly = HostCapabilities(
            hasTmux: false, hasMoshServer: false, os: "Linux",
            packageManager: .aptGet, hasHerdr: true)
        #expect(herdrOnly.has(.herdr))
        #expect(!herdrOnly.has(.tmux))
        // Nothing to install → never degraded.
        #expect(herdrOnly.has(.none))
    }

    // MARK: - Cached-probe decoding

    @Test("A cache entry written before hasHerdr existed still decodes")
    func legacyCacheDecodes() throws {
        // Synthesized Codable would THROW on the missing key, and a throw here
        // silently discards the whole cached probe.
        let legacy = Data("""
        {"hasTmux":true,"hasMoshServer":true,"os":"Linux","packageManager":"aptGet"}
        """.utf8)
        let caps = try JSONDecoder().decode(HostCapabilities.self, from: legacy)
        #expect(caps.hasTmux)
        #expect(caps.packageManager == .aptGet)
        // Absent → assume missing. A stale "present" would boot herdr on a
        // host without it; a stale "missing" only costs one banner.
        #expect(!caps.hasHerdr)
    }

    @Test("Capabilities round-trip through the cache encoding")
    func capsRoundTrip() throws {
        let caps = HostCapabilities(
            hasTmux: false, hasMoshServer: true, os: "Darwin",
            packageManager: .brew, hasHerdr: true)
        let back = try JSONDecoder().decode(
            HostCapabilities.self, from: JSONEncoder().encode(caps))
        #expect(back == caps)
    }

    // MARK: - Multiplexer install commands

    @Test("herdr never routes through a distro package manager")
    func herdrInstallCommand() {
        // No distro ships herdr — `apt-get install herdr` would just fail.
        #expect(Multiplexer.herdr.installCommand(using: .aptGet)
                == "curl -fsSL https://herdr.dev/install.sh | sh")
        #expect(Multiplexer.herdr.installCommand(using: .pacman)
                == "curl -fsSL https://herdr.dev/install.sh | sh")
        // The installer needs no package manager, so there's always an offer.
        #expect(Multiplexer.herdr.installCommand(using: nil)
                == "curl -fsSL https://herdr.dev/install.sh | sh")
        // Homebrew does have a formula.
        #expect(Multiplexer.herdr.installCommand(using: .brew) == "brew install herdr")
    }

    @Test("tmux installs mosh alongside it; none offers nothing")
    func tmuxInstallCommand() {
        #expect(Multiplexer.tmux.installCommand(using: .aptGet)
                == "sudo apt-get install -y tmux mosh")
        #expect(Multiplexer.tmux.installCommand(using: nil) == nil)
        #expect(Multiplexer.none.installCommand(using: .brew) == nil)
    }

    // MARK: - Launch line

    @Test("herdr boots with the same PATH prefix the probe searched")
    func herdrLaunchUsesProbePath() {
        let command = HerdrLaunch.attachCommand(customPath: nil)
        // Otherwise the very binary the probe just found is "command not
        // found" in a login shell — herdr's installer touches no rc file.
        #expect(command.contains("$HOME/.local/bin"))
        #expect(command.hasSuffix(" herdr"))
    }

    @Test("A custom herdr path is trusted verbatim, empty is treated as unset")
    func herdrLaunchCustomPath() {
        #expect(HerdrLaunch.attachCommand(customPath: "/opt/herdr/bin/herdr")
                == "/opt/herdr/bin/herdr")
        // The form stores "" for an untouched field; that must not become the
        // command.
        #expect(HerdrLaunch.attachCommand(customPath: "").contains("herdr"))
        #expect(HerdrLaunch.attachCommand(customPath: "") != "")
    }
}
