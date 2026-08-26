import CryptoKit
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
struct PushPairing: Codable, Equatable, Identifiable {
    /// The app's own connection id. It travels to the host and comes back inside
    /// every sealed envelope, which is what lets a pushed notification route
    /// straight into the existing lock-screen Allow/Deny path.
    var connectionId: UUID
    /// Display name of the host, for the settings row. Never sent anywhere.
    var hostLabel: String
    /// 32 bytes, hex. The end-to-end key. Exists here and in the user's own
    /// ~/.moshpit/push.conf — never on the relay.
    var secretHex: String
    /// 32 bytes, hex. The bearer credential the host presents to the relay. The
    /// relay stores only its SHA-256, so its database cannot be used to send.
    var sendToken: String
    var relayURL: String
    var createdAt: Date

    var id: UUID { connectionId }

    /// What the relay is told to file this device under.
    var sendTokenHash: String {
        Data(SHA256.hash(data: Data(sendToken.utf8))).moshpitHexString
    }


    /// Mint fresh material for a connection. Both values come from the system
    /// CSPRNG; neither is ever derived from anything guessable (a host name, a
    /// device id, a timestamp).
    static func make(connectionId: UUID, hostLabel: String, relayURL: String,
                     now: Date = Date()) -> PushPairing {
        PushPairing(connectionId: connectionId,
                    hostLabel: hostLabel,
                    secretHex: randomHex(bytes: 32),
                    sendToken: randomHex(bytes: 32),
                    relayURL: relayURL.trimmingCharacters(in: .whitespaces)
                        .trimmingSuffix("/"),
                    createdAt: now)
    }

    static func randomHex(bytes count: Int) -> String {
        var bytes = [UInt8](repeating: 0, count: count)
        let status = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        // A CSPRNG failure is not something to paper over with a fallback of
        // lesser randomness — that is how a "secret" silently becomes guessable.
        precondition(status == errSecSuccess, "SecRandomCopyBytes failed: \(status)")
        return Data(bytes).moshpitHexString
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
enum PushPairingStore {
    static let appGroup = "group.com.cluas.moshpit"
    private static let fileName = "push-pairings.json"

    private static var url: URL? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroup)?
            .appendingPathComponent(fileName)
    }

    static func read() -> [PushPairing] {
        guard let url else {
            Log.push.error("no App Group container — pairings cannot be read")
            return []
        }
        guard let data = try? Data(contentsOf: url) else { return [] }  // nothing paired yet
        do {
            return try JSONDecoder().decode([PushPairing].self, from: data)
        } catch {
            // A store that exists but will not decode is the worst case: the app
            // behaves exactly as if nothing were paired, forever. Say so.
            Log.push.error("pairing store present but undecodable: \(error.localizedDescription, privacy: .public)")
            return []
        }
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
    static func write(_ pairings: [PushPairing]) -> Bool {
        guard let url else {
            Log.push.error("no App Group container — pairings cannot be saved")
            return false
        }
        let data: Data
        do {
            data = try JSONEncoder().encode(pairings)
        } catch {
            Log.push.error("could not encode \(pairings.count, privacy: .public) pairing(s): \(error.localizedDescription, privacy: .public)")
            return false
        }
        do {
            try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            // The combination that can fail on a real device and never on a
            // simulator: an atomic write with a protection class, before first
            // unlock or under disk pressure.
            Log.push.error("could not save pairings: \(error.localizedDescription, privacy: .public)")
            return false
        }
        var resource = URLResourceValues()
        resource.isExcludedFromBackup = true
        var mutable = url
        do {
            try mutable.setResourceValues(resource)
        } catch {
            // Not fatal — the pairings are saved, they would just ride out in a
            // backup. Still not silent.
            Log.push.info("saved pairings but could not exclude them from backup: \(error.localizedDescription, privacy: .public)")
        }
        let readBack = read().count
        guard readBack == pairings.count else {
            Log.push.error("saved \(pairings.count, privacy: .public) pairing(s) but read back \(readBack, privacy: .public)")
            return false
        }
        return true
    }

    /// Replace (or add) the pairing for one connection. Re-pairing a host
    /// deliberately mints new material rather than reusing the old: the previous
    /// push.conf on that server stops working, which is the whole point of
    /// re-pairing after a machine changes hands.
    @discardableResult
    static func upsert(_ pairing: PushPairing) -> Bool {
        var all = read().filter { $0.connectionId != pairing.connectionId }
        all.append(pairing)
        return write(all)
    }

    static func remove(connectionId: UUID) {
        write(read().filter { $0.connectionId != connectionId })
    }

    static func pairing(for connectionId: UUID) -> PushPairing? {
        read().first { $0.connectionId == connectionId }
    }

    /// Every secret this device holds, newest first.
    ///
    /// The extension does not know WHICH host sealed the envelope it is holding
    /// — that fact is inside the envelope — so it tries each secret until one
    /// authenticates. Newest first because a freshly paired host is the likeliest
    /// sender, and because a re-paired host leaves its predecessor in place until
    /// the old push.conf is overwritten.
    static func secretsNewestFirst() -> [String] {
        read().sorted { $0.createdAt > $1.createdAt }.map(\.secretHex)
    }
}

private extension String {
    func trimmingSuffix(_ suffix: String) -> String {
        hasSuffix(suffix) ? String(dropLast(suffix.count)) : self
    }
}
