import Foundation
import UIKit
import CryptoKit
import MoshpitKit

// MARK: - Errors

/// What went wrong reaching a host, in the person's words: the sign-in's own
/// failures (the saved credential, Face ID, the lock), and the catalog the
/// Rust session's failures read from (``RustCopy``), one sentence for one
/// situation whichever layer met it.
enum SSHError: Error, CustomStringConvertible, LocalizedError {
    case missingKeychainRef
    case sessionClosed
    case authenticationFailed
    case authMethodRejected
    case channelOpenFailed
    case connectionFailed
    case handshakeFailed
    /// The saved password or key isn't there, or can't be read, with the
    /// device unlocked: lost to a re-install or a re-sign, damaged. Waits
    /// for the person (``credentialRead(failedWith:protectedDataAvailable:)``).
    case credentialUnavailable
    /// Face ID (or the passcode) for the saved password or key was
    /// cancelled — by the person, or by the system when the app left the
    /// screen or a call came in — or failed too many times (B392). The
    /// credential is fine: it used to read as ``credentialUnavailable``,
    /// which told people to re-enter it after a re-install. Not tried again
    /// on its own (Face ID asked in a loop is worse); connecting again asks
    /// once more.
    case credentialCancelled
    /// The saved password or key can't be read YET: the device is locked
    /// (B211). The keychain refused (`errSecInteractionNotAllowed`), or
    /// answered while protected data was unavailable — before the first
    /// unlock it refuses or reads empty, and neither says anything about the
    /// item. Retried when the device unlocks and on the foreground return,
    /// as crates/session's `CredentialsLocked` is; never "re-enter the
    /// password".
    case credentialLocked
    /// The trusted host keys can't be read while the device is locked, so
    /// this server's key could not be checked.
    case hostKeysLocked
    /// The user said no to the server's key: the new-host card's Cancel,
    /// the changed-key card's Disconnect. Their own decision, so the
    /// terminal ends the connect quietly (B151).
    case hostKeyRejected
    /// The SOCKS proxy ("host:port") couldn't be reached.
    case proxyUnreachable(String)
    /// The SOCKS proxy took the connection but didn't open one to the server.
    case proxyFailed
    /// The saved key is behind a passphrase, and none was given.
    case keyNeedsPassphrase
    /// The saved key can't sign in in any form: the kind of key, or a
    /// damaged file (`StoredKey`).
    case keyUnusable(SSHKeyImportError)
    case underlying(Error)

    /// NSError bridging shows "SSHError error 1." unless we hand Foundation
    /// the sentence ourselves — which is exactly what an upload-failure
    /// banner displayed to a user (2026-08-17). Any code path that shows
    /// `localizedDescription` now gets `description` for free.
    var errorDescription: String? { description }

    /// The raw error text, for logs and bug reports. Never shown in the UI —
    /// `description` is what a user sees, and it is a sentence.
    var diagnostic: String {
        if case .underlying(let err) = self {
            return "SSHError.underlying: \(String(describing: err))"
        }
        return "SSHError.\(self)"
    }

