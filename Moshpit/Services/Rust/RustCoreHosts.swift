import Foundation
import UIKit

// What the app gives the Rust core's sessions (crates/ffi/src/session.rs):
// its preferences, the keychain item of the trusted host keys, the
// credentials of a connection, and the place events land. Called on the
// core's threads; nothing here touches UI state except through the main
// queue.

// MARK: - Preferences

/// `UserDefaults.standard` as the store's ``PreferencesHost``: Foundation
/// property-list objects to the store's flat node list and back. What a
/// session remembers lives here under the app's own keys
/// (`moshpit.tmux.lastSelection.<id>`, `hostcaps.<id>`).
final class RustPreferencesHost: PreferencesHost, @unchecked Sendable {
    private let defaults: UserDefaults
    private let available: () -> Bool

    init(defaults: UserDefaults = .standard, protectedData: ProtectedDataFlag) {
        self.defaults = defaults
        available = { protectedData.isAvailable }
    }

    /// Readable as `available` says (a ``DefaultsReadiness``'s word: tests
    /// and scratch suites, a store simulating a sealed launch).
    init(defaults: UserDefaults, available: @escaping () -> Bool) {
        self.defaults = defaults
        self.available = available
    }

    func get(key: String) -> PlistValue? {
        defaults.object(forKey: key).flatMap(PlistNodes.value)
    }

