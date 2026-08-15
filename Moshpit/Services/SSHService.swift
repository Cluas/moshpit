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

    /// Runs the SSH handshake over an already-connected channel instead of
    /// dialing `host:port` itself — the SOCKS-proxy path hands in a channel
    /// that's already past the proxy's CONNECT handshake.
    func connect(
        on channel: Channel,
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

    func connect(
        on channel: Channel,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SSHClient {
        try await SSHClient.connect(
            on: channel,
            authenticationMethod: authenticationMethod,
            hostKeyValidator: hostKeyValidator
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

    /// Upload one file into `~/.moshpit/uploads/` over an SFTP subchannel on
    /// this already-authenticated connection, returning the absolute remote
    /// path. Does not touch the PTY channel, so the terminal stays usable
    /// while bytes move.
    ///
    /// The home directory is resolved with `realpath .` because SFTP starts
    /// in `$HOME` but never expands `~` itself — an upload addressed to a
    /// literal `~/…` lands in a directory named `~`. `mkdir` has no `-p` and
    /// errors on an existing directory, so both creates are best-effort; a
    /// genuinely un-creatable directory still fails loudly at the open.
    /// Directory 700 / file 600: screenshots on a shared host are nobody
    /// else's to read.
    func uploadToUploadsDirectory(
        _ data: Data,
        named filename: String,
        progress: (@Sendable (Double) -> Void)? = nil) async throws -> String {
        guard !closed else { throw SSHError.sessionClosed }
        // SFTPFileAttributes' public init doesn't take permissions — set the
        // field after construction.
        func attributes(permissions: UInt32) -> SFTPFileAttributes {
            var attrs = SFTPFileAttributes()
            attrs.permissions = permissions
            return attrs
        }
        do {
            return try await client.withSFTP { sftp in
                let home = try await sftp.getRealPath(atPath: ".")
                let appDir = home + "/.moshpit"
                let uploads = appDir + "/uploads"
                try? await sftp.createDirectory(
                    atPath: appDir, attributes: attributes(permissions: 0o700))
                try? await sftp.createDirectory(
                    atPath: uploads, attributes: attributes(permissions: 0o700))
                let path = uploads + "/" + filename
                try await sftp.withFile(
                    filePath: path,
                    flags: [.write, .create, .truncate],
                    attributes: attributes(permissions: 0o600)) { file in
                    // Chunked so a cancel lands between writes and progress
                    // moves during the transfer, not just at its end.
                    let chunkSize = 128 * 1024
                    var offset = 0
                    while offset < data.count {
                        try Task.checkCancellation()
                        let end = min(offset + chunkSize, data.count)
                        try await file.write(
                            ByteBuffer(bytes: data[offset..<end]), at: UInt64(offset))
                        offset = end
                        progress?(Double(offset) / Double(data.count))
                    }
                }
                return path
            }
        } catch is CancellationError {
            throw CancellationError()
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

    /// Keychain reads currently in flight, keyed by connection id, so
    /// concurrent resolutions of the same connection share ONE read — and
    /// therefore one Face ID prompt. This actor is reentrant: without this,
    /// two `connect`s racing for the same connection both see an empty
    /// `secretCache`, both suspend into the keychain, and the user gets two
    /// biometry sheets for one tap. Not hypothetical — the mosh+tmux
    /// dual-transport design authenticates two SSH connections (bootstrap +
    /// `-CC` sidecar) with the same credential, and a parallel `resumeAll`
    /// resumes them together.
    private var secretResolutions: [UUID: Task<String, Error>] = [:]

    /// Forget a cached credential (called when the user disconnects), so the
    /// next connect re-authenticates against the keychain.
    func clearCachedSecret(for id: UUID) {
        secretCache[id] = nil
        seBlobCache[id] = nil
        // Detach (don't cancel) any in-flight read: connects already awaiting
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
            let client: SSHClient
            // SOCKS proxies only speak TCP CONNECT — this affects the SSH
            // dial only. If Mosh is also on, its UDP session still connects
            // directly once SSH bootstraps it; the Add Connection form's
            // PROXY footer says so up front rather than let that surface as
            // a silent post-handshake failure.
            if connection.useSOCKSProxy == true, let proxyHost = connection.socksProxyHost, !proxyHost.isEmpty {
                let channel = try await SOCKSProxyDialer.connect(
                    proxyHost: proxyHost,
                    proxyPort: connection.socksProxyPort ?? 1080,
                    targetHost: connection.host,
                    targetPort: connection.port
                )
                client = try await clientProvider.connect(
                    on: channel,
                    authenticationMethod: authMethod,
                    hostKeyValidator: hostKeyValidator
                )
            } else {
                client = try await clientProvider.connect(
                    host: connection.host,
                    port: connection.port,
                    authenticationMethod: authMethod,
                    hostKeyValidator: hostKeyValidator
                )
            }
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
        // Join a read already in flight instead of starting a second one —
        // this is what keeps one user action at one Face ID prompt when two
        // connects race for the same connection (see `secretResolutions`).
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
            } catch is KeychainError {
                // Item missing / unreadable (often after a re-sign) → actionable.
                throw SSHError.credentialUnavailable
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
                    let key = try OpenSSHECDSAKey.p256PrivateKey(fromPEM: secret)
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

// MARK: - ECDSA-P256 OpenSSH parsing

/// Parses a P-256 private key out of an openssh-key-v1 container — the
/// counterpart Citadel doesn't provide (its own key-type enum only knows
/// "ssh-rsa" and "ssh-ed25519"; there's no ECDSA reader to reuse). Only
/// unencrypted (cipher "none") containers are supported, matching what
/// `SSHKeyFactory` produces and what the ed25519/RSA paths above already
/// assume. Format reference: https://dnaeon.github.io/openssh-private-key-binary-format/
private enum OpenSSHECDSAKey {
    enum ParseError: LocalizedError {
        case malformed(String)

        var errorDescription: String? {
            switch self {
            case .malformed(let why): return "Malformed OpenSSH private key: \(why)"
            }
        }
    }

    /// Sequential reader for the SSH wire format (uint32 length-prefixed
    /// strings) — distinct from ASN.1 DER, which `SSHKeyFactory`'s reader
    /// handles; this container is entirely SSH-native framing.
    private struct WireReader {
        private let data: Data
        private var idx: Data.Index

        init(_ data: Data) {
            self.data = data
            self.idx = data.startIndex
        }

        mutating func readUInt32() -> UInt32? {
            guard let bytes = readBytes(4) else { return nil }
            return bytes.reduce(0 as UInt32) { ($0 << 8) | UInt32($1) }
        }

        mutating func readBytes(_ count: Int) -> Data? {
            guard count >= 0, data.distance(from: idx, to: data.endIndex) >= count else { return nil }
            let slice = data[idx..<data.index(idx, offsetBy: count)]
            idx = data.index(idx, offsetBy: count)
            return Data(slice)
        }

        mutating func readSSHString() -> Data? {
            guard let length = readUInt32() else { return nil }
            return readBytes(Int(length))
        }
    }

    static func p256PrivateKey(fromPEM pem: String) throws -> P256.Signing.PrivateKey {
        var text = pem.trimmingCharacters(in: .whitespacesAndNewlines)
        let begin = "-----BEGIN OPENSSH PRIVATE KEY-----"
        let end = "-----END OPENSSH PRIVATE KEY-----"
        guard text.hasPrefix(begin), text.hasSuffix(end) else {
            throw ParseError.malformed("missing OpenSSH PEM boundary")
        }
        text.removeLast(end.count)
        text.removeFirst(begin.count)
        guard let raw = Data(base64Encoded: text.replacingOccurrences(of: "\n", with: "")) else {
            throw ParseError.malformed("invalid base64 payload")
        }

        var reader = WireReader(raw)
        guard reader.readBytes(15) == Data("openssh-key-v1\0".utf8) else {
            throw ParseError.malformed("missing openssh-key-v1 magic")
        }
        guard reader.readSSHString() == Data("none".utf8) else {
            throw ParseError.malformed("encrypted OpenSSH keys are not supported")
        }
        guard reader.readSSHString() == Data("none".utf8) else {
            throw ParseError.malformed("encrypted OpenSSH keys are not supported")
        }
        guard let kdfOptions = reader.readSSHString(), kdfOptions.isEmpty else {
            throw ParseError.malformed("unexpected KDF options")
        }
        guard reader.readUInt32() == 1 else {
            throw ParseError.malformed("expected exactly one key in the container")
        }
        guard reader.readSSHString() != nil else {   // public key blob — re-derived below instead of trusted
            throw ParseError.malformed("missing public key blob")
        }
        guard let privBlob = reader.readSSHString() else {
            throw ParseError.malformed("missing private key blob")
        }

        var priv = WireReader(privBlob)
        guard let check0 = priv.readUInt32(), priv.readUInt32() == check0 else {
            throw ParseError.malformed("private key checksum mismatch")
        }
        guard priv.readSSHString() == Data("ecdsa-sha2-nistp256".utf8) else {
            throw ParseError.malformed("not an ecdsa-sha2-nistp256 key")
        }
        guard priv.readSSHString() == Data("nistp256".utf8) else {
            throw ParseError.malformed("unsupported curve — only nistp256 is handled here")
        }
        guard priv.readSSHString() != nil else {   // public point Q — re-derived below instead of trusted
            throw ParseError.malformed("missing public key point")
        }
        guard let scalarMPInt = priv.readSSHString() else {
            throw ParseError.malformed("missing private scalar")
        }

        return try P256.Signing.PrivateKey(rawRepresentation: fixedLength(fromMPInt: scalarMPInt, length: 32))
    }

    /// Inverse of the `sshMPInt` encoding `SSHKeyFactory` writes: mpint
    /// encoding strips leading zero bytes from the big-endian integer (and
    /// adds one back only as a sign guard when the high bit is set), so the
    /// body isn't reliably exactly 32 bytes — this restores that fixed width.
    private static func fixedLength(fromMPInt mpint: Data, length: Int) -> Data {
        var bytes = mpint
        if bytes.count > length, bytes.first == 0 {
            bytes.removeFirst()
        }
        if bytes.count < length {
            bytes = Data(repeating: 0, count: length - bytes.count) + bytes
        }
        return bytes
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
