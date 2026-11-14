import Foundation
import UIKit
import Crypto
import NIOCore
import NIOSSH
import Citadel
import MoshpitKit

// MARK: - Known-hosts persistence

/// Persistence layer for the TOFU known-hosts map: `"host:port" -> fingerprint`.
protocol KnownHostsStore: Sendable {
    /// The map as stored — empty when nothing has been stored yet.
    ///
    /// Throws when the map can't be read right now: the device is locked (the
    /// production store is a `WhenUnlockedThisDeviceOnly` keychain item), the
    /// keychain failed, or the bytes can't be kept safe. That says nothing
    /// about what the map holds, so it must never be taken for an empty one:
    /// an empty map makes every host "unknown", and the next trust written
    /// over it erases every fingerprint the user ever accepted.
    func read() throws -> [String: String]
    func write(_ entries: [String: String]) throws
}

/// The known-hosts map couldn't be read — see ``KnownHostsStore/read()``.
/// Also what a handshake fails with when that is why its host key could not
/// be checked (``HostKeyDecision/unverifiable``).
struct KnownHostsUnreadable: Error, CustomStringConvertible {
    let reason: String
    /// The device is locked: the map comes back the moment it is unlocked.
    /// Anything else is a store that is broken until something changes.
    var locked = false
    var description: String { "known hosts unreadable: \(reason)" }
}

/// Default `KnownHostsStore` backed by the iOS Keychain.
///
/// The TOFU fingerprint map is not a secret, but it MUST be tamper-resistant:
/// if an attacker could pre-seed a fingerprint (e.g. by editing a plist inside
/// an extracted device backup, or on a jailbroken device), a later MITM'd host
/// key would wrongly classify as `.trusted` and the handshake would proceed
/// silently. Storing the map as a Keychain generic password with
/// `WhenUnlockedThisDeviceOnly` protection excludes it from backups and scopes
/// it to this device, closing that vector. No biometry is required — this is
/// integrity / backup-exclusion, not confidentiality — so reads never prompt.
///
/// The whole map is serialized to a single JSON blob under one keychain
/// account, matching the semantics of the previous `UserDefaults` backing (one
/// value, read/rewritten as a unit) so nothing downstream changes.
final class KeychainKnownHostsStore: KnownHostsStore, @unchecked Sendable {
    private let backend: KeychainBackend
    private let account: String
    /// Whether the device's protected data is available — asked around a
    /// read that found nothing, to tell "nothing stored" from "nothing the
    /// keychain will show yet" (B226). nil takes "nothing stored" at its
    /// word (a test's store; the app's is ``app``).
    private let protectedDataAvailable: (@Sendable () -> Bool)?

    init(backend: KeychainBackend? = nil, account: String = "moshpit.knownHosts",
         protectedDataAvailable: (@Sendable () -> Bool)? = nil) {
        self.backend = backend ?? Self.defaultBackend()
        self.account = account
        self.protectedDataAvailable = protectedDataAvailable
    }

    /// Mirror `KeychainService`'s backend selection: the system keychain on a
    /// real device, and a sandboxed file vault on the Simulator (where unsigned
    /// builds get `errSecMissingEntitlement` from every keychain call).
    private static func defaultBackend() -> KeychainBackend {
        #if targetEnvironment(simulator)
        return FileVaultKeychainBackend(filename: "moshpit-known-hosts.json")
        #else
        return SystemKeychainBackend()
        #endif
    }

    /// Where a map this build can't decode is copied before anything may be
    /// written over it.
    var backupAccount: String { account + ".backup" }

