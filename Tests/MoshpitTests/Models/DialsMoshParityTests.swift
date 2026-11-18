import Foundation
import Testing
@testable import Moshpit

/// `ServerConnection.dialsMosh` is the core's own rule, asked through the
/// FFI (`coreDialsMosh`, crates/ffi), never a copy of it. This is the same
/// table as `DIALS_MOSH` in crates/ffi/src/session.rs, which the Rust side
/// asserts against `ConnectionSpec::dialled_protocol`: change both together.
@Suite("dialsMosh follows the core's rule")
struct DialsMoshParityTests {
    /// (saved as mosh, through a jump host, route) → dials mosh.
    static let table: [(mosh: Bool, through: Bool, route: String?, want: Bool)] = [
        (false, false, nil, false),
        (true, false, nil, true),
        (true, false, "direct", true),
        (true, true, nil, false),
        (true, true, "ssh", false),
        (true, true, "direct", true),
        (true, true, "viaJump", true),
        (true, true, "auto", false),
        (false, true, "direct", false),
        (false, true, "viaJump", false),
    ]

    @Test("every row of the shared table")
    func table_rows() {
        for row in Self.table {
            var connection = ServerConnection(
                host: "box.local", username: "me", connectionProtocol: row.mosh ? .mosh : .ssh)
            connection.viaConnectionId = row.through ? UUID() : nil
            connection.jumpMoshRoute = row.route
            #expect(connection.dialsMosh == row.want, "\(row)")
            #expect(coreDialsMosh(mosh: row.mosh, throughJump: row.through, jumpMoshRoute: row.route) == row.want)
        }
    }

    @Test("a record round trip keeps the route, and a missing one reads as ssh")
    @MainActor func record_round_trip() {
        var connection = ServerConnection(host: "box.local", username: "me", connectionProtocol: .mosh)
        #expect(connection.jumpRoute == .ssh)
        connection.jumpMoshRoute = "viaJump"
        let back = ServerConnection(ConnectionRecord(connection))
        #expect(back?.jumpMoshRoute == "viaJump")
        #expect(back?.jumpRoute == .viaJump)
        let spec = RustSessionDriver.spec(for: connection, grid: (80, 24))
        #expect(spec.jumpMoshRoute == "viaJump")
    }

    @Test("an old saved connection without the field decodes as ssh")
    func old_json() throws {
        var connection = ServerConnection(host: "box.local", username: "me", connectionProtocol: .mosh)
        connection.viaConnectionId = UUID()
        let data = try JSONEncoder().encode(connection)
        var json = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        json.removeValue(forKey: "jumpMoshRoute")
        let old = try JSONDecoder().decode(
            ServerConnection.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(old.jumpRoute == .ssh)
        #expect(!old.dialsMosh)
    }
}