    func set(key: String, value: PlistValue?) {
        if let value {
            defaults.set(PlistNodes.object(value), forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }

    func protectedDataAvailable() -> Bool {
        available()
    }
}

/// `UIApplication.isProtectedDataAvailable`, readable from any thread: the
/// core asks from its own threads, and a hop to the main queue there could
/// wait on a main thread that waits on the core.
///
/// Read when made, kept by the system's two notifications, and read again
/// each time the app becomes active. 1.0.8's readiness asked the system on
/// every read; a flag read once, at `MoshpitApp.init`, kept an early
/// "unavailable" until the phone was locked and unlocked again, and with no
/// sentinel yet (a first launch) no store could be trusted all that time.
/// CI's test host showed it: its preferences stayed untrusted the whole run.
final class ProtectedDataFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var available: Bool
    private var observers: [NSObjectProtocol] = []

    /// `read` is the system's word, asked on the main thread (a test's own).
    @MainActor
    init(center: NotificationCenter = .default,
         read: @escaping @MainActor () -> Bool = { UIApplication.shared.isProtectedDataAvailable }) {
        available = read()
        observers.append(center.addObserver(
            forName: UIApplication.protectedDataDidBecomeAvailableNotification,
            object: nil, queue: nil) { [weak self] _ in self?.set(true) })
        observers.append(center.addObserver(
            forName: UIApplication.protectedDataWillBecomeUnavailableNotification,
            object: nil, queue: nil) { [weak self] _ in self?.set(false) })
        // Posted on the main thread, where `read` asks.
        observers.append(center.addObserver(
            forName: UIApplication.didBecomeActiveNotification,
            object: nil, queue: nil) { [weak self] _ in
                MainActor.assumeIsolated { self?.set(read()) }
            })
    }

    /// A fixed answer, for tests.
    init(fixed: Bool) {
        available = fixed
    }

    var isAvailable: Bool {
        lock.lock(); defer { lock.unlock() }
        return available
    }

    private func set(_ value: Bool) {
        lock.lock(); available = value; lock.unlock()
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

/// Foundation property-list objects to the store's node list and back
/// (crates/ffi/src/store.rs: node 0 is the value; an array or dictionary
/// names its members by index).
enum PlistNodes {
    static func value(_ object: Any) -> PlistValue? {
        var nodes: [PlistNode] = []
        var failed = false
        @discardableResult func push(_ object: Any) -> UInt32 {
            let index = nodes.count
            nodes.append(.bool(value: false))
            let node: PlistNode
            if let data = object as? Data {
                node = .data(value: data)
            } else if let string = object as? String {
                node = .string(value: string)
            } else if let date = object as? Date {
                node = .date(seconds: date.timeIntervalSinceReferenceDate)
            } else if let array = object as? [Any] {
                node = .array(members: array.map { push($0) })
            } else if let dict = object as? NSDictionary {
                let keys = dict.allKeys.compactMap { $0 as? String }
                node = .dictionary(keys: keys, members: keys.map { push(dict[$0] as Any) })
            } else if let number = object as? NSNumber {
                if CFGetTypeID(number) == CFBooleanGetTypeID() {
                    node = .bool(value: number.boolValue)
                } else if CFNumberIsFloatType(number) {
                    node = .double(value: number.doubleValue)
                } else {
                    node = .int(value: number.int64Value)
                }
            } else {
                failed = true
                node = .bool(value: false)
            }
            nodes[index] = node
            return UInt32(index)
        }
        push(object)
        return failed ? nil : PlistValue(nodes: nodes)
    }

    static func object(_ value: PlistValue) -> Any {
        func build(_ index: Int) -> Any {
            guard value.nodes.indices.contains(index) else { return false }
            switch value.nodes[index] {
            case .bool(let v): return v
            case .int(let v): return NSNumber(value: v)
            case .double(let v): return NSNumber(value: v)
            case .string(let v): return v
            case .data(let v): return v
            case .date(let s): return Date(timeIntervalSinceReferenceDate: s)
            case .array(let members): return members.map { build(Int($0)) }
            case .dictionary(let keys, let members):
                let dict = NSMutableDictionary()
                for (key, member) in zip(keys, members) { dict[key] = build(Int(member)) }
                return dict
            }
        }
        return build(0)
    }
}

// MARK: - Keychain

/// The keychain as the core's ``KeychainHost``: the backend
/// ``KeychainService`` uses, so the sessions' known hosts
/// (`moshpit.knownHosts`) and the secrets keep the protection 1.0.8 gave
/// them.
final class RustKeychainHost: KeychainHost, @unchecked Sendable {
    private let backend: KeychainBackend
    /// The hosts trusted for this launch (``LaunchTrust``) laid over the
    /// known-hosts item: the app's table for the Rust sessions, which decide
    /// inside the core. `nil` reads the item as stored. A test gives its
    /// own table, so nothing it trusts outlives it.
    private let launchTrust: LaunchTrust.Table?

    init(backend: KeychainBackend? = nil, launchTrust: LaunchTrust.Table? = LaunchTrust.shared) {
        self.launchTrust = launchTrust
        if let backend {
            self.backend = backend
        } else {
            #if targetEnvironment(simulator)
            self.backend = FileVaultKeychainBackend(filename: "moshpit-known-hosts.json")
            #else
            self.backend = SystemKeychainBackend()
            #endif
        }
    }

    func save(account: String, secret: Data, requireBiometry: Bool) throws {
        guard let launchTrust else {
            return try map { try backend.save(secret: secret, account: account, requireBiometry: requireBiometry) }
        }
        try map { try backend.save(secret: launchTrust.stripped(secret, account: account),
                                   account: account, requireBiometry: requireBiometry) }
    }

    func load(account: String, prompt: String) throws -> Data {
        guard let launchTrust else { return try map { try backend.load(account: account, prompt: prompt) } }
        do {
            let data = try map { try backend.load(account: account, prompt: prompt) }
            return launchTrust.overlaid(data, account: account)
        } catch KeychainFailure.NotFound where launchTrust.covers(account) {
            return launchTrust.overlaid(Data("{}".utf8), account: account)
        }
    }

    func delete(account: String) throws {
        try map { try backend.delete(account: account) }
    }

    func exists(account: String) -> Bool {
        backend.exists(account: account)
    }

    /// Every item's account, for the store's orphan recovery (B232).
    func accounts() throws -> [String] {
        try map { try backend.accounts() }
    }

    /// An item's bytes without asking anyone, for orphan recovery: a Face
    /// ID item fails `InteractionNotAllowed` rather than prompting.
    func loadSilently(account: String) throws -> Data {
        try map { try backend.loadSilently(account: account) }
    }

    private func map<T>(_ body: () throws -> T) throws -> T {
        do {
            return try body()
        } catch let error as KeychainError {
            switch error {
            case .notFound: throw KeychainFailure.NotFound
            case .duplicate: throw KeychainFailure.Duplicate
            case .authenticationFailed: throw KeychainFailure.AuthenticationFailed
            case .userCanceled: throw KeychainFailure.UserCanceled
            case .interactionNotAllowed: throw KeychainFailure.InteractionNotAllowed
            case .invalidData: throw KeychainFailure.InvalidData
            case .unhandled: throw KeychainFailure.Other(message: error.description)
            }
        } catch {
            throw KeychainFailure.Other(message: String(describing: error))
        }
    }
}

/// Host keys trusted for this launch only (a DEBUG seed's private sshd):
/// laid over the trusted map as the core reads it, and taken out of what
/// it writes back, so nothing about them is ever stored. Empty in Release.
enum LaunchTrust {
    /// A set of hosts trusted for a launch. The app has one, ``shared``,
    /// which the seeds fill; a test makes its own.
    final class Table: @unchecked Sendable {
        private let lock = NSLock()
        private var entries: [String: String] = [:]
        private static let account = "moshpit.knownHosts"

        init() {}

        func trust(host: String, port: Int, fingerprint: String) {
            lock.lock(); entries["\(host):\(port)"] = fingerprint; lock.unlock()
        }

        func fingerprint(host: String, port: Int) -> String? {
            lock.lock(); defer { lock.unlock() }
            return entries["\(host):\(port)"]
        }

        fileprivate func covers(_ account: String) -> Bool {
            lock.lock(); defer { lock.unlock() }
            return account == Self.account && !entries.isEmpty
        }

        fileprivate func overlaid(_ data: Data, account: String) -> Data {
            guard covers(account),
                  var map = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return data }
            lock.lock(); for (key, value) in entries { map[key] = value }; lock.unlock()
            return (try? JSONSerialization.data(withJSONObject: map)) ?? data
        }

        fileprivate func stripped(_ data: Data, account: String) -> Data {
            guard covers(account),
                  var map = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return data }
            lock.lock()
            for (key, value) in entries where map[key] as? String == value { map[key] = nil }
            lock.unlock()
            return (try? JSONSerialization.data(withJSONObject: map)) ?? data
        }
    }

    /// The app's: what the DEBUG seeds trust for this process.
    static let shared = Table()

    static func trust(host: String, port: Int, fingerprint: String) {
        shared.trust(host: host, port: port, fingerprint: fingerprint)
    }

    static func fingerprint(host: String, port: Int) -> String? {
        shared.fingerprint(host: host, port: port)
    }
}

// MARK: - Credentials

/// A connection's credentials for the core, from the keychain through
/// ``SSHService`` — the same read, Face ID and secret cache the Swift
/// connects use (one prompt per foreground for both paths), the same
/// conversion of a key kept as pasted. A Secure Enclave key gives its
/// public half; the core asks it for signatures, which never leave the chip.
///
/// The core names a connection by its id; the session that started it
/// registers the connection (and where to ask a key's passphrase) first.
final class RustCredentialHost: CredentialHost, @unchecked Sendable {
    typealias PassphraseAsker = @Sendable (_ wrong: Bool) async -> String?

