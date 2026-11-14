import Foundation
import MoshpitKit

/// Deleting a saved connection, and everything this phone keeps for it — its
/// keychain item too, once no other connection and no SSH key names it
/// (``KeychainReferences``).
///
/// The push pairing is what used to stay behind. Deleting a connection ended
/// its session, dropped its keychain secret and its entry — and left its
/// pairing in the App Group file: the end-to-end secret still on the phone,
/// and a send token the phone went on asking the relay to re-mint, for a
/// connection that no longer existed. The confirmation had promised "Removes
/// the saved server and its stored credentials".
///
/// The pairing goes FIRST, in the order unpairing already keeps (Host
/// Setup's unpair): forgetting it is the one step here that can
/// fail — the pairings file won't write — and every step after it can't be
/// taken back. A phone that can't let go stops the delete before anything is
/// removed, and the caller says so; the connection stays, the one handle left
/// on a pairing that would otherwise belong to nothing on screen. The host
/// isn't reached: its conf keeps a send token until the relay's TTL runs out,
/// and without the secret on the phone it can only ever ring the generic line.
///
/// Host care hears of the delete before the pairing goes. A care pass for the
/// connection — one starts with every session — may be out reading the host,
/// and it used to come back during the seconds the session takes to end, find
/// no pairing (this delete had just forgotten it, or there never was one) and
/// make one: minted at the relay and saved on the phone, for a connection gone
/// a moment later. Now care pairs nothing for a connection being deleted, the
/// pass in flight is cancelled, and the delete waits it out before the
/// connection goes. Care and the pairing are the core's (``HostCare``,
/// `crates/hostcare`); this keeps the delete's order and the keychain.
@MainActor
enum ConnectionDeletion {

    /// The connections a delete is under way for: from care hearing of it to
    /// the record going, which can take seconds (the host asked to drop its
    /// pairing, up to three, B377; a care pass waited out, up to five). Their cards are still on Home all that time, and a tap
    /// on one used to start a new session for a connection about to go —
    /// left running, unseen, after its card and its keychain item had gone
    /// (B327). Nothing starts a session for one of these.
    private(set) static var underWay: Set<UUID> = []

    /// Whether a delete is under way for connection `id`.
    static func isUnderWay(_ id: UUID) -> Bool { underWay.contains(id) }

    /// Delete `connection` from `store` once `care` has forgotten its pairing.
    ///
    /// Throws ``PushPairingForgetFailure`` when the phone couldn't let go of
    /// the pairing, and then nothing else has happened: no teardown, no
    /// secret dropped, the connection still saved, and care free to tend it
    /// again.
    /// `tearDown` ends what is live for it (the island's tracking, the
    /// session); `dropSecret` removes the keychain item the connection
    /// signed in with, once nothing else names it (`keys`: the key records,
    /// which name theirs); `dropShares` removes what the share extension
    /// queued for it, photos included (B343).
    static func delete(_ connection: ServerConnection,
                       from store: ConnectionStore,
                       keys: SSHKeyStore,
                       care: any ConnectionCareHandshake,
                       tearDown: () async -> Void,
                       dropSecret: (String) async -> Void,
                       dropShares: (UUID) -> Void = { ShareQueue.removeEntries(for: $0) }) async throws {
        // Before anything that waits: from here on care pairs nothing for
        // it, and nothing it saves now outlives the forgetting.
        try care.deletionBegan(connection.id)
        underWay.insert(connection.id)
        defer { underWay.remove(connection.id) }
        // Its conf off the host while the session can still ask (B377):
        // a moment at most, and the delete goes on whatever the host says.
        await care.forgetWindDown(connection.id)
        await tearDown()
        // The pass it stood down, bounded: the session is down by now, so
        // the host calls a pass waits on fail fast.
        await care.careWindDown(connection.id)
        // The item goes with the last record naming it (B232). A managed
        // key's item is named by the key, and stays while it is listed. It
        // used to stay whenever the connection had picked a managed key —
        // also once that key's record was gone (deleting a key left its
        // item for the connections still using it): the private key was
        // then named by nothing at all, for good. Unknown keeps it.
        if let ref = connection.keychainRef,
           let held = KeychainReferences.referenced(connections: store, keys: keys,
                                                    excludingConnection: connection.id),
           !held.contains(ref) {
            await dropSecret(ref)
        }
        // Its queued shares can never be delivered (the drain takes an entry
        // only for a live session), and their photos stayed in the App Group
        // until they aged out (B343). After the pairing, as the secret: a
        // delete that stopped there keeps them.
        dropShares(connection.id)
        store.delete(id: connection.id)
        care.deletionEnded(connection.id)
    }
}
