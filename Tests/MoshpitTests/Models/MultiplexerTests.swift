import Foundation
import Testing
@testable import Moshpit

/// The multiplexer choice and — the part that can silently ruin an upgrade —
/// how connections saved BEFORE the picker existed decode.
@Suite("Multiplexer selection + legacy migration")
struct MultiplexerTests {

    /// Every non-optional stored field of `ServerConnection` as an older build
    /// would have written it: no `multiplexerRaw`, no `herdrPath`.
    private func legacyJSON(useTmux: Bool) -> Data {
        Data("""
        {
          "id": "8F1C0F4E-3B1A-4E7D-9B2A-2C7E5D6A1B33",
          "name": "legacy",
          "host": "example.com",
          "port": 22,
          "username": "dev",
          "authMethod": "password",
          "connectionProtocol": "ssh",
          "sshPort": 22,
          "moshPortRangeStart": 60001,
          "moshPortRangeEnd": 60999,
          "createdAt": 760000000,
          "forwardAgent": false,
          "compression": false,
          "keepAliveInterval": 60,
          "useTailscale": false,
          "tmuxAutoAttach": true,
          "tmuxCreateNew": true,
          "useTmux": \(useTmux)
        }
        """.utf8)
    }

    // MARK: - Migration

    @Test("An explicit multiplexerRaw wins over the legacy flag")
    func explicitRawWins() throws {
        var c = try JSONDecoder().decode(ServerConnection.self, from: legacyJSON(useTmux: true))
        c.multiplexerRaw = Multiplexer.herdr.rawValue
        #expect(c.multiplexer == .herdr)
    }

    @Test("An unrecognized stored value falls back instead of trapping")
    func unknownRawFallsBack() throws {
        var c = try JSONDecoder().decode(ServerConnection.self, from: legacyJSON(useTmux: true))
        c.multiplexerRaw = "zellij"     // e.g. written by a newer build
        #expect(c.multiplexer == .tmux) // falls back to the legacy flag
    }

    // MARK: - Round trip

    @Test("Setting the multiplexer keeps the legacy flag in sync",
          arguments: [Multiplexer.none, .tmux, .herdr])
    func setterSyncsLegacyFlag(_ mux: Multiplexer) {
        var c = ServerConnection()
        c.multiplexer = mux
        #expect(c.multiplexer == mux)
        #expect(c.useTmux == (mux == .tmux))
        #expect(c.multiplexerRaw == mux.rawValue)
    }

}
