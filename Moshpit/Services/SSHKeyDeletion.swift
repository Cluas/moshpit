import Foundation
import MoshpitKit

/// Deleting an SSH key: its private key out of the keychain, then its record
/// — unless a saved connection still signs in with it, in which case
/// nothing is deleted and the user hears which connections (B232).
///
/// It used to be the record alone, and "connections already using it keep
/// working" was the reason given: the private key stayed behind for good,
/// whether anything used it or not.
///
/// The rule is the core's (`secrets::check_key`, `secrets::delete_key`,
/// through ``SSHKeyStore/checkDelete(id:connections:)`` and
/// ``SSHKeyStore/delete(id:connections:keychain:)``); B810 took out the Swift
/// copy of it. What is left here is what the pages show.
@MainActor
enum SSHKeyDeletion {

    /// What deleting a key would do now.
    enum Check: Equatable {
        /// Nothing uses it: the record and its private key can go.
        case free
        /// These saved connections still sign in with it: nothing goes.
        case inUse([String])
        /// Which connections use it can't be told now: nothing goes.
        case unknown
    }

    /// Why a key wasn't deleted.
    enum Failure: Error, Equatable {
        case inUse([String])
        case unknown
        /// The keychain wouldn't delete the private key; the record stays,
        /// the one handle left on it.
        case keychain(String)
    }

    /// What deleting `key` would do, from the stores as they are now (read
    /// again first if their last read can't be trusted). A key already gone
    /// is free: there is nothing left to keep.
    static func check(_ key: SSHKeyRecord, connections: ConnectionStore, keys: SSHKeyStore) -> Check {
        switch keys.checkDelete(id: key.id, connections: connections) {
        case .free?, .notFound?: .free
        case .inUse(let names)?: .inUse(names)
        case .unknown?, nil: .unknown
        }
    }

    /// Delete `key`: its private key first, then its record. A key some
    /// connection uses, or one whose users can't be told, is left as it is;
    /// so is one whose private key the keychain won't delete — the record is
    /// the only way back to it. The item stays, the record going, only while
    /// another key record names the same item. A key already gone is done.
    static func delete(_ key: SSHKeyRecord, from keys: SSHKeyStore, connections: ConnectionStore,
                       keychain: KeychainHost) throws {
        switch keys.delete(id: key.id, connections: connections, keychain: keychain) {
        case .deleted?, .notFound?:
            return
        case .inUse(let names)?:
            throw Failure.inUse(names)
        case .unknown?, nil:
            throw Failure.unknown
        case .keychainFailed(let message)?:
            Log.store.error("key delete: the keychain kept the private key: \(message, privacy: .public)")
            throw Failure.keychain(message)
        }
    }
}

extension SSHKeyDeletion.Failure {
    /// The notice's title.
    var title: String {
        switch self {
        case .inUse: String(localized: "Key in use")
        case .unknown, .keychain: String(localized: "Couldn't delete the key")
        }
    }

    /// The notice's message.
    var message: String {
        switch self {
        case .inUse(let names):
            String(localized: "Connections using it: \(ListFormatter.localizedString(byJoining: names)). Choose another key for them first, then delete this one.")
        case .unknown:
            String(localized: "Moshpit can't tell yet which connections use this key, so it was kept. Try again in a moment.")
        case .keychain:
            String(localized: "Its private key couldn't be removed from the keychain, so the key was kept. The diagnostics log has the reason.")
        }
    }
}