    var description: String {
        switch self {
        case .missingKeychainRef:
            return String(localized: "Connection has no associated keychain reference")
        case .sessionClosed:
            return String(localized: "SSH session is closed")
        case .authenticationFailed:
            return String(localized: "Authentication failed — the server rejected your username and password / key. Double-check your credentials.")
        case .authMethodRejected:
            return String(localized: "The server rejected this sign-in method. Try a different one (e.g. a key instead of a password).")
        case .channelOpenFailed:
            return String(localized: "Connected, but the server wouldn't open a session channel.")
        case .connectionFailed:
            return String(localized: "Couldn't reach the server. Check the host, port, and that it's online.")
        case .handshakeFailed:
            return String(localized: "The server closed the connection while setting up SSH. Check that the port really is an SSH server, and that a firewall or proxy isn't cutting the connection.")
        case .credentialUnavailable:
            return String(localized: "Couldn't read the saved credential. After re-installing or re-signing the app (e.g. via SideStore), open Edit and re-enter your password / re-select your key.")
        case .credentialCancelled:
            // The Rust session's words for its `Cancelled` too (RustCopy):
            // Face ID's cancel and the passphrase card's.
            return String(localized: "Signing in was cancelled, so Moshpit didn't connect. Connect again to try once more.")
        case .credentialLocked:
            // The Rust session's words for its `CredentialsLocked` too
            // (RustCopy): one catalog entry for one situation.
            return String(localized: "This device is locked, so Moshpit can't read the saved password or key yet. It connects again once you unlock it.")
        case .hostKeysLocked:
            return String(localized: "While this device is locked, Moshpit can't read the server keys you've trusted, so it can't check this server. Unlock it and connect again.")
        case .hostKeyRejected:
            return String(localized: "You didn't trust the server's key, so Moshpit didn't connect.")
        case .proxyUnreachable(let proxy):
            return String(localized: "Couldn't reach the SOCKS proxy at \(proxy). Check that it's running, and its host and port.")
        case .proxyFailed:
            return String(localized: "The SOCKS proxy didn't connect you to the server. Check that it's a SOCKS5 proxy that needs no password, and that it can reach the server.")
        case .keyNeedsPassphrase:
            return String(localized: "This key is locked with a passphrase. Connect again and enter it — Moshpit asks for it only once.")
        case .keyUnusable(let reason):
            switch reason {
            case .unsupportedType, .securityKey, .unsupportedEncryption:
                return reason.errorDescription ?? ""
            default:
                return String(localized: "The private key saved for this connection is damaged, or isn't one Moshpit can read. Import or paste the key again.")
            }
        case .underlying:
            // Deliberately says nothing about the wrapped error: a library
            // type that doesn't implement LocalizedError renders as "The
            // operation couldn't be completed. (… error 1.)", pointer
            // addresses and all, on a product whose troubleshooting page
            // promises "plain language, not stderr". `diagnostic` keeps the
            // detail for logs.
            return String(localized: "The connection failed for a reason Moshpit didn't recognise. Check the host, port and that the server is reachable, then try again.")
        }
    }

    /// What a failed keychain read of a connection's saved credential means
    /// (B211). It used to be ``credentialUnavailable`` whatever the keychain
    /// said, and B163 rightly never retries that: a reconnect made while the
    /// phone was locked — in the background, from a notification on the
    /// lock screen, a launch before the first unlock — gave up for good, and
    /// told the person to re-enter a password that was there all along.
    ///
    /// The device being locked is ``credentialLocked``: the keychain
    /// refusing (`errSecInteractionNotAllowed` — an item kept `WhenUnlocked`,
    /// or one behind Face ID asked for where no prompt can show), or any
    /// failure while protected data is unavailable, when the keychain
    /// refuses or reads empty and an untrusted read is never taken for the
    /// truth (the sealed-launch rule, ``DefaultsReadiness``). With the
    /// device unlocked, Face ID cancelled or failed is ``credentialCancelled``
    /// (B392); anything else is the item's own — gone, unreadable — and
    /// ``credentialUnavailable``.
    static func credentialRead(failedWith error: Error, protectedDataAvailable: Bool) -> SSHError {
        if case .interactionNotAllowed? = error as? KeychainError { return .credentialLocked }
        guard protectedDataAvailable else { return .credentialLocked }
        switch error as? KeychainError {
        case .userCanceled?, .authenticationFailed?: return .credentialCancelled
        default: return .credentialUnavailable
        }
    }
}

// MARK: - SSHService

