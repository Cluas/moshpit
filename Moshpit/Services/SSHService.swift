import Foundation
import Citadel
import Crypto
import NIOCore
import NIOSSH

// MARK: - Errors

enum SSHError: Error, CustomStringConvertible {
    case missingKeychainRef
    case missingSecret
    case unsupportedKeyType(String)
    case ptyAlreadyOpen
    case ptyNotReady
    case sessionClosed
    case authenticationFailed
    case authMethodRejected
    case channelOpenFailed
    case connectionFailed
    case handshakeFailed
    case credentialUnavailable
    case underlying(Error)

    /// Translate a raw Citadel / NIO error into the friendliest case. The
    /// SSH handshake reaches the server before the password is ever judged,
    /// so a wrong password surfaces here as `allAuthenticationOptionsFailed`
    /// — which users must see as "authentication failed", not "error 4".
    static func map(_ error: Error) -> SSHError {
        if let e = error as? SSHError { return e }
        if let c = error as? SSHClientError {
            switch c {
            case .allAuthenticationOptionsFailed:
                return .authenticationFailed
            case .unsupportedPasswordAuthentication,
                 .unsupportedPrivateKeyAuthentication,
                 .unsupportedHostBasedAuthentication:
                return .authMethodRejected
            case .channelCreationFailed:
                return .channelOpenFailed
            }
        }
        // Connection-level failures (host down / refused / timeout / DNS).
        if error is ChannelError { return .connectionFailed }
        let s = String(describing: error).lowercased()
        for needle in ["refused", "timeout", "timedout", "unreachable",
                       "connectionreset", "connectfailed", "nameresolution",
                       "posixerror"] where s.contains(needle) {
            return .connectionFailed
        }
        // The transport came up and then went away during the SSH handshake.
        // Citadel reports this as a ClientHandshakeHandler context with
        // "Disconnected error 1", which matched none of the needles above and
        // so reached the user verbatim — pointer addresses and all — on a
        // product whose troubleshooting page promises "plain language, not
        // stderr". Found by pointing a capture at an unroutable address.
        for needle in ["handshake", "disconnected"] where s.contains(needle) {
            return .handshakeFailed
        }
        return .underlying(error)
    }

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
        case .missingSecret:
            return String(localized: "Password / private key was not provided")
        case .unsupportedKeyType(let label):
            return String(localized: "Unsupported SSH key type: \(label)")
        case .ptyAlreadyOpen:
            return String(localized: "A PTY is already attached to this session")
        case .ptyNotReady:
            return String(localized: "PTY has not been opened yet — call requestPTY first")
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
        case .underlying:
            // Deliberately says nothing about the wrapped error. For a library
            // type that does not implement LocalizedError — which Citadel's
            // internals do not — `localizedDescription` renders as
            // "The operation couldn't be completed. (Citadel.ClientHandshake-
            // Handler.(unknown context at $105f91dd4)… error 1.)". That went
            // straight to the alert, on a product whose troubleshooting page
            // promises "plain language, not stderr". Adding needles to the
            // classifier only ever closes the paths someone thought of; this
            // closes the default. `diagnostic` keeps the detail for logs.
            return String(localized: "The connection failed for a reason Moshpit didn't recognise. Check the host, port and that the server is reachable, then try again.")
        }
    }
}

// MARK: - SSHClientProvider (testable)

