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
        /home/dev/.local/bin/herdr
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

    // MARK: - Cached-probe decoding

    @Test("Capabilities round-trip through the cache encoding")
    func capsRoundTrip() throws {
        let caps = HostCapabilities(
            hasTmux: false, hasMoshServer: true, os: "Darwin",
            packageManager: .brew, hasHerdr: true)
        let back = try JSONDecoder().decode(
            HostCapabilities.self, from: JSONEncoder().encode(caps))
        #expect(back == caps)
    }
}