    func read() throws -> [String: String] {
        let unlocked = { self.protectedDataAvailable?() ?? true }
        let unlockedBefore = unlocked()
        let data: Data
        do {
            data = try backend.load(account: account, prompt: "")
        } catch KeychainError.notFound {
            // Nothing trusted yet — or nothing the keychain will show yet.
            // A launch before the first unlock after a reboot (a push, a
            // Live Activity intent, prewarming) can find no item at all
            // where there is one, and that empty map was believed: cached
            // as the table, every host read as new, and the first "Trust"
            // after the unlock wrote one entry over every fingerprint the
            // user had (B226, the other half of B22). An empty answer is
            // the truth only from a device unlocked around the read — the
            // sealed-launch rule (``DefaultsReadiness``); otherwise it is
            // the lock's, and comes back once the device is unlocked.
            guard unlockedBefore, unlocked() else {
                throw KnownHostsUnreadable(reason: "nothing read before the first unlock", locked: true)
            }
            return [:]
        } catch KeychainError.interactionNotAllowed {
            // What a WhenUnlocked item answers while the device is locked.
            // The map is there or it isn't; nobody here knows which.
            throw KnownHostsUnreadable(reason: "the device is locked", locked: true)
        } catch {
            throw KnownHostsUnreadable(reason: String(describing: error))
        }
        if let entries = try? JSONDecoder().decode([String: String].self, from: data) {
            return entries
        }
        return try salvage(data)
    }

    /// A map that won't decode as `[host:port: fingerprint]` — written by a
    /// newer build, or damaged. Its bytes are copied untouched to
    /// ``backupAccount`` (read back, not assumed) before the entries that DO
    /// read as host → fingerprint are handed out, so the write that follows
    /// a trust never destroys the only copy. No copy, no map.
    private func salvage(_ data: Data) throws -> [String: String] {
        if (try? backend.load(account: backupAccount, prompt: "")) != data {
            try? backend.save(secret: data, account: backupAccount, requireBiometry: false)
            guard (try? backend.load(account: backupAccount, prompt: "")) == data else {
                throw KnownHostsUnreadable(reason: "undecodable, and no copy of it could be kept")
            }
            Log.ssh.error("known hosts: stored map undecodable (\(data.count, privacy: .public) bytes); copied to \(self.backupAccount, privacy: .public)")
        }
        // StoredJSON keeps a host's leading U+FEFF, which JSONSerialization
        // alone drops (B155): the entry would come back under another host.
        let object = (try? StoredJSON.object(with: data)) as? [String: Any]
        return object?.compactMapValues { $0 as? String } ?? [:]
    }

    func write(_ entries: [String: String]) throws {
        let data = try JSONEncoder().encode(entries)
        // requireBiometry: false → item is stored WhenUnlockedThisDeviceOnly
        // with no access-control prompt, so writes/reads stay silent.
        try backend.save(secret: data, account: account, requireBiometry: false)
    }
}

/// Legacy `KnownHostsStore` backed by `UserDefaults`. No longer the production
/// default (it has no tamper-resistance and is captured in device backups —
/// see `KeychainKnownHostsStore`), but retained for tests that want a plain,
/// easily-inspectable persistence layer scoped to an injected `UserDefaults`
/// suite. Survives app restarts; cleared by deleting/reinstalling the app.
final class UserDefaultsKnownHostsStore: KnownHostsStore, @unchecked Sendable {
    private let key: String
    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard, key: String = "moshpit.knownHosts") {
        self.defaults = defaults
        self.key = key
    }

    func read() throws -> [String: String] {
        guard let data = defaults.data(forKey: key) else { return [:] }
        do {
            return try JSONDecoder().decode([String: String].self, from: data)
        } catch {
            throw KnownHostsUnreadable(reason: "undecodable: \(error)")
        }
    }

    func write(_ entries: [String: String]) throws {
        defaults.set(try JSONEncoder().encode(entries), forKey: key)
    }
}

/// In-memory store for unit tests.
final class InMemoryKnownHostsStore: KnownHostsStore, @unchecked Sendable {
    private let lock = NSLock()
    private var entries: [String: String]

    init(seed: [String: String] = [:]) {
        self.entries = seed
    }

    func read() throws -> [String: String] {
        lock.lock(); defer { lock.unlock() }
        return entries
    }

    func write(_ entries: [String: String]) throws {
        lock.lock(); defer { lock.unlock() }
        self.entries = entries
    }
}

// MARK: - Decision model

