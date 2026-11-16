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
    /// anything up in. Empty until `PushService` has a device token to mint
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
    /// that — see `needsMint`.
    public var sendTokenIssuedAt: Date?

    public var id: UUID { connectionId }

    /// Re-mint this far after issue. The relay refuses tokens older than its
    /// TTL (45 days by default); refreshing at a third of that means a host
    /// only loses pushes if it goes a month and a half with no connection from
    /// this phone — and the first connect after that heals it.
    public static let refreshAfter: TimeInterval = 14 * 24 * 3600

    /// Whether the credential must be (re)minted before this pairing is worth
    /// installing: never minted, minted for a device token this phone no
    /// longer has, or old enough that expiry is in sight.
    public func needsMint(currentToken: String?, now: Date = Date()) -> Bool {
        if sendToken.isEmpty { return true }
        guard let apnsToken, let sendTokenIssuedAt else { return true }
        if let currentToken, apnsToken.lowercased() != currentToken.lowercased() { return true }
        return now.timeIntervalSince(sendTokenIssuedAt) > Self.refreshAfter
    }

    /// A conf written from a pairing missing any of these would be one the
    /// sender skips (at best) or the relay 401s (at worst) — never install it.
    public var isReadyToInstall: Bool {
        !sendToken.isEmpty && apnsToken?.isEmpty == false && sendTokenIssuedAt != nil
    }

    /// Mint the end-to-end secret for a connection. The secret comes from the
    /// system CSPRNG; it is never derived from anything guessable (a host
    /// name, a device id, a timestamp). The SEND token is deliberately absent —
    /// only the relay can mint one, and only against a device token.
    public static func make(connectionId: UUID, hostLabel: String, relayURL: String,
                            now: Date = Date()) -> PushPairing {
        PushPairing(connectionId: connectionId,
                    hostLabel: hostLabel,
                    secretHex: randomHex(bytes: 32),
                    sendToken: "",
                    relayURL: relayURL.trimmingCharacters(in: .whitespaces)
                        .trimmingSuffix("/"),
                    createdAt: now)
    }

    public static func randomHex(bytes count: Int) -> String {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        // A CSPRNG failure is not something to paper over with a fallback of
        // lesser randomness — that is how a "secret" silently becomes guessable.
        precondition(status == errSecSuccess, "SecRandomCopyBytes failed: \(status)")
        return Data(bytes).moshpitHexString
    }

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

    /// What the app reads and writes the pairings through instead of
    /// ``PushPairingFile``: the Rust store (crates/ffi's pairings file, the
    /// same file and rules), behind the app's `MOSHPIT_RUST_STORE` switch.
    /// Set once at launch, before anything reads a pairing; nil is the code
    /// below. The notification service never sets it: it reads the file as
    /// it always has.
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

    /// Persist the pairings, and say so when it fails.
    ///
    /// This used to be three `try?`s and no logging, which was the one silent
    /// path left in this feature — and it guards the entrance to the whole
    /// chain. A failed write makes `read()` return an empty list, which makes
    /// `registerForRemoteNotifications()` return early on "no paired hosts",
    /// which means no permission prompt, no device token and no push: a user
    /// looking at "paired" and never hearing a sound, with nothing anywhere
    /// pointing at the cause. `read()`'s own comment already demanded better —
    /// "a store that exists but will not decode is the worst case… Say so" —
    /// and this is the same failure on the half of the pair it could not see.
    ///
    /// Returns whether the pairings are actually readable back afterwards, so a
    /// caller can refuse to go further. That read-back is the same discipline
    /// `HostInstaller` applies to every file it writes to a host, pointed at the
    /// local container instead: writing and hoping is what this whole engine
    /// exists to stop doing.
    @discardableResult
    public static func write(_ pairings: [PushPairing]) -> Bool {
        if let backing { return backing.write(pairings) }
        return file("saved")?.write(pairings) ?? false
    }

    /// Replace (or add) the pairing for one connection. Re-pairing a host
    /// deliberately mints new material rather than reusing the old: the previous
    /// push.conf on that server stops working, which is the whole point of
    /// re-pairing after a machine changes hands.
    @discardableResult
    public static func upsert(_ pairing: PushPairing) -> Bool {
        if let backing { return backing.upsert(pairing) }
        return file("saved")?.upsert(pairing) ?? false
    }

    /// Forget one connection's pairing. True once nothing for it is left on
    /// this phone — including when nothing was there to begin with, as in a
    /// build with no App Group container, where no pairing can ever have been
    /// saved. Deleting a connection asks this of every connection, paired or
    /// not, and a "couldn't" there stops the delete.
    @discardableResult
    public static func remove(connectionId: UUID) -> Bool {
        if let backing { return backing.remove(connectionId: connectionId) }
        guard let url else { return true }
        return PushPairingFile(url: url).remove(connectionId: connectionId)
    }

    public static func pairing(for connectionId: UUID) -> PushPairing? {
        read().first { $0.connectionId == connectionId }
    }

    /// Every secret this device holds, newest first.
    ///
    /// The extension does not know WHICH host sealed the envelope it is holding
    /// — that fact is inside the envelope — so it tries each secret until one
    /// authenticates. Newest first because a freshly paired host is the likeliest
    /// sender, and because a re-paired host leaves its predecessor in place until
    /// the old push.conf is overwritten.
    public static func secretsNewestFirst() -> [String] {
        read().sorted { $0.createdAt > $1.createdAt }.map(\.secretHex)
    }
}

