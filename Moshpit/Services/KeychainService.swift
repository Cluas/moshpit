import Foundation
import Security
import LocalAuthentication

// MARK: - Errors

enum KeychainError: Error, Equatable, CustomStringConvertible {
    case notFound
    case duplicate
    case authenticationFailed
    case userCanceled
    case interactionNotAllowed
    case invalidData
    case unhandled(OSStatus)

    var description: String {
        switch self {
        case .notFound: return "Keychain item not found"
        case .duplicate: return "Keychain item already exists"
        case .authenticationFailed: return "Biometric authentication failed"
        case .userCanceled: return "User canceled biometric prompt"
        case .interactionNotAllowed: return "Keychain interaction not allowed"
        case .invalidData: return "Keychain data was invalid"
        case .unhandled(let status):
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "OSStatus \(status)"
            return "Keychain error: \(message)"
        }
    }

    init(osStatus: OSStatus) {
        switch osStatus {
        case errSecItemNotFound:
            self = .notFound
        case errSecDuplicateItem:
            self = .duplicate
        case errSecAuthFailed:
            self = .authenticationFailed
        case errSecUserCanceled:
            self = .userCanceled
        case errSecInteractionNotAllowed:
            self = .interactionNotAllowed
        default:
            self = .unhandled(osStatus)
        }
    }
}

// MARK: - Backend protocol (testable)

/// Low-level backend abstraction so tests can inject an in-memory mock instead
/// of touching the real keychain. All methods are synchronous because the
/// underlying `SecItem*` calls are synchronous (LAContext prompts the user
/// from inside the kernel call).
protocol KeychainBackend: Sendable {
    func save(secret: Data, account: String, requireBiometry: Bool) throws
    func load(account: String, prompt: String) throws -> Data
    func delete(account: String) throws
    func exists(account: String) -> Bool
    /// Change what an existing item holds and nothing else — its protection
    /// stays as it was. Throws `.notFound` rather than create an item.
    func replace(secret: Data, account: String, prompt: String) throws
    /// The account of every item this backend holds, without reading one —
    /// no prompt, nothing decrypted. Throws when it can't say (the device is
    /// locked, a backend that can't list): never an empty list for "don't
    /// know".
    func accounts() throws -> [String]
    /// The item's bytes if they can be had without asking anyone: an item
    /// behind Face ID throws (`.interactionNotAllowed`) instead of prompting.
    func loadSilently(account: String) throws -> Data
}

extension KeychainBackend {
    /// A backend that can't change an item in place leaves it alone.
    func replace(secret: Data, account: String, prompt: String) throws {
        throw KeychainError.unhandled(errSecUnimplemented)
    }

    /// A backend that can't list its items says so; nothing is recovered.
    func accounts() throws -> [String] {
        throw KeychainError.unhandled(errSecUnimplemented)
    }

    /// A backend that can't read without asking says so.
    func loadSilently(account: String) throws -> Data {
        throw KeychainError.unhandled(errSecUnimplemented)
    }
}

// MARK: - System keychain backend

/// The `SecItem*` calls ``SystemKeychainBackend`` changes the keychain with.
/// A value rather than direct calls only so a test can stand in for the
/// keychain and make a write fail the way a real one does — the device
/// locking halfway through a save.
struct KeychainWrites: Sendable {
    var add: @Sendable (_ attributes: CFDictionary) -> OSStatus
    var update: @Sendable (_ query: CFDictionary, _ changes: CFDictionary) -> OSStatus
    var delete: @Sendable (_ query: CFDictionary) -> OSStatus

    static let system = KeychainWrites(
        add: { SecItemAdd($0, nil) },
        update: { SecItemUpdate($0, $1) },
        delete: { SecItemDelete($0) })
}

/// Production backend that talks to Apple's Keychain Services.
///
/// Items are stored as Generic Passwords scoped by `service` (defaults to the
/// bundle id). The keychain `account` is the `keychainRef` opaque ID assigned
/// by `KeychainService.generateRef()`. When `requireBiometry` is true the
/// item is gated by `.userPresence`, which prompts Face ID / Touch ID / device
/// passcode on every read.
final class SystemKeychainBackend: KeychainBackend {
    private let service: String
    private let accessGroup: String?
    private let writes: KeychainWrites