/// TOFU decision the SSH layer must act on before completing the handshake.
enum HostKeyDecision: Equatable {
    /// Fingerprint matches a stored entry → safe to proceed.
    case trusted
    /// Host never seen before → ask user, then call `trust(...)` if they accept.
    case unknown
    /// Stored fingerprint differs from server's → potential MITM, prompt user
    /// with the previously-trusted fingerprint for comparison.
    case changed(expected: String)
    /// The known-hosts map can't be read until the device is unlocked.
    /// Neither a prompt nor a pass: "unknown" would offer to trust a host the
    /// user trusted long ago, and that trust used to be the write that erased
    /// every other fingerprint. The handshake fails; the next one after an
    /// unlock reads the map and decides for real.
    case unverifiable
}

// MARK: - Validator actor

/// Trust-on-first-use host key store.
///
/// The validator is an `actor` so concurrent SSH connection attempts mutate
/// the known-hosts map serially. It is intentionally I/O-free: callers
/// (typically `SSHService`) decide UI flow based on `HostKeyDecision` and
/// then commit acceptance via `trust(...)`.
actor HostKeyValidator {
    private let store: KnownHostsStore
    /// The map as last read from the store, plus every trust written since.
    /// nil until a read has SUCCEEDED: "couldn't read it" is not "nothing
    /// trusted", and is never cached as such. It used to be — the first
    /// SSH of the process creates `SSHService.shared`, and when that
    /// happened with the device locked (the keychain item is readable only
    /// while unlocked) the empty answer stuck for the life of the process:
    /// every host showed as new, and the first "Trust" wrote a one-entry
    /// map over every fingerprint the user had.
    private var table: [String: String]?
    /// Trusts not in the store yet — made while the map couldn't be read, or
    /// whose write failed. They count for this process's decisions, and are
    /// written on top of the map the first time it can be read.
    private var unsaved: [String: String] = [:]
    private let unlocks = UnlockWatch()

    /// `unlocks` is where "protected data became available" is heard — the
    /// moment a map that was unreadable can be read again.
    init(store: KnownHostsStore = AppStores.knownHosts(),
         unlocks center: NotificationCenter = .default) {
        self.store = store
        // Nothing is read here: this runs whenever the shared service is
        // first touched, locked or not. The first decision reads.
        unlocks.observe(center) { [weak self] in
            Task { await self?.reload() }
        }
    }

    deinit { unlocks.stop() }

    /// Test convenience.
    static func inMemory(seed: [String: String] = [:]) -> HostKeyValidator {
        HostKeyValidator(store: InMemoryKnownHostsStore(seed: seed))
    }

    /// The decision for one presented key. No side effects beyond reading
    /// the map the first time it can be read; the SSH layer interprets and
    /// optionally calls `trust(...)` afterwards.
    func decide(host: String, port: Int, fingerprint: String) -> HostKeyDecision {
        #if DEBUG
        // A DEBUG seed's private sshd, trusted for this launch only.
        if let seeded = LaunchTrust.fingerprint(host: host, port: port) {
            return seeded == fingerprint ? .trusted : .changed(expected: seeded)
        }
        #endif
        let key = Self.entryKey(host: host, port: port)
        let stored: String?
        if let pending = unsaved[key] {
            stored = pending
        } else {
            switch loadTable() {
            case .read(let table):
                stored = table[key]
            case .locked:
                return .unverifiable
            case .broken:
                // Not a lock that will lift: refusing every connection until
                // the keychain heals itself would strand the user. Ask, as
                // for a new host — what this did before — but nothing asked
                // here is ever written over the map (see `save()`).
                stored = nil
            }
        }
        guard let stored else { return .unknown }
        return stored == fingerprint ? .trusted : .changed(expected: stored)
    }

    /// Commit acceptance of a fingerprint (either after first-time prompt or
    /// after the user manually approves a host-key change).
    ///
    /// Written on top of the map as READ — never on top of a guess. While the
    /// map can't be read the trust holds for this process only, and goes into
    /// the store with the first read that succeeds.
    func trust(host: String, port: Int, fingerprint: String) {
        unsaved[Self.entryKey(host: host, port: port)] = fingerprint
        save()
    }

    /// Read the map if no read has succeeded yet, and write out any trust
    /// still waiting. Runs when protected data becomes available.
    func reload() {
        _ = loadTable()
        save()
    }

    /// Snapshot for tests / UI.
    func snapshot() -> [String: String] {
        (table ?? [:]).merging(unsaved) { _, newer in newer }
    }

    private enum TableRead {
        case read([String: String])
        /// Comes back once the device is unlocked.
        case locked
        /// Won't come back on its own.
        case broken
    }

    /// The map as stored, read now if no read has succeeded yet. A failure
    /// is not remembered — the next call asks again.
    private func loadTable() -> TableRead {
        if let table { return .read(table) }
        do {
            let read = try store.read()
            table = read
            return .read(read)
        } catch {
            Log.ssh.info("host keys: \(String(describing: error), privacy: .public) — not deciding from it")
            return (error as? KnownHostsUnreadable)?.locked == true ? .locked : .broken
        }
    }

    /// Write the waiting trusts over the map as stored right now. Nothing is
    /// written without a read that succeeded just before it, so the stored
    /// map never shrinks to what this process happens to know.
    ///
    /// Read again for the write, not taken from the copy decisions are made
    /// from (as `moshpit-ssh`'s `KnownHosts::save` does): the keychain item
    /// has other writers in this process — the Rust core's sessions, while
    /// its switch is on — and a trust one of them wrote after this copy was
    /// read was written away by the next trust made here. And a copy is
    /// only ever as good as the read it came from (B226).
    private func save() {
        guard !unsaved.isEmpty else { return }
        let current: [String: String]
        do {
            current = try store.read()
        } catch {
            Log.ssh.info("host keys: not written now (\(String(describing: error), privacy: .public)); kept for the next try")
            return
        }
        let merged = current.merging(unsaved) { _, newer in newer }
        do {
            try store.write(merged)
            self.table = merged
            unsaved = [:]
        } catch {
            Log.ssh.error("host keys: write failed (\(String(describing: error), privacy: .public)); kept for the next try")
        }
    }

    private static func entryKey(host: String, port: Int) -> String {
        "\(host):\(port)"
    }

    // MARK: - Fingerprint helpers

    /// OpenSSH-style fingerprint: `SHA256:<unpadded-base64>` of the wire
    /// encoding of the host key (matches `ssh-keygen -lf -E sha256`).
    static func fingerprint(of key: NIOSSHPublicKey) -> String {
        var buffer = ByteBuffer()
        // Write the full SSH wire encoding including the `ssh-...` header,
        // which is what OpenSSH hashes for fingerprints.
        key.write(to: &buffer)
        let bytes = buffer.getBytes(at: buffer.readerIndex, length: buffer.readableBytes) ?? []
        let digest = SHA256.hash(data: Data(bytes))
        let b64 = Data(digest).base64EncodedString().trimmingCharacters(in: CharacterSet(charactersIn: "="))
        return "SHA256:\(b64)"
    }
}