/// A connection's saved credential, for the Rust session's sign-in
/// (``rustCoreCredential(_:onKeyPassphrase:)``, ``signForRustCore(_:data:)``):
/// the keychain read behind Face ID, kept in memory for the foreground so a
/// reconnect doesn't ask again, and a key kept as it was pasted made
/// signable once.
actor SSHService {
    /// Asks the user for the passphrase of the key `connection` signs in
    /// with; `wrong` when the last one given didn't unlock it. nil = cancel.
    typealias KeyPassphraseHandler = @Sendable (_ connection: ServerConnection, _ wrong: Bool) async -> String?

    private let keychain: KeychainService
    /// Whether the device's protected data is available — asked after a
    /// keychain read failed, to tell the lock from the item
    /// (``SSHError/credentialRead(failedWith:protectedDataAvailable:)``).
    private let protectedDataAvailable: @Sendable () async -> Bool

    /// `UIApplication.isProtectedDataAvailable`, asked on the main actor
    /// where UIKit answers it: awaited, never waited on, so a main thread
    /// that is itself waiting on this service can't hold the answer up.
    static let systemProtectedData: @Sendable () async -> Bool = {
        await MainActor.run { UIApplication.shared.isProtectedDataAvailable }
    }

    /// In-memory credential cache, keyed by connection id. Populated after the
    /// first successful (biometry-gated) keychain read so the now-frequent
    /// auto-reconnects don't re-prompt Face ID on every attempt — and don't
    /// surface a keychain error mid-reconnect. Plaintext lives only in memory
    /// for the app session; cleared on intentional disconnect AND whenever the
    /// app backgrounds (see ``clearAllCachedSecrets()``).
    private var secretCache: [UUID: String] = [:]

    #if DEBUG
    /// DEBUG seeds' keys (``seedSecretForThisLaunch(connectionId:secret:)``),
    /// read before ``secretCache`` and never in the keychain. Unlike the
    /// cache they outlive a background: nothing guards them, and a seeded
    /// session that reconnected after one had no key left (B200).
    private var launchSecrets: [UUID: String] = [:]
    #endif
    private var seBlobCache: [UUID: Data] = [:]

    /// Keychain reads currently in flight, keyed by connection id, so
    /// concurrent resolutions of the same connection share ONE read — and
    /// therefore one Face ID prompt. This actor is reentrant: without this,
    /// two sign-ins racing for the same connection both see an empty
    /// `secretCache`, both suspend into the keychain, and the user gets two
    /// biometry sheets for one tap — the core redials every session on the
    /// way back at once.
    private var secretResolutions: [UUID: Task<String, Error>] = [:]

    /// Forget a cached credential (called when the user disconnects), so the
    /// next sign-in reads the keychain again.
    func clearCachedSecret(for id: UUID) {
        secretCache[id] = nil
        seBlobCache[id] = nil
        // Detach (don't cancel) any in-flight read: sign-ins already awaiting
        // it still get their value, but its result no longer lands in the
        // cache — see `resolveSecret`'s identity check.
        secretResolutions[id] = nil
    }

    /// Forget every cached credential. Called when the app backgrounds (see
    /// `SessionHub.setForeground(false)`) so the keychain's `.userPresence`
    /// "authenticate on every read" guarantee isn't silently downgraded to
    /// "once per app session" — without this, a decrypted password/PEM read
    /// once via Face ID would linger in process memory and get reused across
    /// backgroundings, with no re-authentication required on return. Clearing
    /// here only affects re-foreground: reconnects that happen while the app
    /// STAYS foregrounded still hit `secretCache`/`seBlobCache` and never
    /// re-prompt, exactly as before.
    func clearAllCachedSecrets() {
        secretCache.removeAll()
        seBlobCache.removeAll()
        // Same detach as `clearCachedSecret`: a read that finishes AFTER this
        // wipe must not repopulate the cache, or the background-wipe guarantee
        // ("authenticate on every read" survives backgrounding) is quietly
        // undone by whichever reconnect happened to be mid-keychain.
        secretResolutions.removeAll()
    }

    init(keychain: KeychainService,
         protectedDataAvailable: @escaping @Sendable () async -> Bool = SSHService.systemProtectedData) {
        self.keychain = keychain
        self.protectedDataAvailable = protectedDataAvailable
    }

    /// ``SSHError/credentialRead(failedWith:protectedDataAvailable:)`` for a
    /// read that just failed with `error`.
    private func credentialReadFailure(_ error: Error) async -> SSHError {
        let failure = SSHError.credentialRead(failedWith: error,
                                              protectedDataAvailable: await protectedDataAvailable())
        if case .credentialLocked = failure {
            Log.ssh.notice("keychain: the saved credential can't be read while the device is locked (\(String(describing: error), privacy: .public)) — retried on the unlock")
        }
        return failure
    }

    // MARK: - Helpers

    /// The key `stored` as the sign-in signs with it (``StoredKey``).
    ///
    /// A key kept as it was pasted — PEM, behind a passphrase, Windows line
    /// endings — is converted on its first use, asking for the passphrase
    /// first when it has one, and once the converted key has read back to
    /// the same public key it takes the original's place in the keychain.
    /// Until that replacement has succeeded the stored item is left as it
    /// was: no step of this can lose the key. A replacement that fails costs
    /// nothing but doing this again next time.
    private func signableKey(_ stored: String, for connection: ServerConnection,
                             onPassphrase: KeyPassphraseHandler) async throws -> String {
        var assessment: StoredKey.Assessment
        do {
            assessment = try StoredKey.assess(stored)
        } catch let error as SSHKeyImportError {
            throw SSHError.keyUnusable(error)
        }
        var wrong = false
        while true {
            switch assessment {
            case .usable:
                return stored
            case .converted(let converted):
                await keepConverted(converted, replacing: stored, for: connection)
                return converted
            case .needsPassphrase:
                guard let passphrase = await onPassphrase(connection, wrong), !passphrase.isEmpty else {
                    throw SSHError.keyNeedsPassphrase
                }
                do {
                    assessment = try StoredKey.assess(stored, passphrase: passphrase)
                } catch SSHKeyImportError.wrongPassphrase {
                    wrong = true
                } catch let error as SSHKeyImportError {
                    throw SSHError.keyUnusable(error)
                }
            }
        }
    }

    /// Store `converted` in place of `stored`, the key it was made from, and
    /// cache it: a sign-in while this foreground lasts (a side connection's,
    /// a reconnect) then neither converts again nor asks again.
    private func keepConverted(_ converted: String, replacing stored: String,
                               for connection: ServerConnection) async {
        if secretCache[connection.id] == stored {
            secretCache[connection.id] = converted
        }
        guard let ref = connection.keychainRef else { return }
        do {
            try await keychain.replacePrivateKey(
                converted, forRef: ref,
                reason: "Authenticate to keep the SSH key for \(connection.host) ready to use")
            Log.ssh.info("stored key converted to openssh-key-v1 for \(connection.host, privacy: .private)")
        } catch {
            Log.ssh.error("stored key: converted for this connection, but the keychain kept the original (\(String(describing: error), privacy: .public))")
        }
    }

    /// `connection`'s saved password or key: a DEBUG seed's, the cache's, a
    /// read already in flight, or a keychain read of its own.
    private func resolveSecret(for connection: ServerConnection) async throws -> String {
        #if DEBUG
        if let seeded = launchSecrets[connection.id] { return seeded }
        #endif
        if let cached = secretCache[connection.id] { return cached }
        // Join a read already in flight instead of starting a second one —
        // this is what keeps one user action at one Face ID prompt when two
        // sign-ins race for the same connection (see `secretResolutions`).
        if let inFlight = secretResolutions[connection.id] {
            return try await inFlight.value
        }
        guard let ref = connection.keychainRef else {
            throw SSHError.missingKeychainRef
        }
        let keychain = self.keychain
        let task = Task<String, Error> {
            do {
                switch connection.authMethod {
                case .password:
                    return try await keychain.loadPassword(forRef: ref,
                                                           reason: "Authenticate to connect to \(connection.host)")
                case .key:
                    return try await keychain.loadPrivateKey(forRef: ref,
                                                             reason: "Authenticate to use SSH key for \(connection.host)")
                }
            } catch let error as KeychainError {
                // The device locked (retried on the unlock), or the item
                // missing / unreadable — often after a re-sign — which only
                // the person can fix (B211).
                throw await self.credentialReadFailure(error)
            }
        }
        secretResolutions[connection.id] = task
        do {
            let secret = try await task.value
            // Cache only if this read is still the registered one. A background
            // wipe (`clearAllCachedSecrets`) between start and finish detaches
            // it, and a late write here would silently undo the wipe.
            if secretResolutions[connection.id] == task {
                secretResolutions[connection.id] = nil
                secretCache[connection.id] = secret
            }
            return secret
        } catch {
            // A failed read must not pin the failure: the next attempt gets a
            // fresh keychain call (and a fresh prompt) rather than this error.
            if secretResolutions[connection.id] == task {
                secretResolutions[connection.id] = nil
            }
            throw error
        }
    }
}