/// What ``PushPairingStore`` reads and writes through when it isn't
/// ``PushPairingFile``: the same file at the same place, by the same rules —
/// each member keeps the contract of ``PushPairingFile``'s member of the same
/// name (an entry at a time, what can't be read carried, a file not fully
/// read copied aside before a write, never a write over one that can't be
/// read, read back after) — and the same protection: complete until first
/// user authentication, out of backups.
public protocol PushPairingBacking: AnyObject, Sendable {
    func read() -> [PushPairing]
    func write(_ pairings: [PushPairing]) -> Bool
    func upsert(_ pairing: PushPairing) -> Bool
    func remove(connectionId: UUID) -> Bool
}

/// The pairings file itself: a JSON array of ``PushPairing``, one per paired
/// connection — the format every build and the notification service
/// extension read. ``PushPairingStore`` is this file at the App Group path.
///
/// Read an entry at a time, and nothing it can't read is ever dropped. The
/// array used to decode in one piece, and anything that failed — one entry
/// in a shape this build doesn't know (a rollback to an older TestFlight
/// build), one damaged byte, a file that couldn't be opened — read as "no
/// pairings". The next pair or unpair wrote that over the file, and every
/// other host stopped being able to reach this phone, with nothing on screen
/// saying so. Now:
///  * entries this build decodes are the pairings it uses;
///  * entries it can't are written back exactly as stored, after the others,
///    on every save — unless the save re-pairs or unpairs that very
///    connection;
///  * a file that holds anything this build couldn't decode is copied to
///    ``backupURL`` before the first write over it;
///  * a file that is there but can't be read at all (before the first unlock,
///    an I/O error) is never written over.
public struct PushPairingFile {
    public let url: URL

    public init(url: URL) {
        self.url = url
    }

    /// Where a file this build couldn't fully read is kept, untouched, before
    /// anything is written over it.
    public var backupURL: URL { url.appendingPathExtension("backup") }

    /// What one read of the file found.
    struct Contents {
        /// The pairings this build decoded, in stored order.
        var pairings: [PushPairing] = []
        /// The entries it couldn't, exactly as stored (JSON values).
        var carried: [Any] = []
        /// The file's bytes; nil when there is no file.
        var raw: Data?
        /// False when the bytes are not a JSON array at all — then none of
        /// them can be carried entry by entry.
        var isList = true
        /// The file is there but couldn't be read. Nothing is known about
        /// what it holds.
        var unreadable = false

        /// Everything stored was read.
        var isComplete: Bool { carried.isEmpty && isList && !unreadable }
    }

    /// The pairings this build can read. Never throws and never guesses:
    /// whatever it can't read is skipped here and kept on disk.
    public func read() -> [PushPairing] {
        contents().pairings
    }

    /// Replace every pairing with `pairings` — after keeping a copy of a file
    /// that held anything this build couldn't read.
    @discardableResult
    public func write(_ pairings: [PushPairing]) -> Bool {
        save(pairings, carrying: [], over: contents())
    }

    /// Replace (or add) the pairing for one connection.
    @discardableResult
    public func upsert(_ pairing: PushPairing) -> Bool {
        let contents = contents()
        var pairings = contents.pairings.filter { $0.connectionId != pairing.connectionId }
        pairings.append(pairing)
        return save(pairings,
                    carrying: contents.carried.filter { !Self.entry($0, names: pairing.connectionId) },
                    over: contents)
    }

