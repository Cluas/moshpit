import Foundation
import Crypto
import NIOCore
import NIOSSH
import Citadel

// MARK: - Known-hosts persistence

/// Persistence layer for the TOFU known-hosts map: `"host:port" -> fingerprint`.
protocol KnownHostsStore: Sendable {
    func read() -> [String: String]
    func write(_ entries: [String: String])
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

    init(backend: KeychainBackend? = nil, account: String = "moshpit.knownHosts") {
        self.backend = backend ?? Self.defaultBackend()
        self.account = account
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

    func read() -> [String: String] {
        guard let data = try? backend.load(account: account, prompt: "") else { return [:] }
        return (try? JSONDecoder().decode([String: String].self, from: data)) ?? [:]
    }

    func write(_ entries: [String: String]) {
        guard let data = try? JSONEncoder().encode(entries) else { return }
        // requireBiometry: false → item is stored WhenUnlockedThisDeviceOnly
        // with no access-control prompt, so writes/reads stay silent.
        try? backend.save(secret: data, account: account, requireBiometry: false)
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

    func read() -> [String: String] {
        guard let data = defaults.data(forKey: key) else { return [:] }
        return (try? JSONDecoder().decode([String: String].self, from: data)) ?? [:]
    }

    func write(_ entries: [String: String]) {
        guard let data = try? JSONEncoder().encode(entries) else { return }
        defaults.set(data, forKey: key)
    }
}

/// In-memory store for unit tests.
final class InMemoryKnownHostsStore: KnownHostsStore, @unchecked Sendable {
    private let lock = NSLock()
    private var entries: [String: String]

    init(seed: [String: String] = [:]) {
        self.entries = seed
    }

    func read() -> [String: String] {
        lock.lock(); defer { lock.unlock() }
        return entries
    }

    func write(_ entries: [String: String]) {
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
    private var cache: [String: String]

    init(store: KnownHostsStore = KeychainKnownHostsStore()) {
        self.store = store
        self.cache = store.read()
    }

    /// Test convenience.
    static func inMemory(seed: [String: String] = [:]) -> HostKeyValidator {
        HostKeyValidator(store: InMemoryKnownHostsStore(seed: seed))
    }

    /// Pure decision — no side effects, no I/O. SSH layer interprets and
    /// optionally calls `trust(...)` afterwards.
    func decide(host: String, port: Int, fingerprint: String) -> HostKeyDecision {
        let key = Self.entryKey(host: host, port: port)
        guard let stored = cache[key] else {
            return .unknown
        }
        if stored == fingerprint {
            return .trusted
        }
        return .changed(expected: stored)
    }

    /// Commit acceptance of a fingerprint (either after first-time prompt or
    /// after the user manually approves a host-key change).
    func trust(host: String, port: Int, fingerprint: String) {
        let key = Self.entryKey(host: host, port: port)
        cache[key] = fingerprint
        store.write(cache)
    }

    /// Drop a stored fingerprint (e.g. user re-keys the server).
    func forget(host: String, port: Int) {
        let key = Self.entryKey(host: host, port: port)
        cache.removeValue(forKey: key)
        store.write(cache)
    }

    /// Snapshot for tests / UI.
    func snapshot() -> [String: String] {
        cache
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

/// Marker error thrown when the user (or policy) rejects a presented host
/// key. Used to fail the Citadel `validateHostKey` promise; Citadel's own
/// `InvalidHostKey` type is public but its synthesized initializer is
/// module-internal, so we surface our own to abort the handshake.
public struct HostKeyRejected: Error, Equatable {}

// MARK: - Citadel adapter

/// Bridge between Citadel's synchronous `NIOSSHClientServerAuthenticationDelegate`
/// callback and our async `HostKeyValidator` actor.
///
/// Citadel hands us an `EventLoopPromise<Void>` on the SSH channel's event
/// loop and expects us to either `succeed(())` (trust this server) or
/// `fail(HostKeyRejected())` (abort handshake). We:
///
///   1. Compute the SHA-256 fingerprint synchronously.
///   2. Hop into an unstructured `Task` to query the validator actor.
///   3. For `.unknown` / `.changed`, await the supplied user-prompt closures.
///   4. On accept, persist via `trust(...)` then succeed; on reject, fail.
///
/// The closures are `@Sendable` async so the consumer (typically a SwiftUI
/// VM bridging through `MainActor`) can show a sheet and resume with a Bool.
final class TOFUHostKeyDelegate: NIOSSHClientServerAuthenticationDelegate, @unchecked Sendable {
    typealias TrustNewHost = @Sendable (_ host: String, _ port: Int, _ fingerprint: String) async -> Bool
    typealias AcceptChangedHost = @Sendable (_ host: String, _ port: Int, _ newFingerprint: String, _ oldFingerprint: String) async -> Bool

    private let validator: HostKeyValidator
    private let host: String
    private let port: Int
    private let onNewHost: TrustNewHost
    private let onChangedHost: AcceptChangedHost

    init(validator: HostKeyValidator,
         host: String,
         port: Int,
         onNewHost: @escaping TrustNewHost,
         onChangedHost: @escaping AcceptChangedHost) {
        self.validator = validator
        self.host = host
        self.port = port
        self.onNewHost = onNewHost
        self.onChangedHost = onChangedHost
    }

    func validateHostKey(hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>) {
        let fingerprint = HostKeyValidator.fingerprint(of: hostKey)
        let validator = self.validator
        let host = self.host
        let port = self.port
        let onNewHost = self.onNewHost
        let onChangedHost = self.onChangedHost

        Task {
            let decision = await validator.decide(host: host, port: port, fingerprint: fingerprint)
            switch decision {
            case .trusted:
                validationCompletePromise.succeed(())
            case .unknown:
                let accept = await onNewHost(host, port, fingerprint)
                if accept {
                    await validator.trust(host: host, port: port, fingerprint: fingerprint)
                    validationCompletePromise.succeed(())
                } else {
                    validationCompletePromise.fail(HostKeyRejected())
                }
            case .changed(let expected):
                let accept = await onChangedHost(host, port, fingerprint, expected)
                if accept {
                    await validator.trust(host: host, port: port, fingerprint: fingerprint)
                    validationCompletePromise.succeed(())
                } else {
                    validationCompletePromise.fail(HostKeyRejected())
                }
            }
        }
    }
}
