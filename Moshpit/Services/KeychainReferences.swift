import Foundation
import MoshpitKit

/// The launch steps over the keychain items the app's records point at that
/// only ask the core — never a copy of its rules.
///
/// B810: this used to hold a Swift copy of `crates/store/src/secrets.rs` —
/// which items the records name, orphan recovery, what a connection lets go
/// of — and the app's own set of the refs made this launch, beside the
/// core's. The two disagreed depending on who made an item. Those are the
/// core's now, through the stores (``ConnectionStore/releaseSecret(_:keys:keychain:)``,
/// ``ConnectionStore/settleReleasesOnce(keys:keychain:)``,
/// ``SSHKeyStore/recoverOrphansOnce(connections:keychain:reader:)``,
/// ``SSHKeyStore/delete(id:connections:keychain:)``) and
/// `storeNewKeychainRef()`.
@MainActor
enum KeychainReferences {

    /// What this phone's keychain doesn't hold of what the records name
    /// (B781, B782; `store_missing_items`): a phone restored from a backup
    /// has every connection and key record and none of their items, all
    /// `ThisDeviceOnly`. Asked without reading an item, and again at each
    /// return to the foreground — an item imported again since is there;
    /// nothing is judged while either store can't be trusted. The keys go
    /// to their store, which groups them apart in SSH Keys and keeps them
    /// out of the picker.
    static func findMissing(connections: ConnectionStore, keys: SSHKeyStore, keychain: KeychainService) {
        guard let connectionsHandle = connections.rustHandle, let keysHandle = keys.rustHandle,
              let missing = storeMissingItems(connections: connectionsHandle, keys: keysHandle,
                                              keychain: keychain.storeHost)
        else { return }
        let missingKeys = Set(missing.keys.compactMap(UUID.init(uuidString:)))
        if !missingKeys.isEmpty {
            Log.store.notice("keychain: \(missingKeys.count, privacy: .public) key(s) whose private half isn't on this device")
        }
        keys.setMissing(missingKeys)
        // B782: the connections' own passwords and pasted keys. Home's
        // Connect opens their Edit, its field asking for the secret.
        let missingSecrets = Set(missing.connections.compactMap(UUID.init(uuidString:)))
        if !missingSecrets.isEmpty {
            Log.store.notice("keychain: \(missingSecrets.count, privacy: .public) connection(s) whose password or key isn't on this device")
        }
        connections.setSecretMissing(missingSecrets)
    }

    /// Whether this launch has moved the trusted host keys
    /// (``moveTrustedHostKeysOnce(keychain:)``).
    private static var movedHostKeys = false

    /// The trusted host keys onto the class that moves with a backup (B783),
    /// once a launch: a phone restored from an encrypted backup or set up by
    /// Quick Start keeps them, not a fresh trust on first use for every host.
    /// A keychain that refuses (locked) is tried again the next launch.
    static func moveTrustedHostKeysOnce(keychain: KeychainService) async {
        guard !movedHostKeys else { return }
        do {
            try await keychain.moveTrustedHostKeysWithBackups()
            movedHostKeys = true
        } catch {
            Log.store.info("trusted host keys: not moved yet (\(String(describing: error), privacy: .public)) — next launch")
        }
    }

    #if DEBUG
    /// Tests move the host keys more than once a process.
    static func resetForTesting() {
        movedHostKeys = false
    }
    #endif
}