    /// Forget one connection's pairing, and only that one.
    ///
    /// A file that holds nothing for it — there is no file, or it is a list
    /// it could read and no entry, decoded or carried, names the connection —
    /// is left exactly as it is: nothing to forget, and a write that could
    /// fail would only fail the forgetting with it. Deleting a connection
    /// that was never paired comes through here.
    @discardableResult
    public func remove(connectionId: UUID) -> Bool {
        let contents = contents()
        if contents.isList, !contents.unreadable,
           !contents.pairings.contains(where: { $0.connectionId == connectionId }),
           !contents.carried.contains(where: { Self.entry($0, names: connectionId) }) {
            return true
        }
        return save(contents.pairings.filter { $0.connectionId != connectionId },
                    carrying: contents.carried.filter { !Self.entry($0, names: connectionId) },
                    over: contents)
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
        contents.raw = data
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

    /// Whether a carried entry is (some shape of) the pairing for `id`: re-
    /// pairing or unpairing that connection replaces it too.
    private static func entry(_ entry: Any, names id: UUID) -> Bool {
        guard let fields = entry as? [String: Any],
              let raw = fields["connectionId"] as? String else { return false }
        return UUID(uuidString: raw) == id
    }

    /// Write `pairings`, then `carried` as read, over the file `contents` was
    /// read from — and read it back.
    private func save(_ pairings: [PushPairing], carrying carried: [Any], over contents: Contents) -> Bool {
        guard !contents.unreadable else {
            Log.push.error("not saving pairings: the store couldn't be read, and a save now would replace what it holds")
            return false
        }
        guard secure(contents) else { return false }
        let data: Data
        do {
            let encoded = try JSONEncoder().encode(pairings)
            if carried.isEmpty {
                data = encoded
            } else {
                let list = ((try JSONSerialization.jsonObject(with: encoded)) as? [Any] ?? []) + carried
                data = try JSONSerialization.data(withJSONObject: list)
            }
        } catch {
            Log.push.error("could not encode \(pairings.count, privacy: .public) pairing(s): \(error.localizedDescription, privacy: .public)")
            return false
        }
        guard store(data, at: url, what: "pairings") else { return false }
        let readBack = self.contents()
        guard readBack.pairings.count == pairings.count, readBack.carried.count == carried.count else {
            Log.push.error("saved \(pairings.count, privacy: .public) pairing(s) but read back \(readBack.pairings.count, privacy: .public)")
            return false
        }
        return true
    }

    /// Keep an untouched copy of a file this build couldn't fully read, and
    /// say whether a write over it is now safe. A list with entries skipped
    /// carries them through every save, so its first copy is all it needs; a
    /// file that is no list at all is the only copy of whatever it holds, so
    /// each distinct one is kept.
    private func secure(_ contents: Contents) -> Bool {
        guard !contents.isComplete, let raw = contents.raw else { return true }
        let manager = FileManager.default
        if (try? Data(contentsOf: backupURL)) == raw { return true }
        var target = backupURL
        if manager.fileExists(atPath: target.path) {
            if contents.isList { return true }
            target = url.appendingPathExtension("backup-\(Int(Date().timeIntervalSince1970))")
        }
        guard store(raw, at: target, what: "a copy of the pairing store"),
              (try? Data(contentsOf: target)) == raw else {
            Log.push.error("could not keep a copy of the pairing store; not writing over it")
            return false
        }
        Log.push.notice("pairing store as found copied to \(target.lastPathComponent, privacy: .public)")
        return true
    }

    /// One atomic, protected, backup-excluded write. Secrets live in these
    /// files, so a copy gets exactly the treatment the store does.
    private func store(_ data: Data, at target: URL, what: String) -> Bool {
        // The container directory normally exists the moment iOS installs an
        // app with the App Group entitlement — but not on every path an
        // unsigned simulator test host takes, and an atomic write into a
        // missing directory fails identically to a full disk.
        try? FileManager.default.createDirectory(at: target.deletingLastPathComponent(),
                                                 withIntermediateDirectories: true)
        do {
            try data.write(to: target, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            // The combination that can fail on a real device and never on a
            // simulator: an atomic write with a protection class, before first
            // unlock or under disk pressure.
            Log.push.error("could not save \(what, privacy: .public): \(error.localizedDescription, privacy: .public)")
            return false
        }
        var resource = URLResourceValues()
        resource.isExcludedFromBackup = true
        var mutable = target
        do {
            try mutable.setResourceValues(resource)
        } catch {
            // Not fatal — the file is saved, it would just ride out in a
            // backup. Still not silent.
            Log.push.info("saved \(what, privacy: .public) but could not exclude it from backup: \(error.localizedDescription, privacy: .public)")
        }
        return true
    }
}

private extension String {
    func trimmingSuffix(_ suffix: String) -> String {
        hasSuffix(suffix) ? String(dropLast(suffix.count)) : self
    }
}
