import Foundation

/// What a delete's confirmation says: what goes, and — when other
/// connections go through the one deleted (F1) — which, and that they won't
/// connect until another way is picked. They are never silently switched
/// to Direct; their cards say "Jump host deleted" until they are edited.
enum ConnectionDeleteWords {
    static func dependents(of connection: ServerConnection, in saved: [ServerConnection]) -> [String] {
        saved.filter { $0.viaConnectionId == connection.id }.map(\.displayName)
    }

    static func message(for connection: ServerConnection, in saved: [ServerConnection]) -> String {
        let removes = String(localized: "Removes the saved server and its stored credentials from this device.")
        let through = dependents(of: connection, in: saved)
        guard !through.isEmpty else { return removes }
        let names = ListFormatter.localizedString(byJoining: through)
        let fail = through.count == 1
            ? String(localized: "\(names) connects through this host. It won't connect until you pick another way.")
            : String(localized: "\(names) connect through this host. They won't connect until you pick another way.")
        return "\(removes) \(fail)"
    }
}
