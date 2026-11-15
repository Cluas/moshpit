import Foundation
import MoshpitKit

/// The keychain items the app's records point at — each connection's
/// password or pasted key, each SSH key's private half — and what becomes of
/// the ones nothing points at any more (B232).
///
/// Deleting a key used to remove its record and nothing else: the private
/// key stayed in the keychain, under a ref no record named any more, so
/// nothing in the app could ever reach it again — a Secure Enclave key's
/// handle as much as an imported key's file (a Secure Enclave key lives
/// nowhere else: its `dataRepresentation` in that one item IS the key, so
/// deleting the item deletes the key). Deleting a connection that signed in
/// with such a key left the item too. Now an item goes when the last record
/// naming it does — counted across every saved connection and every key
/// record, the ones this build carries without decoding included.
///
/// The items earlier builds left behind are NOT deleted: a record can have
/// been lost without the user deleting anything (a key list read as empty
/// and saved over, B8 / B32), and then the orphan is the only copy of their
/// key. Once a launch they become key records again
/// (``recoverOrphansOnce``), listed in SSH Keys like any key — and deleting
/// one there deletes its secret, by the rule above.
///
/// Nothing here guesses. A store whose read can't be trusted (a launch
/// before the first unlock), or one holding an entry whose ref can't be
/// read, makes the answer "unknown", and unknown changes nothing.
@MainActor
enum KeychainReferences {

    /// Every ref a saved connection or a stored key record holds; nil when
    /// either store can't say (``ConnectionStore/keychainRefs(excluding:)``,
    /// ``SSHKeyStore/keychainRefs(excluding:)``). `excludingConnection` and
    /// `excludingKey` leave out the record about to go.
    static func referenced(connections: ConnectionStore, keys: SSHKeyStore,
                           excludingConnection: UUID? = nil,
                           excludingKey: UUID? = nil) -> Set<String>? {
        guard let held = connections.keychainRefs(excluding: excludingConnection),
              let keyRefs = keys.keychainRefs(excluding: excludingKey) else { return nil }
        return held.union(keyRefs)
    }

    /// Drop `ref` once no saved connection and no key record names it: a
    /// connection's own item it left for a managed key (B325), which used to
    /// stay named by nothing — a password for good, a pasted key listed
    /// again as a "Recovered key" at the next launch. One that can't go now
    /// (unknown, or the keychain kept it) stays on the store's list of
    /// items to drop, and a later launch drops it (B428). Named again, it
    /// stays.
    @discardableResult
    static func release(_ ref: String, connections: ConnectionStore, keys: SSHKeyStore,
                        keychain: KeychainService,
                        preferences: PreferencesHost = AppStores.preferences) async -> StoreSecretFate {
        let held = referenced(connections: connections, keys: keys)
        let fate = storeRelease(preferences: preferences, mode: .system, account: ref,
                                referenced: held.map(Array.init), keychain: keychain.storeHost)
        Log.store.info("keychain: an item a connection left: \(String(describing: fate), privacy: .public)")
        return fate
    }

    /// Put `ref` on the store's list of items to drop (B428): a connection
    /// let go of it, but it can't go yet — its session still signs in with
    /// it (B325), or the drop failed. ``release(_:connections:keys:keychain:preferences:)``
    /// drops it once the session lets go; if the app ends first, the next
    /// launch does (``settleReleasesOnce``). Never offered back as a key.
    static func hold(_ ref: String, preferences: PreferencesHost = AppStores.preferences) {
        if !storeHoldRelease(preferences: preferences, mode: .system, account: ref) {
            Log.store.error("keychain: an item a connection left couldn't be listed to drop later")
        }
    }

    /// Whether this launch has settled the list (``settleReleasesOnce``).
    private static var settled = false

    /// Drop what earlier launches listed (B428): once a launch, the first
    /// time both stores can be trusted, before recovery looks. An item no
    /// saved record names goes; one named again stays; one the keychain
    /// keeps is tried at the next launch. What this launch listed waits for
    /// its session. Returns what became of each.
    @discardableResult
    static func settleReleasesOnce(connections: ConnectionStore, keys: SSHKeyStore,
                                   keychain: KeychainService,
                                   preferences: PreferencesHost = AppStores.preferences)
        -> [StoreSettledRelease] {
        guard !settled, let held = referenced(connections: connections, keys: keys) else { return [] }
        settled = true
        let done = storeSettleReleases(preferences: preferences, mode: .system,
                                       referenced: Array(held), keychain: keychain.storeHost)
        if !done.isEmpty {
            Log.store.notice("keychain: \(done.count, privacy: .public) item(s) connections let go of settled")
        }
        return done
    }

    // MARK: - Orphans

    /// Whether this launch has looked (``recoverOrphansOnce``).
    private static var looked = false