/// Indirection over `Citadel.SSHClient.connect(...)` so tests can inject a
/// mock that never opens a network socket. Production uses
/// `CitadelSSHClientProvider`.
protocol SSHClientProvider: Sendable {
    func connect(
        host: String,
        port: Int,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SSHClient
}

struct CitadelSSHClientProvider: SSHClientProvider {
    func connect(
        host: String,
        port: Int,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SSHClient {
        try await SSHClient.connect(
            host: host,
            port: port,
            authenticationMethod: authenticationMethod,
            hostKeyValidator: hostKeyValidator,
            reconnect: .never
        )
    }
}

// MARK: - SSHSession

/// A single connected SSH session with a PTY-backed shell channel.
///
/// Lifecycle:
///  1. `SSHService.connect(connection)` returns an authenticated `SSHSession`.
///  2. Caller calls `requestPTY(rows:cols:)` once — that opens the PTY and
///     starts pumping inbound bytes into `dataStream`.
///  3. Caller writes user input with `write(_:)` and forwards resize events
///     with `resize(rows:cols:)`.
///  4. Caller terminates with `close()` (or by deinit — `close()` is safe to
///     call twice).
actor SSHSession {
    nonisolated let connection: ServerConnection
    nonisolated let dataStream: AsyncStream<Data>

    private let client: SSHClient
    private let continuation: AsyncStream<Data>.Continuation
    private var writer: TTYStdinWriter?
    private var ptyOpened = false
    private(set) var rows: Int = 24
    private(set) var cols: Int = 80
    private var closed = false
    /// Fired when the transport dies on its own (read loop hit EOF/error) —
    /// NOT on an intentional `close()`. Drives auto-reconnect. Cleared after.
    private var onUnexpectedClose: (@Sendable () -> Void)?

    init(connection: ServerConnection, client: SSHClient) {
        self.connection = connection
        self.client = client

        var captured: AsyncStream<Data>.Continuation!
        self.dataStream = AsyncStream<Data>(bufferingPolicy: .unbounded) { cont in
            captured = cont
        }
        self.continuation = captured
    }

    /// True until `close()` has run or the upstream stream finishes.
    var isConnected: Bool { !closed }

    /// Register a death handler. `close()` sets `closed` first, so the handler
    /// only fires on an *unexpected* drop, never on intentional teardown.
    func setOnUnexpectedClose(_ handler: @escaping @Sendable () -> Void) {
        onUnexpectedClose = handler
    }

    // MARK: - PTY

    func requestPTY(rows: Int, cols: Int) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        guard !ptyOpened else { throw SSHError.ptyAlreadyOpen }
        ptyOpened = true
        self.rows = rows
        self.cols = cols

        let request = SSHChannelRequestEvent.PseudoTerminalRequest(
            wantReply: true,
            term: "xterm-256color",
            terminalCharacterWidth: cols,
            terminalRowHeight: rows,
            terminalPixelWidth: 0,
            terminalPixelHeight: 0,
            terminalModes: SSHTerminalModes([:])
        )
        // Ask for a UTF-8 locale on the remote PTY, same value the mosh
        // bootstrap forwards (`-l LANG=…`). Without it many Linux hosts start
        // the session in the C locale and CJK input/output turns into
        // mojibake ("？？？"/<ffff>) — the classic "输入中文乱码" report.
        // OpenSSH accepts LANG by default (`AcceptEnv LANG LC_*`); servers
        // that refuse env requests just ignore this (wantReply: false).
        let environment = [
            SSHChannelRequestEvent.EnvironmentRequest(
                wantReply: false, name: "LANG", value: "en_US.UTF-8")
        ]

        // Handshake: `requestPTY()` should return only after Citadel has
        // accepted the PTY request and handed us a writer, but the
        // surrounding read-loop must keep running for the lifetime of the
        // session. We bridge with a CheckedContinuation. Closing the
        // underlying client breaks `withPTY`'s read loop, so we don't need
        // to hold the Task handle for cancellation.
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            Task { [client] in
                var resumed = false
                do {
                    try await client.withPTY(request, environment: environment) { output, writer in
                        await self.installWriter(writer)
                        if !resumed {
                            resumed = true
                            continuation.resume()
                        }
                        do {
                            for try await chunk in output {
                                let buffer: ByteBuffer
                                switch chunk {
                                case .stdout(let buf): buffer = buf
                                case .stderr(let buf): buffer = buf
                                }
                                let bytes = Array(buffer.readableBytesView)
                                if !bytes.isEmpty {
                                    await self.deliver(Data(bytes))
                                }
                            }
                        } catch {
                            // Read loop ended in error — fall through.
                        }
                    }
                } catch {
                    if !resumed {
                        resumed = true
                        continuation.resume(throwing: error)
                    }
                }
                await self.markClosed()
            }
        }
    }

    // MARK: - I/O

    func write(_ data: Data) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        guard let writer else { throw SSHError.ptyNotReady }
        var buffer = ByteBuffer()
        buffer.writeBytes(data)
        do {
            try await writer.write(buffer)
        } catch {
            throw SSHError.underlying(error)
        }
    }

    func writeText(_ text: String) async throws {
        guard let data = text.data(using: .utf8) else { return }
        try await write(data)
    }

    /// Run a one-shot command over a fresh exec channel and return its stdout.
    /// Used to bootstrap mosh-server (`mosh-server new -s …` prints the UDP
    /// port + session key, then daemonizes). Does not touch the PTY channel.
    func executeCommand(_ command: String) async throws -> Data {
        guard !closed else { throw SSHError.sessionClosed }
        do {
            let buffer = try await client.executeCommand(command, mergeStreams: true)
            return Data(buffer.readableBytesView)
        } catch {
            throw SSHError.underlying(error)
        }
    }

    func resize(rows: Int, cols: Int) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        self.rows = rows
        self.cols = cols
        guard let writer else { throw SSHError.ptyNotReady }
        do {
            try await writer.changeSize(cols: cols, rows: rows, pixelWidth: 0, pixelHeight: 0)
        } catch {
            throw SSHError.underlying(error)
        }
    }

    func close() async {
        guard !closed else { return }
        closed = true
        writer = nil
        continuation.finish()
        // Closing the SSHClient cancels in-flight PTY reads via channel
        // close → withPTY exits → markClosed is a no-op.
        try? await client.close()
    }

    // MARK: - Private actor-isolated helpers

    private func installWriter(_ w: TTYStdinWriter) {
        self.writer = w
    }

    private func deliver(_ data: Data) {
        guard !closed else { return }
        continuation.yield(data)
    }

    private func markClosed() {
        guard !closed else { return }
        closed = true
        writer = nil
        continuation.finish()
        // Reached only on an unexpected drop — `close()` flips `closed` first,
        // so the read loop's markClosed is a no-op there.
        let handler = onUnexpectedClose
        onUnexpectedClose = nil
        handler?()
    }
}