    private let lock = NSLock()
    private var connections: [String: (ServerConnection, PassphraseAsker)] = [:]
    /// Hardware keys by keychain ref → their connection (the signer is
    /// asked by ref).
    private var hardware: [String: ServerConnection] = [:]
    private let service: SSHService
    /// Whether the keychain can be read now: a read that fails while the
    /// device is locked (a launch before the first unlock) is "locked", which
    /// the core retries on the unlock — not "unavailable", which it never
    /// retries on its own (B201).
    private let protectedData: () -> Bool

    /// Told each managed key a sign-in reads — its connection, and the key
    /// as read — so a key 1.0.8 imported without its public half shows what
    /// it is from its first sign-in on (B726,
    /// ``SSHKeyStore/learnFacts(for:privateKey:)``): one behind Face ID
    /// can't be read at launch. Set once, at launch.
    nonisolated(unsafe) static var keyRead: (@Sendable (ServerConnection, String) -> Void)?

    init(service: SSHService = .shared, protectedData: @escaping @Sendable () -> Bool = { true }) {
        self.service = service
        self.protectedData = protectedData
    }

    func register(_ connection: ServerConnection, passphrase: @escaping PassphraseAsker) {
        lock.lock(); connections[connection.id.uuidString] = (connection, passphrase); lock.unlock()
    }