    init(service: String = Bundle.main.bundleIdentifier ?? "com.cluas.moshpit",
         accessGroup: String? = nil,
         writes: KeychainWrites = .system) {
        self.service = service
        self.accessGroup = accessGroup
        self.writes = writes
    }

    /// Common attributes for every keychain operation. `kSecUseDataProtectionKeychain`
    /// pins us to the modern data-protection keychain, which:
    ///   - Doesn't require a `keychain-access-groups` entitlement on unsigned
    ///     simulator builds (legacy path fails with -34018 there).
    ///   - Sandboxes items to the app by default (no implicit team-group sharing).
    ///   - Is what Apple recommends for new code on iOS 13+.
    private func baseQuery(account: String) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let accessGroup {
            q[kSecAttrAccessGroup as String] = accessGroup
        }
        return q
    }

    /// Store `secret` under `account`: added when there is no item yet, and
    /// rewritten where it stands when there is one.
    ///
    /// Never delete-then-add. That was the overwrite, and between its two
    /// calls the account held nothing: an add refused there — the device
    /// locking mid-save (a `WhenUnlocked` item can't be written while
    /// locked), any keychain error — left the item gone for good, whatever
    /// it was. The known-hosts table is one item, so that was every
    /// fingerprint the user ever trusted. Now a write that fails leaves the
    /// item exactly as it was.
    ///
    /// Overwriting a biometry-gated item asks for the user's presence, as
    /// the keychain requires for changing one; no caller does that today
    /// (biometry-gated keys are saved under a fresh ref).
    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        let protection = try Self.protection(requireBiometry: requireBiometry)

        var item = baseQuery(account: account)
        item[kSecValueData as String] = secret
        item.merge(protection) { _, new in new }
        let added = writes.add(item as CFDictionary)
        if added == errSecSuccess { return }
        guard added == errSecDuplicateItem else {
            throw KeychainError(osStatus: added)
        }

        // There is one already: replace its contents in place.
        var changes = protection
        changes[kSecValueData as String] = secret
        let updated = writes.update(baseQuery(account: account) as CFDictionary, changes as CFDictionary)
        guard updated == errSecSuccess else {
            throw KeychainError(osStatus: updated)
        }
    }

    /// How an item is protected: gated by `.userPresence`, or plainly
    /// `WhenUnlockedThisDeviceOnly`. Exactly one of the two attributes —
    /// the keychain refuses an item that carries both.
    private static func protection(requireBiometry: Bool) throws -> [String: Any] {
        guard requireBiometry else {
            return [kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        }
        var accessError: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil,
            kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            .userPresence,
            &accessError
        ) else {
            if let err = accessError?.takeRetainedValue() {
                throw err as Error
            }
            throw KeychainError.unhandled(errSecParam)
        }
        return [kSecAttrAccessControl as String: access]
    }

    func load(account: String, prompt: String) throws -> Data {
        let context = LAContext()
        context.localizedReason = prompt

        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        query[kSecUseAuthenticationContext as String] = context

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess else {
            throw KeychainError(osStatus: status)
        }
        guard let data = item as? Data else {
            throw KeychainError.invalidData
        }
        return data
    }

    /// One `SecItemUpdate` of the data alone: the item keeps its protection
    /// (a biometry-gated one asks for the user's presence, with `prompt`),
    /// and either holds the new bytes afterwards or, on any failure, exactly
    /// what it held before.
    func replace(secret: Data, account: String, prompt: String) throws {
        let context = LAContext()
        context.localizedReason = prompt
        var query = baseQuery(account: account)
        query[kSecUseAuthenticationContext as String] = context
        let status = writes.update(query as CFDictionary, [kSecValueData as String: secret] as CFDictionary)
        guard status == errSecSuccess else {
            throw KeychainError(osStatus: status)
        }
    }

    func delete(account: String) throws {
        let query = baseQuery(account: account)
        let status = writes.delete(query as CFDictionary)
        // Treat "not found" as success — delete is idempotent.
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(osStatus: status)
        }
    }

    func exists(account: String) -> Bool {
        var query = baseQuery(account: account)
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        // No UI: a biometry-gated item answers errSecInteractionNotAllowed,
        // which still proves it exists.
        let context = LAContext()
        context.interactionNotAllowed = true
        query[kSecUseAuthenticationContext as String] = context
        let status = SecItemCopyMatching(query as CFDictionary, nil)
        return status == errSecSuccess || status == errSecInteractionNotAllowed
    }

    /// The item's bytes, never with a prompt: an authentication context
    /// that may not interact makes a Face ID item answer
    /// `errSecInteractionNotAllowed` instead of asking.
    func loadSilently(account: String) throws -> Data {
        let context = LAContext()
        context.interactionNotAllowed = true
        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        query[kSecUseAuthenticationContext as String] = context
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess else { throw KeychainError(osStatus: status) }
        guard let data = item as? Data else { throw KeychainError.invalidData }
        return data
    }

    /// Every item of this service, by account: attributes only (what an
    /// item is called, never what it holds), so nothing is decrypted and
    /// nothing prompts — a Face ID item's attributes come back like any
    /// other's. An authentication context that may not interact keeps it
    /// that way whatever the system decides: a query that would need the
    /// user fails, and the caller recovers nothing.
    func accounts() throws -> [String] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecUseDataProtectionKeychain as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
            kSecReturnAttributes as String: true,
        ]
        if let accessGroup {
            query[kSecAttrAccessGroup as String] = accessGroup
        }
        let context = LAContext()
        context.interactionNotAllowed = true
        query[kSecUseAuthenticationContext as String] = context
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess else { throw KeychainError(osStatus: status) }
        guard let items = result as? [[String: Any]] else { throw KeychainError.invalidData }
        return items.compactMap { $0[kSecAttrAccount as String] as? String }
    }
}