// MARK: - SSHService

/// Top-level façade that resolves credentials, authenticates, and hands the
/// caller an `SSHSession`. The service holds the long-lived dependencies
/// (`KeychainService`, `HostKeyValidator`) so views/view-models only need
/// one reference.
actor SSHService {
    typealias TrustNewHostHandler = TOFUHostKeyDelegate.TrustNewHost
    typealias AcceptChangedHostHandler = TOFUHostKeyDelegate.AcceptChangedHost

    private let keychain: KeychainService
    private let validator: HostKeyValidator
    private let clientProvider: SSHClientProvider

    /// In-memory credential cache, keyed by connection id. Populated after the
    /// first successful (biometry-gated) keychain read so the now-frequent
    /// auto-reconnects don't re-prompt Face ID on every attempt — and don't
    /// surface a keychain error mid-reconnect. Plaintext lives only in memory
    /// for the app session; cleared on intentional disconnect AND whenever the
    /// app backgrounds (see ``clearAllCachedSecrets()``).
    private var secretCache: [UUID: String] = [:]
    private var seBlobCache: [UUID: Data] = [:]

    /// Forget a cached credential (called when the user disconnects), so the
    /// next connect re-authenticates against the keychain.
    func clearCachedSecret(for id: UUID) {
        secretCache[id] = nil
        seBlobCache[id] = nil
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
    }

    init(keychain: KeychainService,
         hostKeyValidator: HostKeyValidator,
         clientProvider: SSHClientProvider = CitadelSSHClientProvider()) {
        self.keychain = keychain
        self.validator = hostKeyValidator
        self.clientProvider = clientProvider
    }

    // MARK: - Connect

    /// Authenticates and returns an open SSH session. Caller is responsible
    /// for calling `session.requestPTY(rows:cols:)` next.
    ///
    /// `overrideSecret` lets callers bypass the keychain (useful for the
    /// "Test connection" button in the add-connection form, before the
    /// credential is persisted).
    ///
    /// `onUnknownHost`/`onChangedHost` are the TOFU prompts for THIS
    /// handshake specifically, passed as parameters rather than read off
    /// mutable actor state. `SSHService.shared` is a process-wide singleton
    /// serving every concurrent session (plus background reconnect/keepalive
    /// and the mosh -CC sidecar), so a previous design that installed these
    /// via a separate `setHostKeyHandlers()` call before `connect()` had a
    /// window — between one session's install and its own connect — where a
    /// DIFFERENT session's concurrent install-then-connect could land in
    /// between and silently steal (or hand out) the handler for the wrong
    /// handshake. Threading them through the call itself makes that
    /// impossible: each `connect()` call is self-contained. Both default to
    /// "deny" so a caller that forgets to wire real UI never silently
    /// auto-trusts an unknown or changed host key.
    func connect(_ connection: ServerConnection,
                 overrideSecret: String? = nil,
                 onUnknownHost: @escaping TrustNewHostHandler = { _, _, _ in false },
                 onChangedHost: @escaping AcceptChangedHostHandler = { _, _, _, _ in false }) async throws -> SSHSession {
        let authMethod: SSHAuthenticationMethod
        if connection.authMethod == .key, connection.sshKeyAlgorithm == .seP256 {
            // Secure Enclave key: the keychain blob is the key's
            // dataRepresentation (an enclave handle), not PEM. Signing runs
            // inside the chip via NIOSSH's native SE support.
            guard let ref = connection.keychainRef else { throw SSHError.missingKeychainRef }
            let blob: Data
            if let cached = seBlobCache[connection.id] {
                blob = cached
            } else {
                do {
                    blob = try await keychain.loadSecret(
                        forRef: ref,
                        reason: "Authenticate to use the Secure Enclave key for \(connection.host)")
                } catch {
                    throw SSHError.credentialUnavailable
                }
                seBlobCache[connection.id] = blob
            }
            do {
                let seKey = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
                authMethod = .custom(SecureEnclaveP256AuthDelegate(
                    username: connection.username, key: seKey))
            } catch {
                throw SSHError.underlying(error)
            }
        } else {
            let secret = try await resolveSecret(for: connection, override: overrideSecret)
            authMethod = try makeAuthMethod(connection: connection, secret: secret)
        }

        let delegate = TOFUHostKeyDelegate(
            validator: validator,
            host: connection.host,
            port: connection.port,
            onNewHost: onUnknownHost,
            onChangedHost: onChangedHost
        )
        let hostKeyValidator = SSHHostKeyValidator.custom(delegate)

        do {
            let client = try await clientProvider.connect(
                host: connection.host,
                port: connection.port,
                authenticationMethod: authMethod,
                hostKeyValidator: hostKeyValidator
            )
            return SSHSession(connection: connection, client: client)
        } catch {
            throw SSHError.map(error)
        }
    }

    // MARK: - Helpers

    private func resolveSecret(for connection: ServerConnection,
                               override: String?) async throws -> String {
        if let override { return override }
        if let cached = secretCache[connection.id] { return cached }
        guard let ref = connection.keychainRef else {
            throw SSHError.missingKeychainRef
        }
        let secret: String
        do {
            switch connection.authMethod {
            case .password:
                secret = try await keychain.loadPassword(forRef: ref,
                                                         reason: "Authenticate to connect to \(connection.host)")
            case .key:
                secret = try await keychain.loadPrivateKey(forRef: ref,
                                                           reason: "Authenticate to use SSH key for \(connection.host)")
            }
        } catch is KeychainError {
            // Item missing / unreadable (often after a re-sign) → actionable.
            throw SSHError.credentialUnavailable
        }
        secretCache[connection.id] = secret
        return secret
    }

    private func makeAuthMethod(connection: ServerConnection,
                                secret: String) throws -> SSHAuthenticationMethod {
        switch connection.authMethod {
        case .password:
            return .passwordBased(username: connection.username, password: secret)

        case .key:
            let keyType: SSHKeyType
            do {
                keyType = try SSHKeyDetection.detectPrivateKeyType(from: secret)
            } catch {
                throw SSHError.underlying(error)
            }

            switch keyType {
            case .rsa:
                do {
                    let rsa = try Insecure.RSA.PrivateKey(sshRsa: secret)
                    return .rsa(username: connection.username, privateKey: rsa)
                } catch {
                    throw SSHError.underlying(error)
                }
            case .ed25519:
                do {
                    let key = try Curve25519.Signing.PrivateKey(sshEd25519: secret)
                    return .ed25519(username: connection.username, privateKey: key)
                } catch {
                    throw SSHError.underlying(error)
                }
            case .ecdsaP256:
                do {
                    let key = try P256.Signing.PrivateKey(pemRepresentation: secret)
                    return .p256(username: connection.username, privateKey: key)
                } catch {
                    throw SSHError.underlying(error)
                }
            default:
                // ECDSA P-384/P-521 PEM loading still TODO; ECDSA-sk lives on
                // a hardware token and cannot be loaded at all.
                throw SSHError.unsupportedKeyType(keyType.description)
            }
        }
    }
}

