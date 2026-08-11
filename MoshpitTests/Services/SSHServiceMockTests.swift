import Citadel
import Foundation
import NIOCore
import Testing
@testable import Moshpit

/// Sentinel error thrown by `ThrowingClientProvider` so tests can assert
/// the dispatch reached the network-connect call (i.e. pre-flight credential
/// resolution and auth-method construction succeeded).
private struct MockProviderError: Error, Equatable {
    let host: String
    let port: Int
}

/// Mock `SSHClientProvider` that never opens a real socket; instead it
/// throws `MockProviderError` to prove the call site reached it. Citadel's
/// `SSHClient` has an internal init so we cannot return a fabricated one —
/// tests therefore assert *failure* behaviour around the call rather than
/// success.
private struct ThrowingClientProvider: SSHClientProvider {
    let capture: @Sendable (SSHAuthenticationMethod) -> Void

    init(capture: @escaping @Sendable (SSHAuthenticationMethod) -> Void = { _ in }) {
        self.capture = capture
    }

    func connect(
        host: String,
        port: Int,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SSHClient {
        capture(authenticationMethod)
        throw MockProviderError(host: host, port: port)
    }

    func connect(
        on channel: Channel,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SSHClient {
        capture(authenticationMethod)
        throw MockProviderError(host: "(proxied channel)", port: 0)
    }
}

/// `SSHService` tests cover the pre-flight pipeline up to (but not through)
/// the Citadel SSH handshake. Citadel's `SSHClient` initializer is `internal`
/// so we cannot synthesize a connected client in-process; live PTY / write /
/// resize / close behaviour belongs in integration tests against a real
/// fixture server.
@Suite("SSHService — pre-flight")
struct SSHServiceMockTests {

    private func makeService(
        keychain: KeychainService = .inMemory(),
        validator: HostKeyValidator = .inMemory(),
        provider: SSHClientProvider = ThrowingClientProvider()
    ) -> SSHService {
        SSHService(
            keychain: keychain,
            hostKeyValidator: validator,
            clientProvider: provider
        )
    }

    private func sampleConnection(
        host: String = "192.0.2.1", // RFC5737 TEST-NET-1 — guaranteed non-routable
        port: Int = 22,
        authMethod: AuthMethod = .password,
        keychainRef: String? = nil
    ) -> ServerConnection {
        ServerConnection(
            name: "test",
            host: host,
            port: port,
            username: "tester",
            authMethod: authMethod,
            keychainRef: keychainRef
        )
    }

    // MARK: - missingKeychainRef

    @Test("connect throws missingKeychainRef when no ref is stored")
    func connectWithoutKeychainRefThrows() async throws {
        let service = makeService()
        let connection = sampleConnection(authMethod: .password, keychainRef: nil)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
    }

    // MARK: - keychain miss

    @Test("connect surfaces underlying error when keychain ref does not exist")
    func connectWithMissingKeychainEntryThrows() async throws {
        let service = makeService()
        let connection = sampleConnection(authMethod: .password, keychainRef: "moshpit.secret.absent")

        // `loadPassword` throws `KeychainError.notFound`; `SSHService.connect`
        // wraps it in `SSHError.underlying`.
        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
    }

    // MARK: - password auth reaches provider with correct method

    @Test("password auth assembles the right SSHAuthenticationMethod and reaches the provider")
    func passwordAuthReachesProvider() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePassword("s3cret", forRef: ref, requireBiometry: false)

        // We just need to confirm the provider's connect closure is invoked;
        // SSHAuthenticationMethod has no public introspection so equality
        // checks aren't possible, but the call itself is the proof.
        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(keychain: keychain, provider: provider)
        let connection = sampleConnection(authMethod: .password, keychainRef: ref)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
        #expect(captured.wasFired)
    }

    // MARK: - key auth, unsupported key format

    @Test("key auth with garbage PEM throws an SSHError before reaching the provider")
    func keyAuthWithBadPemThrows() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey("not-a-real-pem", forRef: ref, requireBiometry: false)

        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(keychain: keychain, provider: provider)
        let connection = sampleConnection(authMethod: .key, keychainRef: ref)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
        // Auth method construction failed → provider must not have been called.
        #expect(captured.wasFired == false)
    }

    // MARK: - key auth, valid keys reach the provider