// MARK: - File vault backend (simulator development)

/// File-backed keychain replacement used on the iOS Simulator, where the
/// system keychain rejects writes from unsigned builds with
/// `errSecMissingEntitlement (-34018)`.
///
/// The vault is a JSON file in the app's sandboxed Application Support folder
/// (one file per backend), so:
///   - It persists across simulator app launches (unlike the in-memory
///     backend used by unit tests).
///   - It's scoped to the app's sandbox — other simulator apps can't read it.
///   - It does NOT prompt for biometrics — `requireBiometry` is ignored.
///
/// On a real device, code-signed builds use the system keychain instead;
/// this backend is selected at compile time via `#if targetEnvironment(simulator)`.
///
/// A vault file that is there but won't read is never written over. It used
/// to read as an empty vault, and the next save — any secret, for any
/// connection — wrote a one-entry vault over every stored key. Now every
/// call that would write refuses instead, after keeping a copy of the file
/// at ``backupURL``, and a load says the vault is unreadable rather than
/// "not found" (which callers take as "nothing was ever stored").
final class FileVaultKeychainBackend: KeychainBackend, @unchecked Sendable {
    private let lock = NSLock()
    let url: URL

    init(filename: String = "moshpit-vault.json") {
        let fm = FileManager.default
        let supportDir = (try? fm.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )) ?? URL(fileURLWithPath: NSTemporaryDirectory())
        self.url = supportDir.appendingPathComponent(filename)
    }

    /// Where an unreadable vault is copied, untouched.
    var backupURL: URL { url.appendingPathExtension("backup") }

    /// The vault as stored — empty when there is no file yet. Throws when
    /// there is a file that can't be read or decoded; callers hold the lock.
    private func readStore() throws -> [String: Data] {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
            throw KeychainError.unhandled(errSecIO)
        }
        do {
            return try JSONDecoder().decode([String: Data].self, from: data)
        } catch {
            keepCopy(of: data)
            throw KeychainError.invalidData
        }
    }

    /// Copy an undecodable vault aside, once per distinct content, so that
    /// whatever happens to the file next its bytes survive.
    private func keepCopy(of data: Data) {
        if (try? Data(contentsOf: backupURL)) == data { return }
        var target = backupURL
        if FileManager.default.fileExists(atPath: target.path) {
            target = url.appendingPathExtension("backup-\(Int(Date().timeIntervalSince1970))")
        }
        try? data.write(to: target, options: [.atomic, .completeFileProtection])
    }

    private func saveStore(_ store: [String: Data]) throws {
        let data = try JSONEncoder().encode(store)
        try data.write(to: url, options: [.atomic, .completeFileProtection])
    }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        lock.lock(); defer { lock.unlock() }
        var store = try readStore()
        store[account] = secret
        try saveStore(store)
    }

    func load(account: String, prompt: String) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        guard let data = try readStore()[account] else {
            throw KeychainError.notFound
        }
        return data
    }

    func replace(secret: Data, account: String, prompt: String) throws {
        lock.lock(); defer { lock.unlock() }
        var store = try readStore()
        guard store[account] != nil else { throw KeychainError.notFound }
        store[account] = secret
        try saveStore(store)
    }

    func delete(account: String) throws {
        lock.lock(); defer { lock.unlock() }
        var store = try readStore()
        guard store.removeValue(forKey: account) != nil else { return }
        try saveStore(store)
    }

    func exists(account: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return (try? readStore())?[account] != nil
    }

    func accounts() throws -> [String] {
        lock.lock(); defer { lock.unlock() }
        return Array(try readStore().keys)
    }

    func loadSilently(account: String) throws -> Data {
        try load(account: account, prompt: "")
    }
}