extension KeychainKnownHostsStore {
    /// The app's store: the keychain item, read by the sealed-launch rule
    /// (B226) — "nothing stored" believed only from an unlocked device.
    static func app() -> KeychainKnownHostsStore {
        KeychainKnownHostsStore(protectedDataAvailable: { ProtectedDataFlag.app.isAvailable })
    }
}

extension ProtectedDataFlag {
    /// The app's one flag, made the first time anything asks for it: at
    /// launch, on the main thread, where `MoshpitApp.init` asks first — so
    /// a read off the main thread later never waits on it. Made from off
    /// the main thread (a test with no app) it waits for the main queue
    /// once, as ``DefaultsReadiness/system`` does.
    static let app: ProtectedDataFlag = {
        if Thread.isMainThread {
            return MainActor.assumeIsolated { ProtectedDataFlag() }
        }
        return DispatchQueue.main.sync { MainActor.assumeIsolated { ProtectedDataFlag() } }
    }()
}

/// One block-based `NotificationCenter` observation of the device's unlock
/// (`protectedDataDidBecomeAvailable`), removable from a nonisolated
/// `deinit` — the validator is an actor, and its init can't keep mutable
/// state of its own once `self` has been captured. The session hub hears
/// the unlock through one too. `action` runs on the posting thread.
final class UnlockWatch: @unchecked Sendable {
    private let lock = NSLock()
    private var center: NotificationCenter?
    private var token: NSObjectProtocol?