// MARK: - Shared instance

extension SSHService {
    /// The process-wide service, over the real keychain. Tests construct
    /// their own ``SSHService`` with mocks rather than touching this one.
    static let shared = SSHService(keychain: KeychainService())
}

// MARK: - Exec-channel uploads

/// The no-SFTP upload fallback's chunking (``BorrowedHost``): raw bytes per
/// exec round trip. 48KB of payload is 64KB of base64, comfortably inside
/// every sshd/shell argument limit encountered in the wild, while keeping a
/// 2MB image to ~45 round trips. The commands themselves are the core's.
enum ExecUploadCommands {
    static let chunkBytes = 48 * 1024
}

// MARK: - The Rust core's sign-in (W14a)

/// What a connection signs in with, for the Rust core's session layer
/// (`RustCredentialHost`).
enum RustCoreCredential: Equatable {
    case password(String)
    /// An unencrypted openssh-key-v1 private key, as text.
    case key(String)
    /// A Secure Enclave key: its public half as an `authorized_keys` line and
    /// its keychain ref. It signs through ``SSHService/signForRustCore(_:data:)``.
    case secureEnclave(publicKey: String, keyRef: String)
}

extension SSHService {
    /// The credential the Rust core signs in to `connection` with: the
    /// keychain read — Face ID when the item asks for it, then the in-memory
    /// cache (one prompt per foreground) — and the conversion of a key kept
    /// as it was pasted, its passphrase asked through `onKeyPassphrase`. A Secure Enclave key gives
    /// its public half; its signatures come from the chip, never its key.
    func rustCoreCredential(_ connection: ServerConnection,
                            onKeyPassphrase: @escaping KeyPassphraseHandler) async throws -> RustCoreCredential {
        if connection.authMethod == .key, connection.sshKeyAlgorithm == .seP256 {
            guard let ref = connection.keychainRef else { throw SSHError.missingKeychainRef }
            let key = try await secureEnclaveKey(for: connection, ref: ref)
            return .secureEnclave(publicKey: Self.openSSHLine(of: key.publicKey), keyRef: ref)
        }
        let secret = try await resolveSecret(for: connection)
        switch connection.authMethod {
        case .password:
            return .password(secret)
        case .key:
            return .key(try await signableKey(secret, for: connection,
                                              onPassphrase: onKeyPassphrase))
        }
    }