// MARK: - In-memory backend (test double)

/// Process-local in-memory backend for unit tests. Does NOT touch the real
/// keychain and does NOT prompt for biometrics.
final class InMemoryKeychainBackend: KeychainBackend, @unchecked Sendable {
    private let lock = NSLock()
    private var store: [String: Data] = [:]

    init(seed: [String: Data] = [:]) {
        self.store = seed
    }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        lock.lock(); defer { lock.unlock() }
        store[account] = secret
    }

    func load(account: String, prompt: String) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        guard let data = store[account] else {
            throw KeychainError.notFound
        }
        return data
    }

    func delete(account: String) throws {
        lock.lock(); defer { lock.unlock() }
        store.removeValue(forKey: account)
    }

    func exists(account: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return store[account] != nil
    }

    func replace(secret: Data, account: String, prompt: String) throws {
        lock.lock(); defer { lock.unlock() }
        guard store[account] != nil else { throw KeychainError.notFound }
        store[account] = secret
    }

    func accounts() throws -> [String] {
        lock.lock(); defer { lock.unlock() }
        return Array(store.keys)
    }

    func loadSilently(account: String) throws -> Data {
        try load(account: account, prompt: "")
    }
}

// MARK: - Public service

/// Stores per-connection secrets (password OR private key PEM blob) in the
/// Keychain, gated by biometric authentication on read.
///
/// `ServerConnection.keychainRef` holds the opaque ID returned by
/// `generateRef()`; the actual secret bytes never leave the keychain except
/// inside `load(...)`. The service is an `actor` so concurrent SSH attempts
/// against the same account serialize cleanly.
actor KeychainService {
    private let backend: KeychainBackend

    init(backend: KeychainBackend? = nil) {
        if let backend {
            self.backend = backend
        } else {
            self.backend = Self.defaultBackend()
        }
    }

    /// On a real iOS device we use the system keychain (`SystemKeychainBackend`).
    /// On the Simulator, unsigned debug builds get `errSecMissingEntitlement`
    /// from every keychain call because there is no team-prefixed access group
    /// — so we fall back to a file-backed sandbox vault that still persists
    /// across launches.
    private static func defaultBackend() -> KeychainBackend {
        #if targetEnvironment(simulator)
        return FileVaultKeychainBackend()
        #else
        return SystemKeychainBackend()
        #endif
    }

    /// Convenience initializer for tests: use the in-memory backend.
    static func inMemory(seed: [String: Data] = [:]) -> KeychainService {
        KeychainService(backend: InMemoryKeychainBackend(seed: seed))
    }

    // MARK: - References

    /// What every ref ``generateRef()`` makes starts with — the only items
    /// ``KeychainReferences`` ever looks at on its own.
    static let refPrefix = "moshpit.secret."

    /// Returns a fresh opaque ref that callers should store in
    /// `ServerConnection.keychainRef`. UUID-based, lowercase.
    ///
    /// Remembered for the life of the process: an item is written under a
    /// fresh ref BEFORE the record that names it is saved (a key's private
    /// half, a connection's password), so for a moment nothing references
    /// it. Nothing made in this launch is ever taken for an orphan here.
    nonisolated func generateRef() -> String {
        let ref = Self.refPrefix + UUID().uuidString.lowercased()
        Self.minted.withLock { _ = $0.insert(ref) }
        return ref
    }

    /// Whether `ref` was made by ``generateRef()`` in this process.
    static func mintedThisLaunch(_ ref: String) -> Bool {
        minted.withLock { $0.contains(ref) }
    }

    private static let minted = MintedRefs()

    /// The refs this process made, behind a lock: refs are made on any
    /// thread.
    private final class MintedRefs: @unchecked Sendable {
        private let lock = NSLock()
        private var refs: Set<String> = []

        func withLock<T>(_ body: (inout Set<String>) -> T) -> T {
            lock.lock(); defer { lock.unlock() }
            return body(&refs)
        }
    }

    // MARK: - Password

    func savePassword(_ password: String, forRef ref: String, requireBiometry: Bool = true) throws {
        guard let data = password.data(using: .utf8) else {
            throw KeychainError.invalidData
        }
        try backend.save(secret: data, account: ref, requireBiometry: requireBiometry)
    }

    func loadPassword(forRef ref: String,
                      reason: String = "Authenticate to use SSH connection") throws -> String {
        let data = try backend.load(account: ref, prompt: reason)
        guard let string = String(data: data, encoding: .utf8) else {
            throw KeychainError.invalidData
        }
        return string
    }

    // MARK: - Private key

    /// Saves a PEM-encoded private key (e.g. an `-----BEGIN OPENSSH PRIVATE KEY-----` blob).
    func savePrivateKey(_ pem: String, forRef ref: String, requireBiometry: Bool = true) throws {
        guard let data = pem.data(using: .utf8) else {
            throw KeychainError.invalidData
        }
        try backend.save(secret: data, account: ref, requireBiometry: requireBiometry)
    }

    func loadPrivateKey(forRef ref: String,
                        reason: String = "Authenticate to use SSH private key") throws -> String {
        let data = try backend.load(account: ref, prompt: reason)
        guard let pem = String(data: data, encoding: .utf8) else {
            throw KeychainError.invalidData
        }
        return pem
    }

    /// Put `pem` in place of the private key already stored at `ref`,
    /// keeping the item's protection (Face ID or not) as it was. Throws, and
    /// leaves the item untouched, when there is no item or it can't be
    /// changed.
    func replacePrivateKey(_ pem: String, forRef ref: String,
                           reason: String = "Authenticate to update the SSH private key") throws {
        guard let data = pem.data(using: .utf8) else {
            throw KeychainError.invalidData
        }
        try backend.replace(secret: data, account: ref, prompt: reason)
    }

    // MARK: - Raw

    func saveSecret(_ data: Data, forRef ref: String, requireBiometry: Bool = true) throws {
        try backend.save(secret: data, account: ref, requireBiometry: requireBiometry)
    }

    func loadSecret(forRef ref: String,
                    reason: String = "Authenticate to access stored secret") throws -> Data {
        try backend.load(account: ref, prompt: reason)
    }

    // MARK: - Delete

    func delete(forRef ref: String) throws {
        try backend.delete(account: ref)
    }

    func contains(ref: String) -> Bool {
        backend.exists(account: ref)
    }

    /// Every item's account (``KeychainBackend/accounts()``): what orphan
    /// recovery looks through. Throws when the backend can't say.
    func accounts() throws -> [String] {
        try backend.accounts()
    }

    /// An item's bytes without asking anyone
    /// (``KeychainBackend/loadSilently(account:)``).
    func loadSecretSilently(forRef ref: String) throws -> Data {
        try backend.loadSilently(account: ref)
    }
}
