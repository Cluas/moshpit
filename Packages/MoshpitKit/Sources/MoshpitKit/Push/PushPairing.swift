import Foundation

/// One paired dev host: the two secrets that let it wake this phone, and the
/// connection they belong to.
///
/// Pairing is per CONNECTION, not per device. That costs a little (each host
/// gets its own one-liner) and buys three things worth more: unpairing one
/// server cannot break the others, a compromised host cannot forge
/// notifications that appear to come from a different one, and the relay's
/// rate limiting — keyed on the send token — is per host rather than shared
/// across all of them.
public struct PushPairing: Codable, Equatable, Identifiable {
    /// The app's own connection id. It travels to the host and comes back inside
    /// every sealed envelope, which is what lets a pushed notification route
    /// straight into the existing lock-screen Allow/Deny path.
    public var connectionId: UUID
    /// Display name of the host, for the settings row. Never sent anywhere.
    public var hostLabel: String
    /// 32 bytes, hex. The end-to-end key. Exists here and in the user's own
    /// ~/.moshpit/push.d conf — never on the relay.
    public var secretHex: String
    /// The bearer credential the host presents to the relay — MINTED BY THE
    /// RELAY, not here. It is an HMAC, under a key only the relay holds, over
    /// the routing facts below; presenting it alongside those facts is what
    /// lets a stateless relay authenticate a push with no registry to look
    /// anything up in. Empty until the phone has a device token to mint
    /// against; a pairing with an empty token must never reach a host.
    public var sendToken: String
    public var relayURL: String
    public var createdAt: Date
    /// The APNs device token `sendToken` was minted over. Travels to the host's
    /// conf so every push carries its own routing; compared against the CURRENT
    /// device token to notice a rotation (reinstall, restore) that silently
    /// invalidates the credential.
    public var apnsToken: String?
    /// "production" or "sandbox" — the build's own guess, a routing hint only.
    public var apnsEnv: String?
    /// When the relay minted `sendToken`. Tokens expire relay-side (the
    /// stateless design's revocation story), so the phone re-mints well before
    /// that (crates/seal's `needs_mint`).
    public var sendTokenIssuedAt: Date?

    public var id: UUID { connectionId }

    public init(connectionId: UUID, hostLabel: String, secretHex: String, sendToken: String,
                relayURL: String, createdAt: Date, apnsToken: String? = nil, apnsEnv: String? = nil,
                sendTokenIssuedAt: Date? = nil) {
        self.connectionId = connectionId; self.hostLabel = hostLabel; self.secretHex = secretHex
        self.sendToken = sendToken; self.relayURL = relayURL; self.createdAt = createdAt
        self.apnsToken = apnsToken; self.apnsEnv = apnsEnv; self.sendTokenIssuedAt = sendTokenIssuedAt
    }
}

/// Where the pairings live, and why they live there.
///
/// The notification service extension MUST be able to read the secret: it is the
/// only process awake when a push lands, and without the key it can only show
/// the generic fallback. So the store has to be shared, and it has to be
/// readable while the device is LOCKED — which is precisely when an "agent needs
/// you" notification matters.
///
/// That second requirement is what rules out the app's `KeychainService`: every
/// item it writes is `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, so an
/// extension running on the lock screen cannot read it. This store instead uses
/// a file in the App Group container with
/// `.completeUntilFirstUserAuthentication` — the file-system equivalent of the
/// keychain's `AfterFirstUnlock` class, encrypted at rest under a key the
/// passcode protects — and excludes it from backups so the secrets do not ride
/// out in an unencrypted one.
///
/// A keychain item in an App-Group access group would be marginally stronger and
/// is the obvious hardening step; it is deliberately NOT the first
/// implementation because that path cannot be verified on the simulator (the
/// simulator keychain is permissive about access groups in a way devices are
/// not), and a store that silently fails on device would present as "push just
/// doesn't work".
public enum PushPairingStore {
    public static let appGroup = "group.com.cluas.moshpit"
    private static let fileName = "push-pairings.json"

    /// What the app reads and writes the pairings through: the Rust store
    /// (crates/seal's pairing file, the file ``PushPairingFile`` reads). Set
    /// once at launch, before anything reads a pairing. The notification
    /// service never sets it: it reads the file (``PushPairingFile``) and
    /// writes nothing.
    nonisolated(unsafe) public static var backing: PushPairingBacking?

    /// The file's place in the App Group container; nil with no container
    /// (an unsigned build), when nothing can be paired.
    public static var fileURL: URL? { url }

