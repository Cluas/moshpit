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
}

// MARK: - System keychain backend

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

    init(service: String = Bundle.main.bundleIdentifier ?? "com.cluas.moshpit",
         accessGroup: String? = nil) {
        self.service = service
        self.accessGroup = accessGroup
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

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        // Always delete first so save is idempotent (overwrite semantics).
        try? delete(account: account)

        var query = baseQuery(account: account)
        query[kSecValueData as String] = secret

        if requireBiometry {
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
            query[kSecAttrAccessControl as String] = access
        } else {
            query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        }

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw KeychainError(osStatus: status)
        }
    }

    func load(account: String, prompt: String) throws -> Data {
        let context = LAContext()
        context.localizedReason = prompt

        var query = baseQuery(account: account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        query[kSecUseAuthenticationContext as String] = context
        query[kSecUseOperationPrompt as String] = prompt

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

    func delete(account: String) throws {
        let query = baseQuery(account: account)
        let status = SecItemDelete(query as CFDictionary)
        // Treat "not found" as success — delete is idempotent.
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(osStatus: status)
        }
    }

    func exists(account: String) -> Bool {
        var query = baseQuery(account: account)
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        query[kSecUseAuthenticationUI as String] = kSecUseAuthenticationUIFail
        let status = SecItemCopyMatching(query as CFDictionary, nil)
        return status == errSecSuccess || status == errSecInteractionNotAllowed
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
final class FileVaultKeychainBackend: KeychainBackend, @unchecked Sendable {
    private let lock = NSLock()
    private let url: URL

    init(filename: String = "mosaic-vault.json") {
        let fm = FileManager.default
        let supportDir = (try? fm.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )) ?? URL(fileURLWithPath: NSTemporaryDirectory())
        self.url = supportDir.appendingPathComponent(filename)
    }

    private func loadStore() -> [String: Data] {
        lock.lock(); defer { lock.unlock() }
        guard
            let data = try? Data(contentsOf: url),
            let decoded = try? JSONDecoder().decode([String: Data].self, from: data)
        else { return [:] }
        return decoded
    }

    private func saveStore(_ store: [String: Data]) throws {
        let data = try JSONEncoder().encode(store)
        try data.write(to: url, options: [.atomic, .completeFileProtection])
    }

    func save(secret: Data, account: String, requireBiometry: Bool) throws {
        var store = loadStore()
        store[account] = secret
        try saveStore(store)
    }

    func load(account: String, prompt: String) throws -> Data {
        let store = loadStore()
        guard let data = store[account] else {
            throw KeychainError.notFound
        }
        return data
    }

    func delete(account: String) throws {
        var store = loadStore()
        store.removeValue(forKey: account)
        try saveStore(store)
    }

    func exists(account: String) -> Bool {
        loadStore()[account] != nil
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

    /// Returns a fresh opaque ref that callers should store in
    /// `ServerConnection.keychainRef`. UUID-based, lowercase.
    nonisolated func generateRef() -> String {
        "moshpit.secret." + UUID().uuidString.lowercased()
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
}