// MARK: - Secure Enclave auth delegate

/// Offers a Secure Enclave P-256 key for publickey auth. NIOSSH signs via the
/// enclave (`NIOSSHPrivateKey(secureEnclaveP256Key:)`); the private key never
/// leaves the chip. One attempt — if the server rejects it, auth fails.
private final class SecureEnclaveP256AuthDelegate: NIOSSHClientUserAuthenticationDelegate {
    private let username: String
    private let key: SecureEnclave.P256.Signing.PrivateKey
    private var attempted = false

    init(username: String, key: SecureEnclave.P256.Signing.PrivateKey) {
        self.username = username
        self.key = key
    }

    func nextAuthenticationType(
        availableMethods: NIOSSHAvailableUserAuthenticationMethods,
        nextChallengePromise: EventLoopPromise<NIOSSHUserAuthenticationOffer?>
    ) {
        guard !attempted, availableMethods.contains(.publicKey) else {
            nextChallengePromise.succeed(nil)   // no further methods → clean failure
            return
        }
        attempted = true
        nextChallengePromise.succeed(NIOSSHUserAuthenticationOffer(
            username: username,
            serviceName: "",
            offer: .privateKey(.init(privateKey: NIOSSHPrivateKey(secureEnclaveP256Key: key)))))
    }
}

// MARK: - Shared instance

extension SSHService {
    /// Process-wide default service used by views/view-models that don't
    /// otherwise inject one. Backed by the default ``KeychainService`` and
    /// ``HostKeyValidator`` so it pulls from the real keychain + known-hosts
    /// store. Tests should construct their own ``SSHService`` with mocks
    /// rather than touching this singleton.
    static let shared: SSHService = {
        let kc = KeychainService()
        let hkv = HostKeyValidator()
        return SSHService(keychain: kc, hostKeyValidator: hkv)
    }()
}