    func observe(_ center: NotificationCenter, _ action: @escaping @Sendable () -> Void) {
        let token = center.addObserver(
            forName: UIApplication.protectedDataDidBecomeAvailableNotification,
            object: nil, queue: nil) { _ in action() }
        lock.withLock {
            self.center = center
            self.token = token
        }
    }

    func stop() {
        let (center, token) = lock.withLock { (self.center, self.token) }
        if let center, let token { center.removeObserver(token) }
    }
}

/// Marker error thrown when the user (or policy) rejects a presented host
/// key: what `SSHService.connect` fails with once the user has declined the
/// key a ``HostKeyNeedsDecision`` put to them. (Citadel's own
/// `InvalidHostKey` is public but its synthesized initializer is
/// module-internal.)
public struct HostKeyRejected: Error, Equatable {}

// MARK: - Citadel adapter

/// What a handshake fails with when the server's key is one the user has to
/// judge — a host never seen, or a key that changed. The handshake ends there
/// and its connection is closed; `SSHService.connect` asks, with no
/// connection open and no clock running, and dials again once the user has
/// trusted the key.
struct HostKeyNeedsDecision: Error, Equatable {
    let host: String
    let port: Int
    let fingerprint: String
    /// The fingerprint trusted before, when the key changed; nil for a host
    /// never seen.
    let previous: String?
}

/// Bridge between Citadel's synchronous `NIOSSHClientServerAuthenticationDelegate`
/// callback and our async `HostKeyValidator` actor.
///
/// Citadel hands us an `EventLoopPromise<Void>` on the SSH channel's event
/// loop and expects us to either `succeed(())` (trust this server) or fail it
/// (abort the handshake). A trusted key passes; a key the user has to judge
/// fails it with ``HostKeyNeedsDecision`` and is never asked about here.
///
/// It used to ask from inside the handshake, which Citadel gives 10 seconds
/// for everything: a user who took longer over the new-host card saw "can't
/// reach the server", while the Trust they then tapped was saved anyway and
/// finished a sign-in on a connection nobody held any more (B132). The server
/// meanwhile counted the user's thinking time against its own login grace.
final class TOFUHostKeyDelegate: NIOSSHClientServerAuthenticationDelegate, @unchecked Sendable {
    typealias TrustNewHost = @Sendable (_ host: String, _ port: Int, _ fingerprint: String) async -> Bool
    typealias AcceptChangedHost = @Sendable (_ host: String, _ port: Int, _ newFingerprint: String, _ oldFingerprint: String) async -> Bool

    private let validator: HostKeyValidator
    private let host: String
    private let port: Int

    init(validator: HostKeyValidator, host: String, port: Int) {
        self.validator = validator
        self.host = host
        self.port = port
    }

    func validateHostKey(hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>) {
        let fingerprint = HostKeyValidator.fingerprint(of: hostKey)
        let validator = self.validator
        let host = self.host
        let port = self.port

        Task {
            switch await validator.decide(host: host, port: port, fingerprint: fingerprint) {
            case .trusted:
                validationCompletePromise.succeed(())
            case .unknown:
                validationCompletePromise.fail(HostKeyNeedsDecision(
                    host: host, port: port, fingerprint: fingerprint, previous: nil))
            case .changed(let expected):
                validationCompletePromise.fail(HostKeyNeedsDecision(
                    host: host, port: port, fingerprint: fingerprint, previous: expected))
            case .unverifiable:
                // No prompt: there is nothing true to ask. The trusted keys
                // can't be read until the device is unlocked, and the
                // reconnect after that decides for real.
                validationCompletePromise.fail(
                    KnownHostsUnreadable(reason: "the device is locked", locked: true))
            }
        }
    }
}