    /// Give every private key earlier builds orphaned a key record again:
    /// once a launch, the first time both stores can be trusted. Returns
    /// the records added.
    ///
    /// Only an item that is unmistakably one of ours and that nothing names
    /// is looked at: its account is a ref ``KeychainService/generateRef()``
    /// makes (`moshpit.secret.…`) — never the trusted host keys, never
    /// anything else in the keychain — no saved connection and no key record
    /// holds it, and it wasn't made in this launch (whose record may still be
    /// on its way: an item is written before the record naming it). It is
    /// read without asking (an item behind Face ID is left for now), and
    /// becomes a record only if it reads as a private key
    /// (``SSHKeyFactory/describeStored(_:)``): "Recovered key" and its
    /// fingerprint's start, its type and public key from the key itself.
    /// Anything else — a deleted connection's password, a key behind a
    /// passphrase — is left where it is and logged. Nothing is ever deleted
    /// or rewritten here, and a recovered item is named from then on, so a
    /// later launch adds nothing for it.
    @discardableResult
    static func recoverOrphansOnce(connections: ConnectionStore, keys: SSHKeyStore,
                                   keychain: KeychainService,
                                   preferences: PreferencesHost = AppStores.preferences) async -> [SSHKeyRecord] {
        guard !looked else { return [] }
        // What a connection let go of is to be dropped, not offered back
        // as a key (B428).
        guard referenced(connections: connections, keys: keys) != nil,
              let listed = storeToRelease(preferences: preferences, mode: .system).map(Set.init) else {
            Log.store.info("keychain recovery: which items are named can't be told yet — not looking")
            return []
        }
        looked = true
        let accounts: [String]
        do {
            accounts = try await keychain.accounts()
        } catch {
            // Locked again, or a keychain that can't list: the next time.
            looked = false
            Log.store.info("keychain recovery: couldn't list the items (\(String(describing: error), privacy: .public))")
            return []
        }
        var recovered: [SSHKeyRecord] = []
        for account in accounts.sorted() where isOurs(account) && !listed.contains(account) {
            guard let held = referenced(connections: connections, keys: keys),
                  !held.contains(account) else { continue }
            let secret: Data
            do {
                secret = try await keychain.loadSecretSilently(forRef: account)
            } catch {
                Log.store.info("keychain recovery: an orphaned item can't be read without asking; left as it is (\(String(describing: error), privacy: .public))")
                continue
            }
            guard let key = SSHKeyFactory.describeStored(secret) else {
                Log.store.info("keychain recovery: an orphaned item isn't a private key (a password, or a key behind a passphrase); left as it is")
                continue
            }
            // Asked again now: the stores may have changed while the
            // keychain answered.
            guard let now = referenced(connections: connections, keys: keys), !now.contains(account) else { continue }
            let record = recoveredRecord(key, account: account)
            keys.add(record)
            recovered.append(record)
        }
        if !recovered.isEmpty {
            Log.store.notice("keychain recovery: \(recovered.count, privacy: .public) orphaned private key(s) listed in SSH Keys again")
        }
        return recovered
    }

    /// The record an orphaned private key gets: "Recovered key" and the
    /// start of its fingerprint, so two of them can be told apart; its type,
    /// public key and size from the key itself. Read without asking, it
    /// never asked for Face ID; where it came from can't be told, so it is
    /// listed with the imported keys unless it is a Secure Enclave key.
    static func recoveredRecord(_ key: SSHKeyFactory.Generated, account: String) -> SSHKeyRecord {
        let hash = key.fingerprint.hasPrefix("SHA256:") ? String(key.fingerprint.dropFirst(7)) : key.fingerprint
        let short = String(hash.prefix(8))
        return SSHKeyRecord(
            name: String(localized: "Recovered key \(short)"),
            algorithm: key.algorithm,
            bits: key.bits,
            fingerprint: key.fingerprint,
            publicKey: key.publicKeyLine,
            source: key.algorithm == .seP256 ? .secureEnclave : .imported,
            keychainRef: account,
            requireBiometry: false)
    }

    /// Whether `account` is an item recovery may look at at all: a ref the
    /// app made for a connection or key, and not in this launch.
    static func isOurs(_ account: String) -> Bool {
        account.hasPrefix(KeychainService.refPrefix) && !KeychainService.mintedThisLaunch(account)
    }

    #if DEBUG
    /// Tests look, and settle, more than once a process.
    static func resetForTesting() {
        looked = false
        settled = false
    }
    #endif
}

// MARK: - Deleting a key

/// Deleting an SSH key: its private key out of the keychain, then its record
/// — unless a saved connection still signs in with it, in which case
/// nothing is deleted and the user hears which connections (B232).
///
/// It used to be the record alone, and "connections already using it keep
/// working" was the reason given: the private key stayed behind for good,
/// whether anything used it or not.
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
    /// again first if their last read can't be trusted).
    static func check(_ key: SSHKeyRecord, connections: ConnectionStore, keys: SSHKeyStore) -> Check {
        connections.reloadIfNeeded()
        keys.reloadIfNeeded()
        guard keys.keychainRefs() != nil,
              let users = connections.names(usingKey: key.id, ref: key.keychainRef) else { return .unknown }
        return users.isEmpty ? .free : .inUse(users)
    }

    /// Delete `key`: its private key first (`deleteItem`, the keychain's
    /// delete), then its record. A key some connection uses, or one whose
    /// users can't be told, is left as it is; so is one whose private key
    /// the keychain won't delete — the record is the only way back to it.
    /// The item stays, the record going, only while another key record
    /// names the same item.
    static func delete(_ key: SSHKeyRecord, from keys: SSHKeyStore, connections: ConnectionStore,
                       deleteItem: (String) async throws -> Void) async throws {
        switch check(key, connections: connections, keys: keys) {
        case .inUse(let names): throw Failure.inUse(names)
        case .unknown: throw Failure.unknown
        case .free: break
        }
        if let ref = key.keychainRef {
            guard let held = KeychainReferences.referenced(connections: connections, keys: keys,
                                                           excludingKey: key.id) else {
                throw Failure.unknown
            }
            if !held.contains(ref) {
                do {
                    try await deleteItem(ref)
                } catch {
                    Log.store.error("key delete: the keychain kept the private key: \(String(describing: error), privacy: .public)")
                    throw Failure.keychain(String(describing: error))
                }
            }
        }
        keys.remove(id: key.id)
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