    private static var url: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroup)?
            .appendingPathComponent(fileName)
    }

    /// The store's file — or nil, logged, with no App Group container (an
    /// unsigned build), when every caller gets nothing.
    private static func file(_ verb: String) -> PushPairingFile? {
        guard let url else {
            Log.push.error("no App Group container — pairings cannot be \(verb, privacy: .public)")
            return nil
        }
        return PushPairingFile(url: url)
    }

    public static func read() -> [PushPairing] {
        if let backing { return backing.read() }
        return file("read")?.read() ?? []
    }

    /// Persist the pairings through the app's store (``backing``); false
    /// without one — the notification service, which only reads, and an
    /// unsigned build with no App Group container, where nothing can be
    /// paired. True only when the pairings read back as written.
    @discardableResult
    public static func write(_ pairings: [PushPairing]) -> Bool {
        backing?.write(pairings) ?? false
    }

    /// Replace (or add) the pairing for one connection. Re-pairing a host
    /// deliberately mints new material rather than reusing the old: the previous
    /// push.conf on that server stops working, which is the whole point of
    /// re-pairing after a machine changes hands.
    @discardableResult
    public static func upsert(_ pairing: PushPairing) -> Bool {
        backing?.upsert(pairing) ?? false
    }

    /// Forget one connection's pairing. True once nothing for it is left on
    /// this phone — including when nothing was there to begin with, as in a
    /// build with no App Group container, where no pairing can ever have been
    /// saved. Deleting a connection asks this of every connection, paired or
    /// not, and a "couldn't" there stops the delete.
    @discardableResult
    public static func remove(connectionId: UUID) -> Bool {
        backing?.remove(connectionId: connectionId) ?? true
    }

    public static func pairing(for connectionId: UUID) -> PushPairing? {
        read().first { $0.connectionId == connectionId }
    }
}

/// What ``PushPairingStore`` reads and writes through in the app: the Rust
/// store (crates/seal's pairing file, through crates/ffi), the file
/// ``PushPairingFile`` reads, by its rules (an entry at a time, what can't be
/// read carried, a file not fully read copied aside before a write, never a
/// write over one that can't be read, read back after) and with its
/// protection: complete until first user authentication, out of backups.
public protocol PushPairingBacking: AnyObject, Sendable {
    func read() -> [PushPairing]
    func write(_ pairings: [PushPairing]) -> Bool
    func upsert(_ pairing: PushPairing) -> Bool
    func remove(connectionId: UUID) -> Bool
}

/// The pairings file itself, read: a JSON array of ``PushPairing``, one per
/// paired connection — the format every build and the notification service
/// extension read. ``PushPairingStore`` is this file at the App Group path;
/// the app writes it through the Rust store (``PushPairingBacking``), and
/// the notification service only reads.
///
/// Read an entry at a time: entries this build decodes are the pairings it
/// uses, and one it can't is skipped here and kept on disk (the writer
/// carries it). A file that is there but can't be read at all (before the
/// first unlock, an I/O error) reads as no pairings, and says so.
public struct PushPairingFile {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    /// What one read of the file found.
    struct Contents {
        /// The pairings this build decoded, in stored order.
        var pairings: [PushPairing] = []
        /// The entries it couldn't, exactly as stored (JSON values).
        var carried: [Any] = []
        /// False when the bytes are not a JSON array at all — then none of
        /// them can be carried entry by entry.
        var isList = true
        /// The file is there but couldn't be read. Nothing is known about
        /// what it holds.
        var unreadable = false
    }

    /// The pairings this build can read. Never throws and never guesses:
    /// whatever it can't read is skipped here and kept on disk.
    public func read() -> [PushPairing] {
        contents().pairings
    }

    func contents() -> Contents {
        var contents = Contents()
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            guard FileManager.default.fileExists(atPath: url.path) else { return contents }  // nothing paired yet
            // A store that exists but can't be opened behaves exactly as if
            // nothing were paired, for as long as it lasts. Say so.
            contents.unreadable = true
            Log.push.error("pairing store present but unreadable: \(error.localizedDescription, privacy: .public)")
            return contents
        }
        guard let entries = (try? JSONSerialization.jsonObject(with: data)) as? [Any] else {
            contents.isList = false
            Log.push.error("pairing store present but not a list (\(data.count, privacy: .public) bytes) — nothing read, file kept")
            return contents
        }
        let decoder = JSONDecoder()
        for (index, entry) in entries.enumerated() {
            // Wrapped in a one-entry array: JSONSerialization won't write a
            // bare fragment.
            if let single = try? JSONSerialization.data(withJSONObject: [entry]),
               let pairing = try? decoder.decode([PushPairing].self, from: single).first {
                contents.pairings.append(pairing)
            } else {
                contents.carried.append(entry)
                Log.push.error("pairing \(index, privacy: .public) of \(entries.count, privacy: .public) undecodable — kept as stored")
            }
        }
        return contents
    }
}