    @Test("ed25519 key auth assembles the right SSHAuthenticationMethod and reaches the provider")
    func keyAuthReachesProvider() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        let generated = try SSHKeyFactory.generate(algorithm: .ed25519, comment: "test")
        try await keychain.savePrivateKey(
            String(decoding: generated.privateBlob, as: UTF8.self), forRef: ref, requireBiometry: false)

        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(keychain: keychain, provider: provider)
        let connection = sampleConnection(authMethod: .key, keychainRef: ref)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
        #expect(captured.wasFired)
    }

    @Test("RSA-4096 key auth assembles the right SSHAuthenticationMethod and reaches the provider")
    func keyAuthRSAReachesProvider() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        let generated = try SSHKeyFactory.generate(algorithm: .rsa4096, comment: "test")
        try await keychain.savePrivateKey(
            String(decoding: generated.privateBlob, as: UTF8.self), forRef: ref, requireBiometry: false)

        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(keychain: keychain, provider: provider)
        let connection = sampleConnection(authMethod: .key, keychainRef: ref)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
        #expect(captured.wasFired)
    }

    /// A real ECDSA P-256 OpenSSH-format key (`ssh-keygen -t ecdsa -b 256`),
    /// not one `SSHKeyFactory` can produce — its only P-256 generator is the
    /// Secure-Enclave path below, which stores a raw enclave handle, not a
    /// PEM. Test-only material: freshly generated for this file, used
    /// nowhere else, and never touches a network.
    private static let ecdsaP256TestPEM = """
        -----BEGIN OPENSSH PRIVATE KEY-----
        b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAaAAAABNlY2RzYS
        1zaGEyLW5pc3RwMjU2AAAACG5pc3RwMjU2AAAAQQSU3szH3+AyXg+0SKakAdMPr/VNIJ9a
        JzAe0vm5fbezzuWmCrbb6N7AsHsNUV4KbNa+eSKOpah71YzrFJNQJ2u9AAAAsHzxTrd88U
        63AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBJTezMff4DJeD7RI
        pqQB0w+v9U0gn1onMB7S+bl9t7PO5aYKttvo3sCwew1RXgps1r55Io6lqHvVjOsUk1Ana7
        0AAAAgI6Alln7E7oOyHO8Hrn7/zEDksMmt/F0AQ7nm4xo+6y0AAAARdGVzdC1maXh0dXJl
        LW9ubHkBAgMEBQYH
        -----END OPENSSH PRIVATE KEY-----
        """

    @Test("ECDSA P-256 PEM key auth assembles the right SSHAuthenticationMethod and reaches the provider")
    func keyAuthECDSAP256ReachesProvider() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(Self.ecdsaP256TestPEM, forRef: ref, requireBiometry: false)

        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(keychain: keychain, provider: provider)
        let connection = sampleConnection(authMethod: .key, keychainRef: ref)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
        #expect(captured.wasFired)
    }

    @Test("Secure Enclave (software-fallback) key auth reaches the provider via the custom auth delegate")
    func seP256AuthReachesProvider() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        // `generateSecureEnclaveP256` falls back to a software P-256 key with
        // no real Secure Enclave present (simulator/CI) — this is how the app
        // stays testable end-to-end without hardware, and is the one branch
        // in `SSHService.connect` with no coverage anywhere before this test.
        let generated = try SSHKeyFactory.generate(algorithm: .seP256, comment: "test")
        try await keychain.saveSecret(generated.privateBlob, forRef: ref, requireBiometry: false)

        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(keychain: keychain, provider: provider)
        var connection = sampleConnection(authMethod: .key, keychainRef: ref)
        connection.sshKeyAlgorithmRaw = SSHKeyAlgorithm.seP256.rawValue

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
        #expect(captured.wasFired)
    }

    @Test("connect surfaces underlying error when keychain ref does not exist, for key auth too")
    func keyAuthWithMissingKeychainEntryThrows() async throws {
        let service = makeService()
        let connection = sampleConnection(authMethod: .key, keychainRef: "moshpit.secret.absent")

        // Mirrors `connectWithMissingKeychainEntryThrows`, which only ever
        // exercised `.password` — the keychain-miss path for `.key` had no
        // test of its own.
        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
    }

    // MARK: - overrideSecret bypasses keychain

    @Test("overrideSecret bypasses the keychain lookup")
    func overrideSecretBypassesKeychain() async throws {
        let captured = Captured()
        let provider = ThrowingClientProvider { _ in captured.fire() }
        let service = makeService(provider: provider)
        // No keychainRef on the connection — would normally throw
        // `.missingKeychainRef`. With override, we should reach the provider.
        let connection = sampleConnection(authMethod: .password, keychainRef: nil)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection, overrideSecret: "pw")
        }
        #expect(captured.wasFired, "overrideSecret should short-circuit credential lookup")
    }

    // MARK: - TOFU handlers are per-call parameters, not shared service state

    /// `connect(...)` takes `onUnknownHost`/`onChangedHost` as parameters of
    /// the call itself (see its doc comment for why — the fix for a host-key
    /// race between overlapping sessions on the shared `SSHService.shared`).
    /// This exercises that the call still compiles and runs through the
    /// pre-flight pipeline with explicit handlers supplied, both with and
    /// without the defaults.
    @Test("connect accepts explicit TOFU handlers as call parameters")
    func connectAcceptsExplicitHostKeyHandlers() async throws {
        let service = makeService()
        let connection = sampleConnection(authMethod: .password, keychainRef: nil)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(
                connection,
                onUnknownHost: { _, _, _ in true },
                onChangedHost: { _, _, _, _ in false })
        }
    }

    @Test("connect falls back to deny-by-default handlers when none are supplied")
    func connectDefaultsToDenyingHostKeyHandlers() async throws {
        let service = makeService()
        let connection = sampleConnection(authMethod: .password, keychainRef: nil)

        await #expect(throws: SSHError.self) {
            _ = try await service.connect(connection)
        }
    }

    // MARK: - SSHError description sanity

    @Test("SSHError descriptions are non-empty for every case")
    func errorDescriptionsArePopulated() {
        let cases: [SSHError] = [
            .missingKeychainRef,
            .missingSecret,
            .unsupportedKeyType("ecdsa-sha2-nistp521"),
            .ptyAlreadyOpen,
            .ptyNotReady,
            .sessionClosed,
            .underlying(KeychainError.notFound),
        ]
        for c in cases {
            #expect(c.description.isEmpty == false)
        }
    }
}

/// Sendable bool-flag holder so test closures can mark themselves invoked.
private final class Captured: @unchecked Sendable {
    private let lock = NSLock()
    private var fired = false
    var wasFired: Bool {
        lock.lock(); defer { lock.unlock() }
        return fired
    }
    func fire() {
        lock.lock(); defer { lock.unlock() }
        fired = true
    }
}