    /// `data` signed by `connection`'s Secure Enclave key (ECDSA P-256 over
    /// SHA-256), DER — what the Rust core's signer callback hands back.
    func signForRustCore(_ connection: ServerConnection, data: Data) async throws -> Data {
        guard let ref = connection.keychainRef else { throw SSHError.missingKeychainRef }
        let key = try await secureEnclaveKey(for: connection, ref: ref)
        do {
            return try key.signature(for: data).derRepresentation
        } catch {
            throw await secureEnclaveFailure(error)
        }
    }

    /// A Secure Enclave operation that failed (B227): while protected data
    /// is unavailable — the device locked mid-handshake, a reconnect in the
    /// background — the chip refuses a key made `WhenUnlocked`, and that is
    /// the lock (``SSHError/credentialLocked``, retried on the unlock as a
    /// keychain read refused for it is, B211), not a key that is gone.
    private func secureEnclaveFailure(_ error: Error) async -> SSHError {
        if !(await protectedDataAvailable()) {
            Log.ssh.notice("secure enclave: refused while the device is locked (\(String(describing: error), privacy: .public)) — retried on the unlock")
            return .credentialLocked
        }
        return .underlying(error)
    }

    /// The enclave key behind `ref`, from the per-connection cache or the
    /// keychain (Face ID when the item asks).
    private func secureEnclaveKey(for connection: ServerConnection, ref: String) async throws
        -> SecureEnclave.P256.Signing.PrivateKey {
        let blob: Data
        if let cached = seBlobCache[connection.id] {
            blob = cached
        } else {
            do {
                blob = try await keychain.loadSecret(
                    forRef: ref,
                    reason: "Authenticate to use the Secure Enclave key for \(connection.host)")
            } catch {
                throw await credentialReadFailure(error)
            }
            seBlobCache[connection.id] = blob
        }
        do {
            return try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
        } catch {
            throw await secureEnclaveFailure(error)
        }
    }

    /// `ecdsa-sha2-nistp256 AAAA…`: the key's SSH wire blob, as an
    /// `authorized_keys` line without a comment (the key reader's,
    /// `moshpit_ssh::keyfile::hardware_public_key`). A point CryptoKit
    /// made is always one it takes.
    nonisolated static func openSSHLine(of key: P256.Signing.PublicKey) -> String {
        (try? sshKeyHardwarePublicKey(publicKey: key.x963Representation, comment: ""))?.publicKeyLine ?? ""
    }

    #if DEBUG
    /// A DEBUG seed's key, for this launch only: what a connect reads first,
    /// never in the keychain (`RustSessionSeed`, `HomeSeed`, the smoke seed).
    func seedSecretForThisLaunch(connectionId: UUID, secret: String) {
        launchSecrets[connectionId] = secret
    }
    #endif
}