    func unregister(_ connection: ServerConnection) {
        lock.lock(); connections[connection.id.uuidString] = nil; lock.unlock()
    }

    func credential(connectionId: String) async throws -> Credential {
        let entry = lock.withLock { connections[connectionId] }
        guard let (connection, passphrase) = entry else {
            throw CredentialFailure.Unavailable(message: "no connection \(connectionId) in this app")
        }
        #if DEBUG
        if let seeded = RustSessionSeed.credential(for: connection.id) {
            return .key(openssh: seeded)
        }
        #endif
        do {
            let resolved = try await service.rustCoreCredential(connection) { _, wrong in
                await passphrase(wrong)
            }
            switch resolved {
            case .password(let password):
                return .password(password: password)
            case .key(let text):
                Self.keyRead?(connection, text)
                return .key(openssh: text)
            case .secureEnclave(let publicKey, let ref):
                lock.withLock { hardware[ref] = connection }
                return .hardware(publicKey: publicKey, keyRef: ref)
            }
        } catch SSHError.keyNeedsPassphrase {
            throw CredentialFailure.Canceled
        } catch SSHError.credentialCancelled {
            // Face ID cancelled or failed (B392): the person's, or the
            // system's when the app left the screen — not a lost credential.
            throw CredentialFailure.Canceled
        } catch SSHError.credentialLocked {
            // The keychain said so itself — also where the flag below reads
            // unlocked: an item behind Face ID, asked for in the background
            // where no prompt can show, is refused the same way. It used to
            // cross as Unavailable, which the core never retries (B211).
            throw CredentialFailure.Locked
        } catch {
            if !protectedData() { throw CredentialFailure.Locked }
            throw CredentialFailure.Unavailable(message: (error as? SSHError)?.diagnostic
                                                ?? String(describing: error))
        }
    }

    func sign(keyRef: String, data: Data) async throws -> PlatformSignature {
        let connection = lock.withLock { hardware[keyRef] }
        guard let connection else {
            throw CredentialFailure.Unavailable(message: "no Secure Enclave key \(keyRef) in use")
        }
        do {
            return .der(bytes: try await service.signForRustCore(connection, data: data))
        } catch {
            throw Self.signFailure(error, protectedDataAvailable: protectedData())
        }
    }

    /// What a Secure Enclave signature that failed crosses to the core as
    /// (B227). Every failure crossed as Unavailable — SignerFailed in the
    /// core, never retried — so a sign-in whose key read the locked device
    /// refused (a reconnect in the background, the device locked mid-
    /// handshake) gave up for good. The lock is `Locked`, retried on the
    /// unlock as a credential read refused for it is (B211): the keychain
    /// said so (``SSHError/credentialLocked``), or anything failed while
    /// protected data is unavailable — the key's item and the enclave's
    /// operations both need the device unlocked.
    nonisolated static func signFailure(_ error: Error, protectedDataAvailable: Bool) -> CredentialFailure {
        if case .credentialLocked? = error as? SSHError { return .Locked }
        // The key's Face ID cancelled (B392).
        if case .credentialCancelled? = error as? SSHError { return .Canceled }
        if !protectedDataAvailable { return .Locked }
        return .Unavailable(message: String(describing: error))
    }
}

// MARK: - Events

/// Where the core's event pump lands: one hop to the main queue per batch,
/// in order (the main queue is FIFO; a `Task` per batch would not be).
final class RustEventSink: SessionEventSink, @unchecked Sendable {
    private let receive: @MainActor ([SessionEvent]) -> Void

    init(receive: @escaping @MainActor ([SessionEvent]) -> Void) {
        self.receive = receive
    }

    func deliver(events: [SessionEvent]) {
        let receive = self.receive
        DispatchQueue.main.async {
            MainActor.assumeIsolated { receive(events) }
        }
    }
}
