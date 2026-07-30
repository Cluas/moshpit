import Foundation
import Testing
@testable import Ringdown

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
    }
}
