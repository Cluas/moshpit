import Foundation
import UIKit
@preconcurrency import Citadel
import Crypto
import NIOCore
import NIOSSH
import MoshpitKit

// MARK: - Errors

enum SSHError: Error, CustomStringConvertible, LocalizedError {
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
    /// this server's key could not be checked — see `HostKeyDecision.unverifiable`.
    case hostKeysLocked
    /// The user said no to the server's key: the new-host card's Cancel,
    /// the changed-key card's Disconnect (``HostKeyRejected``). Their own
    /// decision, so the terminal ends the connect quietly (B151).
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

    /// Translate a raw Citadel / NIO error into the friendliest case. The
    /// SSH handshake reaches the server before the password is ever judged,
    /// so a wrong password surfaces here as `allAuthenticationOptionsFailed`
    /// — which users must see as "authentication failed", not "error 4".
    static func map(_ error: Error) -> SSHError {
        if let e = error as? SSHError { return e }
        if error is HostKeyRejected { return .hostKeyRejected }
        if let unreadable = error as? KnownHostsUnreadable, unreadable.locked { return .hostKeysLocked }
        if let proxy = error as? SOCKSProxyDialer.DialError {
            switch proxy {
            case .proxyUnreachable(let host, let port, _): return .proxyUnreachable("\(host):\(port)")
            case .proxyFailed: return .proxyFailed
            }
        }
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

    /// What a Secure Enclave key's failed sign-in means (B249), when the
    /// chip is what failed: the offer's signature refused before NIOSSH
    /// signed (``SecureEnclaveRefused``), or the signature NIOSSH makes in
    /// the handshake (a CryptoKit or keychain error). While protected data
    /// is unavailable that is the lock — ``credentialLocked``, retried on the
    /// unlock as B227 has it for the key's other operations — where it read
    /// as a failed sign-in, never retried; with the device unlocked it is
    /// the key's own failure. nil: not the chip's (a refused key, a host
    /// that went away), mapped as any sign-in's.
    static func secureEnclaveSignIn(failedWith error: Error, protectedDataAvailable: Bool) -> SSHError? {
        let chip: Error
        if let refused = error as? SecureEnclaveRefused {
            chip = refused.error
        } else if error is CryptoKitError || [NSOSStatusErrorDomain, "CryptoTokenKit"].contains((error as NSError).domain) {
            chip = error
        } else {
            return nil
        }
        return protectedDataAvailable ? .underlying(chip) : .credentialLocked
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
    ) async throws -> SignedInClient

    /// Runs the SSH handshake over an already-connected channel instead of
    /// dialing `host:port` itself — the SOCKS-proxy path hands in a channel
    /// that's already past the proxy's CONNECT handshake. Returns once the
    /// server accepted the sign-in, as the direct `connect` does, and throws
    /// what it didn't accept.
    func connect(
        on channel: Channel,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SignedInClient
}

/// A signed-in Citadel client and the TCP channel it runs over — whose
/// pipeline holds the NIOSSHHandler a command's own exec channel is opened
/// on (`SSHExec`).
struct SignedInClient {
    let client: SSHClient
    let channel: Channel
}

struct CitadelSSHClientProvider: SSHClientProvider {
    func connect(
        host: String,
        port: Int,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SignedInClient {
        // Dialled here, not by Citadel, so that a connect that fails closes
        // its connection: Citadel's own dial leaves the channel of a refused
        // sign-in or a stopped handshake open (NIOSSH doesn't close on an
        // error) until the server tires of it.
        let channel = try await SSHDial.directChannel(
            host: host, port: port,
            // The TCP connect only; the handshake has its own clock. Citadel's
            // default is 30s. A SYN that gets no answer in 15s is a black hole
            // — most often a VPN address with the tunnel down — and the sooner
            // it fails, the sooner the retry lands on the path the user has
            // meanwhile fixed. Real paths connect in one round trip.
            timeout: .seconds(15))
        let client = try await SSHDial.signIn(on: channel,
                                              authenticationMethod: authenticationMethod,
                                              hostKeyValidator: hostKeyValidator)
        return SignedInClient(client: client, channel: channel)
    }

    func connect(
        on channel: Channel,
        authenticationMethod: SSHAuthenticationMethod,
        hostKeyValidator: SSHHostKeyValidator
    ) async throws -> SignedInClient {
        // Waits for the sign-in, as the direct connect does, on the channel's
        // event loop; Citadel's `connect(on:authenticationMethod:…)` did
        // neither (`SSHDial`).
        let client = try await SSHDial.signIn(on: channel,
                                              authenticationMethod: authenticationMethod,
                                              hostKeyValidator: hostKeyValidator)
        return SignedInClient(client: client, channel: channel)
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
///
/// Internally the session talks to its SSH client through the
/// ``SSHClientTransport`` seam, so lifecycle tests can run a REAL session
/// over a scripted fake — same pattern as `MoshTransport`'s
/// `DatagramChannel`.
actor SSHSession {
    nonisolated let connection: ServerConnection
    nonisolated let dataStream: AsyncStream<Data>

    private let transport: any SSHClientTransport
    /// The concrete Citadel client, kept ONLY for the SFTP subsystem — SFTP
    /// is deliberately outside the ``SSHClientTransport`` seam. A session
    /// built over an injected transport (tests) has none, and uploads then
    /// exercise the exec/base64 fallback, which the fake CAN run.
    private let sftpClient: SSHClient?
    // nonisolated: the PTY read loop yields into it DIRECTLY, synchronously,
    // preserving chunk order. It used to hop through `Task { await deliver }`
    // per chunk — an UNORDERED task per chunk, so under load chunk N+1 could
    // reach the actor before chunk N and the terminal byte stream arrived
    // REORDERED: split escape sequences, protocol lines glued into %output
    // payloads, characters at wrong positions. Bisected to b941818 against a
    // user-verified-good build 334 (2026-08-19); every rendering symptom
    // since traced back to this. `yield` is thread-safe, and yield-after-
    // finish is a documented no-op, so no `closed` guard is needed.
    private nonisolated let continuation: AsyncStream<Data>.Continuation
    private var writer: (any SSHPTYWriter)?
    private var ptyOpened = false
    private var closed = false
    /// `close()` has shut the connection down (once is enough).
    private var transportShut = false
    /// Fired when the PTY channel ends on its own — the remote shell exited,
    /// or the connection under it dropped — NOT on an intentional `close()`.
    /// Told which of the two it was (see ``SSHChannelEnd``): a drop is
    /// redialled, an exit ends the session. Cleared after.
    private var onUnexpectedClose: (@Sendable (SSHChannelEnd) -> Void)?
    /// How the PTY channel ended on its own, once it has — kept for a handler
    /// registered only after the end (a login shell that exits at once).
    private var channelEnd: SSHChannelEnd?

    init(connection: ServerConnection, client: SSHClient, channel: Channel) {
        self.init(connection: connection,
                  transport: CitadelClientTransport(client: client, channel: channel),
                  sftpClient: client)
    }

    /// The lifecycle-test entry: a REAL session over a scripted transport
    /// (hung execs on half-open sockets, refused PTYs, dead writers) — the
    /// same seam pattern `MoshTransport` gets from `DatagramChannel`.
    init(connection: ServerConnection,
         transport: any SSHClientTransport,
         sftpClient: SSHClient? = nil) {
        self.connection = connection
        self.transport = transport
        self.sftpClient = sftpClient

        var captured: AsyncStream<Data>.Continuation!
        self.dataStream = AsyncStream<Data>(bufferingPolicy: .unbounded) { cont in
            captured = cont
        }
        self.continuation = captured
    }

    /// True until `close()` has run or the upstream stream finishes.
    var isConnected: Bool { !closed }

    /// The address this connection reached when it went straight to the
    /// host; nil through a SOCKS proxy (the channel's peer is the proxy) and
    /// when unknown. Where a mosh session started over it sends its UDP: the
    /// address `mosh-server -s` bound to. Resolving the host name again can
    /// land elsewhere when it has several addresses — `localhost` gives
    /// `::1` first, and a server bound to 127.0.0.1 never answers there:
    /// connected, and black (B171).
    nonisolated var peerAddress: String? {
        let proxied = connection.useSOCKSProxy == true && !(connection.socksProxyHost ?? "").isEmpty
        return proxied ? nil : transport.peerAddress
    }

    /// Register a death handler. `close()` sets `closed` first, so the handler
    /// only fires when the channel ends on its own, never on intentional
    /// teardown. A channel that already ended so — the PTY is up before the
    /// caller gets to register, and a login shell can exit straight away —
    /// is reported at once.
    func setOnUnexpectedClose(_ handler: @escaping @Sendable (SSHChannelEnd) -> Void) {
        if let channelEnd {
            handler(channelEnd)
            return
        }
        onUnexpectedClose = handler
    }

    // MARK: - PTY

    /// Open the session's terminal channel: the account's login shell, or
    /// `command` (a POSIX sh script, run as `sh -c`) as the channel's own
    /// program. A program of the app's — tmux's control client, herdr's frame
    /// loop, the push pump — is the channel's command, never a line typed
    /// into a login shell that may still be starting: a startup that flushes
    /// pending input (Powerlevel10k's instant prompt) or a line editor busy
    /// with plugins swallowed it, and nothing came up (B267).
    func requestPTY(rows: Int, cols: Int, command: String? = nil) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        guard !ptyOpened else { throw SSHError.ptyAlreadyOpen }
        ptyOpened = true

        let w = try await transport.openPTY(
            rows: rows, cols: cols, command: command,
            onOutput: { [weak self] data in
                // Synchronous, in-order: the read loop is the only caller and
                // it is sequential. See `continuation`'s doc for the
                // reordering bug the old per-chunk Task caused.
                self?.continuation.yield(data)
            },
            onEnd: { [weak self] end in
                Task { await self?.markClosed(end) }
            })
        installWriter(w)
    }

    // MARK: - I/O

    func write(_ data: Data) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        guard let writer else { throw SSHError.ptyNotReady }
        do {
            try await writer.write(data)
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
        // Debug level, so it costs nothing unless someone is listening — but
        // when they are, this is the only way to see channel churn, which is
        // what a flaky link actually pays for (one exec = one channel = a
        // fork on the server and a couple of round trips).
        // The PATH prefix is 60 characters of boilerplate on every herdr
        // command — log what actually differs.
        let logged = command.hasPrefix("PATH=")
            ? String(command.drop(while: { $0 != " " }).dropFirst())
            : command
        Log.ssh.debug("exec: \(logged.prefix(60), privacy: .public)")
        do {
            return try await transport.run(command)
        } catch {
            throw SSHError.underlying(error)
        }
    }

    /// Remembered evidence that this host's SFTP subsystem is unavailable
    /// (an upload already fell back to exec and succeeded there). Session-
    /// scoped on purpose: a reconnect probes fresh, and sshd config changes
    /// are rare enough that re-learning once per connection is cheap.
    private var sftpKnownUnavailable = false

    /// Upload one file into `~/.moshpit/uploads/` over an SFTP subchannel on
    /// this already-authenticated connection, returning the absolute remote
    /// path. Does not touch the PTY channel, so the terminal stays usable
    /// while bytes move.
    ///
    /// Hosts with `Subsystem sftp` disabled fall back to landing the bytes
    /// over exec channels alone (base64 chunks appended to a temp file, one
    /// decode at the end — see ``ExecUploadCommands``). The fallback runs on
    /// ANY SFTP failure rather than trying to classify Citadel's errors; if
    /// both paths fail, the exec error is the one surfaced, and SFTP is only
    /// remembered as unavailable once exec has actually succeeded.
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
        if !sftpKnownUnavailable {
            do {
                return try await sftpUpload(data, named: filename, progress: progress)
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                // Fall through to the exec path; its error wins if it also
                // fails, since it is the one still standing.
            }
        }
        let path = try await execUpload(data, named: filename, progress: progress)
        sftpKnownUnavailable = true
        return path
    }

    private func sftpUpload(
        _ data: Data,
        named filename: String,
        progress: (@Sendable (Double) -> Void)?) async throws -> String {
        // SFTPFileAttributes' public init doesn't take permissions — set the
        // field after construction.
        @Sendable func attributes(permissions: UInt32) -> SFTPFileAttributes {
            var attrs = SFTPFileAttributes()
            attrs.permissions = permissions
            return attrs
        }
        do {
            // A transport-injected session (tests) has no Citadel client and
            // therefore no SFTP — throw into the exec/base64 fallback, which
            // is exactly the path such a session wants exercised.
            guard let sftpClient else { throw SSHError.sessionClosed }
            return try await sftpClient.withSFTP { sftp in
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

    /// The no-SFTP path: base64 chunks appended over exec channels, decoded
    /// once at the end. ~45 round trips for a 2MB image — a fallback, not
    /// the fast path, but it works against nothing more than a shell.
    private func execUpload(
        _ data: Data,
        named filename: String,
        progress: (@Sendable (Double) -> Void)?) async throws -> String {
        let homeData = try await executeCommand("printf %s \"$HOME\"")
        guard let home = String(data: homeData, encoding: .utf8),
              home.hasPrefix("/") else {
            throw SSHError.underlying(ExecUploadError.homeUnresolvable)
        }
        let plan = ExecUploadCommands.plan(home: home, filename: filename)

        _ = try await executeCommand(plan.begin)
        var offset = 0
        while offset < data.count {
            try Task.checkCancellation()
            let end = min(offset + ExecUploadCommands.chunkBytes, data.count)
            let chunk = data[offset..<end].base64EncodedString()
            _ = try await executeCommand(plan.append(chunk))
            offset = end
            // Cap below 1.0 — the decode round trip is still ahead.
            progress?(0.95 * Double(offset) / Double(data.count))
        }
        let reply = try await executeCommand(plan.finish)
        guard String(data: reply, encoding: .utf8)?
            .contains(ExecUploadCommands.successMarker) == true else {
            throw SSHError.underlying(ExecUploadError.decodeFailed)
        }
        progress?(1)
        return plan.finalPath
    }

    enum ExecUploadError: Error {
        case homeUnresolvable
        case decodeFailed
    }

    // MARK: - Reading files for preview

    /// The SFTP session previews read through, kept open between reads: a
    /// folder of thumbnails is a dozen reads in a row, and opening a fresh
    /// subsystem channel for each (what `withSFTP` does) costs a round trip
    /// apiece. Reopened when the channel has died; closed with the session.
    private var previewSFTP: SFTPClient?

    private func previewReader() async throws -> SFTPClient {
        if let previewSFTP, previewSFTP.isActive { return previewSFTP }
        // A transport-injected session (tests) has no Citadel client; the
        // callers fall back to exec, as uploads do.
        guard let sftpClient else { throw SSHError.sessionClosed }
        let fresh = try await sftpClient.openSFTP()
        previewSFTP = fresh
        return fresh
    }

    /// What `path` is: file or folder, size, modification time. SFTP stat,
    /// or the exec fallback (no time) on a host without the subsystem.
    func remoteStat(_ path: String) async throws -> RemoteFileEntry {
        guard !closed else { throw SSHError.sessionClosed }
        do {
            let attributes = try await previewReader().getAttributes(at: path)
            return Self.entry(path: path, attributes)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            try Self.throwIfSFTPSaidWhy(error)
            let output = try await executeCommand(RemoteFileCommands.stat(path))
            guard let entry = RemoteFileCommands.parseStat(String(decoding: output, as: UTF8.self),
                                                           path: path) else {
                throw SSHError.underlying(RemoteReadError.notFound)
            }
            return entry
        }
    }

    /// Opens `path` for reads at any offset — what streaming playback pulls
    /// byte ranges through. SFTP only: the exec fallback can't read a range,
    /// so on a host without SFTP this throws and the caller downloads.
    func openRemoteFile(_ path: String) async throws -> RemoteFileHandle {
        guard !closed else { throw SSHError.sessionClosed }
        let file: SFTPFile
        do {
            file = try await previewReader().openFile(filePath: path, flags: .read)
        } catch {
            try Self.throwIfSFTPSaidWhy(error)
            throw error
        }
        do {
            let size = try await file.readAttributes().size ?? 0
            guard size > 0 else { throw SSHError.underlying(RemoteReadError.notFound) }
            return RemoteFileHandle(file: file, size: size)
        } catch {
            try? await file.close()
            throw error
        }
    }

    /// The login directory — where SFTP and exec channels start. SFTP
    /// resolves `.`; the exec fallback asks the shell.
    func remoteHomeDirectory() async throws -> String {
        guard !closed else { throw SSHError.sessionClosed }
        do {
            let home = try await previewReader().getRealPath(atPath: ".")
            if home.hasPrefix("/") { return home }
        } catch is CancellationError {
            throw CancellationError()
        } catch {}
        let output = try await executeCommand("pwd")
        let home = String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard home.hasPrefix("/") else { throw SSHError.underlying(RemoteReadError.notFound) }
        return home
    }

    /// The entries of the folder at `path`, unfiltered and unsorted (see
    /// `RemoteFileEntry.browsable`).
    func listRemoteDirectory(_ path: String) async throws -> [RemoteFileEntry] {
        guard !closed else { throw SSHError.sessionClosed }
        do {
            let base = path.hasSuffix("/") ? String(path.dropLast()) : path
            return try await previewReader().listDirectory(atPath: path)
                .flatMap(\.components)
                .filter { $0.filename != "." && $0.filename != ".." }
                .map { Self.entry(path: base + "/" + $0.filename, $0.attributes) }
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            try Self.throwIfSFTPSaidWhy(error)
            let output = try await executeCommand(RemoteFileCommands.list(path))
            return RemoteFileCommands.parseList(String(decoding: output, as: UTF8.self), in: path)
        }
    }

    /// Copy the file at `path` into `destination` (created or replaced).
    /// Reads ride SFTP in 256KB chunks, eight in flight at a time so a long
    /// round trip doesn't set the pace; the exec fallback moves files up to
    /// `RemoteFileCommands.execDownloadLimit` as one base64 reply.
    func downloadRemoteFile(
        _ path: String,
        to destination: URL,
        progress: (@Sendable (Double) -> Void)? = nil) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        do {
            try await sftpDownload(path, to: destination, progress: progress)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            try Self.throwIfSFTPSaidWhy(error)
            try await execDownload(path, to: destination, progress: progress)
        }
    }

    private func sftpDownload(
        _ path: String,
        to destination: URL,
        progress: (@Sendable (Double) -> Void)?) async throws {
        let file = try await previewReader().openFile(filePath: path, flags: .read)
        do {
            let size = try await file.readAttributes().size ?? 0
            FileManager.default.createFile(atPath: destination.path, contents: nil)
            let handle = try FileHandle(forWritingTo: destination)
            defer { try? handle.close() }
            _ = try await WindowedRead.pull(
                size: size, chunk: 64 * 1024, window: 16,
                read: { offset, length in
                    var buffer = try await file.read(from: offset, length: length)
                    return buffer.readBytes(length: buffer.readableBytes) ?? []
                },
                write: { offset, bytes in
                    try handle.seek(toOffset: offset)
                    try handle.write(contentsOf: bytes)
                },
                progress: { received in
                    if size > 0 { progress?(min(1, Double(received) / Double(size))) }
                })
            try? await file.close()
        } catch {
            try? await file.close()
            throw error
        }
    }

    private func execDownload(
        _ path: String,
        to destination: URL,
        progress: (@Sendable (Double) -> Void)?) async throws {
        let entry = try await remoteStat(path)
        guard !entry.isDirectory else { throw SSHError.underlying(RemoteReadError.notFound) }
        guard (entry.size ?? 0) <= RemoteFileCommands.execDownloadLimit else {
            throw SSHError.underlying(RemoteReadError.tooLargeWithoutSFTP)
        }
        let output = try await executeCommand(RemoteFileCommands.cat(path))
        guard let data = Data(base64Encoded: output, options: .ignoreUnknownCharacters) else {
            throw SSHError.underlying(RemoteReadError.notFound)
        }
        try data.write(to: destination)
        progress?(1)
    }

    enum RemoteReadError: Error {
        case notFound
        /// The host won't let this account read it.
        case denied
        case tooLargeWithoutSFTP
    }

    /// What the SFTP server said about the path, when it said something the
    /// exec form can't change: it isn't there, or isn't this account's to
    /// read. Anything else — no SFTP subsystem, a channel that dropped, a
    /// call it doesn't support — leaves the exec form to try (B241: every
    /// SFTP failure went to exec, whose answer showed as a generic error).
    nonisolated static func sftpRefusal(_ code: SFTPStatusCode?) -> RemoteReadError? {
        switch code {
        case .noSuchFile?: return .notFound
        case .permissionDenied?: return .denied
        default: return nil
        }
    }

    /// The status code an SFTP call failed with (Citadel throws the
    /// server's status as it came, or wrapped).
    nonisolated static func sftpStatusCode(_ error: Error) -> SFTPStatusCode? {
        if let status = error as? SFTPMessage.Status { return status.errorCode }
        if case SFTPError.errorStatus(let status)? = error as? SFTPError { return status.errorCode }
        return nil
    }

    private static func throwIfSFTPSaidWhy(_ error: Error) throws {
        if let refusal = sftpRefusal(sftpStatusCode(error)) { throw SSHError.underlying(refusal) }
    }

    // MARK: - Changing files from the folder sheet

    enum RemoteEditError: Error {
        /// Something with the target name is already there.
        case nameTaken
        /// The exec form ran but didn't report success, or the path is one
        /// the sheet never deletes.
        case failed
    }

    /// SFTP first, the exec form (`RemoteFileEdits`) on any SFTP failure —
    /// the same split as reads. The exec reply decides: its marker is the
    /// only success there is.
    private func edit(sftp: (SFTPClient) async throws -> Void, exec command: String?) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        do {
            try await sftp(previewReader())
            return
        } catch is CancellationError {
            throw CancellationError()
        } catch {}
        guard let command else { throw SSHError.underlying(RemoteEditError.failed) }
        try Self.checkEditReply(try await editReply(command))
    }

    /// The exec form's reply, whatever its exit status: "taken" comes with
    /// status 1, and since B136 a non-zero status throws — the marker went
    /// unread and the person heard "couldn't reach the host" (B240). A
    /// channel that ended any other way still fails as it did.
    private func editReply(_ command: String) async throws -> Data {
        do {
            return try await executeCommand(command)
        } catch SSHError.underlying(let inner) {
            guard case .exited(_, let output)? = inner as? SSHExec.Failure else { throw SSHError.underlying(inner) }
            return output
        }
    }

    private static func checkEditReply(_ reply: Data) throws {
        let text = String(decoding: reply, as: UTF8.self)
        if text.contains(RemoteFileEdits.takenMarker) { throw SSHError.underlying(RemoteEditError.nameTaken) }
        guard text.contains(RemoteFileEdits.okMarker) else { throw SSHError.underlying(RemoteEditError.failed) }
    }

    func createRemoteDirectory(_ path: String) async throws {
        try await edit(sftp: { try await $0.createDirectory(atPath: path) },
                       exec: RemoteFileEdits.makeDirectory(path))
    }

    /// Never overwrites: OpenSSH's SFTP rename refuses an existing target,
    /// and the exec form checks first.
    func renameRemoteItem(_ path: String, to newPath: String) async throws {
        try await edit(sftp: { sftp in
            if (try? await sftp.getAttributes(at: newPath)) != nil {
                throw SSHError.underlying(RemoteEditError.nameTaken)
            }
            try await sftp.rename(at: path, to: newPath)
        }, exec: RemoteFileEdits.rename(path, to: newPath))
    }

    /// A file goes through SFTP; a folder, with everything in it, through
    /// `rm -rf` — SFTP can only remove an empty one.
    func deleteRemoteItem(_ entry: RemoteFileEntry) async throws {
        guard let command = RemoteFileEdits.remove(entry.path, isDirectory: entry.isDirectory) else {
            throw SSHError.underlying(RemoteEditError.failed)
        }
        if entry.isDirectory {
            guard !closed else { throw SSHError.sessionClosed }
            try Self.checkEditReply(try await editReply(command))
        } else {
            try await edit(sftp: { try await $0.remove(at: entry.path) }, exec: command)
        }
    }

    /// Streams `local` to `remotePath`, which must not exist yet (SFTP's
    /// exclusive create; the exec form checks first). A failed SFTP upload
    /// removes what it wrote before falling back.
    func uploadRemoteFile(from local: URL, to remotePath: String,
                          progress: (@Sendable (Double) -> Void)? = nil) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        let size = (try? FileManager.default.attributesOfItem(atPath: local.path)[.size] as? UInt64) ?? 0
        do {
            let sftp = try await previewReader()
            let file = try await sftp.openFile(filePath: remotePath, flags: [.write, .create, .forceCreate])
            do {
                let handle = try FileHandle(forReadingFrom: local)
                defer { try? handle.close() }
                var offset: UInt64 = 0
                while let chunk = try handle.read(upToCount: 128 * 1024), !chunk.isEmpty {
                    try Task.checkCancellation()
                    try await file.write(ByteBuffer(bytes: chunk), at: offset)
                    offset += UInt64(chunk.count)
                    if size > 0 { progress?(min(1, Double(offset) / Double(size))) }
                }
                try await file.close()
                return
            } catch {
                try? await file.close()
                try? await sftp.remove(at: remotePath)
                throw error
            }
        } catch is CancellationError {
            throw CancellationError()
        } catch {}

        guard size <= RemoteFileCommands.execDownloadLimit else {
            throw SSHError.underlying(RemoteReadError.tooLargeWithoutSFTP)
        }
        let data = try Data(contentsOf: local)
        let plan = RemoteFileEdits.upload(to: remotePath)
        try Self.checkEditReply(try await editReply(plan.begin))
        var offset = 0
        while offset < data.count {
            try Task.checkCancellation()
            let end = min(offset + ExecUploadCommands.chunkBytes, data.count)
            _ = try await executeCommand(plan.append(data[offset..<end].base64EncodedString()))
            offset = end
            progress?(0.95 * Double(offset) / Double(max(data.count, 1)))
        }
        try Self.checkEditReply(try await editReply(plan.finish))
        progress?(1)
    }

    private static func entry(path: String, _ attributes: SFTPFileAttributes) -> RemoteFileEntry {
        // S_IFMT / S_IFDIR. A symlink reads as what readdir reported for the
        // link itself; a link to a previewable file still previews.
        let isDirectory = attributes.permissions.map { $0 & 0o170000 == 0o040000 } ?? false
        return RemoteFileEntry(path: path, isDirectory: isDirectory, size: attributes.size,
                               modified: attributes.accessModificationTime?.modificationTime)
    }

    func resize(rows: Int, cols: Int) async throws {
        guard !closed else { throw SSHError.sessionClosed }
        guard let writer else { throw SSHError.ptyNotReady }
        do {
            try await writer.resize(cols: cols, rows: rows)
        } catch {
            throw SSHError.underlying(error)
        }
    }

    func close() async {
        // The PTY channel may have ended on its own first (`markClosed`: the
        // remote shell exited, or the connection dropped). That ended the
        // channel, not the SSH connection under it — sshd keeps a connection
        // open after its shell exits, the client is the one to close it — so
        // the connection is still ours to shut. `close()` used to return here
        // when the channel had ended, and every session whose shell exited
        // left its connection ESTABLISHED for as long as the app ran.
        guard !transportShut else { return }
        transportShut = true
        closed = true
        // Ours to end: nobody is told (and a handler that holds on to its
        // listener lets go of it here).
        onUnexpectedClose = nil
        writer = nil
        continuation.finish()
        try? await previewSFTP?.close()
        previewSFTP = nil
        // Shutting the transport cancels in-flight PTY reads via channel
        // close → the read loop exits → its markClosed is a no-op.
        await transport.shutdown()
    }

    // MARK: - Private actor-isolated helpers

    private func installWriter(_ w: any SSHPTYWriter) {
        self.writer = w
    }

    private func markClosed(_ end: SSHChannelEnd) {
        guard !closed else { return }
        closed = true
        channelEnd = end
        writer = nil
        continuation.finish()
        // Reached only when the channel ended on its own — `close()` flips
        // `closed` first, so the read loop's markClosed is a no-op there.
        let handler = onUnexpectedClose
        onUnexpectedClose = nil
        handler?(end)
    }
}

/// How a PTY channel's read loop ended: the one thing that tells a remote
/// shell that EXITED — `exit`, Ctrl-D, its last process gone — from a
/// connection that DROPPED. Both used to look the same (the read loop ends),
/// and both were redialled: `exit` in a plain SSH shell came straight back
/// as a fresh login.
enum SSHChannelEnd: Equatable, Sendable {
    /// The server closed the channel in order: the command it ran (the login
    /// shell) ended, and sshd sent its exit status, EOF and close. `status`
    /// is the exit status when it was not 0; nil for 0, and for an orderly
    /// close that carried none (a shell killed by a signal: Citadel does not
    /// report `exit-signal`).
    case exited(status: Int?)
    /// The channel went down with the connection under it — TCP reset or EOF,
    /// an I/O error, a server that vanished mid-session — or never opened.
    case dropped
}

// MARK: - Client transport seam

/// The slice of the SSH client that ``SSHSession`` consumes, as a protocol —
/// the seam that lets lifecycle tests run a real session over a scripted
/// fake: a hung exec on a half-open socket, a refused PTY, a dead writer —
/// the exact shapes behind this week's suspension/recovery bugs, none of
/// which loopback networking can produce. Production is
/// ``CitadelClientTransport``. SFTP is deliberately NOT part of the seam
/// (see `SSHSession.sftpClient`).
protocol SSHClientTransport: AnyObject, Sendable {
    /// One-shot exec channel: stderr merged, buffered to completion.
    func run(_ command: String) async throws -> Data
    /// Open the interactive PTY. Returns once the server accepted it,
    /// handing back the stdin writer; `onOutput` then streams host bytes for
    /// the channel's lifetime, and `onEnd` fires once when the read loop
    /// finishes, saying how (``SSHChannelEnd``); the session turns that into
    /// `markClosed`.
    func openPTY(rows: Int, cols: Int,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter
    /// The same, the channel's program `command` (run as `sh -c`, see
    /// ``Shell/portable(_:)``) instead of the login shell; nil: the login
    /// shell, as above.
    func openPTY(rows: Int, cols: Int, command: String?,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter
    /// Tear the connection down; must also end an open PTY's read loop
    /// (which makes the session's `close()` race-free against `markClosed`).
    func shutdown() async
    /// The address the TCP connection reached (a numeric host, an IPv6
    /// link-local one with its `%interface`), nil when unknown — see
    /// ``SSHSession/peerAddress``.
    var peerAddress: String? { get }
}

extension SSHClientTransport {
    var peerAddress: String? { nil }

    /// A fake host that models no programs: the channel is its shell either
    /// way. Hosts that tell a program from a shell implement it.
    func openPTY(rows: Int, cols: Int, command: String?,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        try await openPTY(rows: rows, cols: cols, onOutput: onOutput, onEnd: onEnd)
    }
}

/// The PTY stdin surface the session writes through.
protocol SSHPTYWriter: Sendable {
    func write(_ data: Data) async throws
    func resize(cols: Int, rows: Int) async throws
}

/// Production ``SSHClientTransport`` over a Citadel `SSHClient`.
final class CitadelClientTransport: SSHClientTransport {
    private let client: SSHClient
    /// The connection's TCP channel, for exec channels of our own.
    private let channel: Channel
    let peerAddress: String?

    init(client: SSHClient, channel: Channel) {
        self.client = client
        self.channel = channel
        self.peerAddress = Self.numericHost(channel.remoteAddress)
    }

    /// `address` as a host a socket can be pointed at: the numeric address,
    /// and for an IPv6 link-local one its interface too (`fe80::1%en0`) —
    /// without it the address leads nowhere.
    static func numericHost(_ address: SocketAddress?) -> String? {
        guard let address, let ip = address.ipAddress else { return nil }
        if case .v6(let v6) = address, v6.address.sin6_scope_id != 0 {
            var name = [CChar](repeating: 0, count: Int(IF_NAMESIZE) + 1)
            if if_indextoname(v6.address.sin6_scope_id, &name) != nil {
                return ip + "%" + String(cString: name)
            }
        }
        return ip
    }

    /// Not Citadel's `executeCommand`: it can't tell an exit status of 0 from
    /// none, and handed back what a command cut short had printed as its
    /// answer (`SSHExec`).
    func run(_ command: String) async throws -> Data {
        try await SSHExec.run(command, on: channel)
    }

    func openPTY(rows: Int, cols: Int,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
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

        // Handshake: return only after Citadel has accepted the PTY request
        // and handed us a writer, while the surrounding read loop keeps
        // running for the lifetime of the session. Bridged with a
        // CheckedContinuation; closing the client breaks `withPTY`'s read
        // loop, so no Task handle needs holding for cancellation.
        return try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<any SSHPTYWriter, Error>) in
            Task { [client] in
                var resumed = false
                // Decided by how the read loop itself ends, not by whether
                // `withPTY` throws: it closes the channel after the loop, and
                // closing one the server already closed throws too.
                var end = SSHChannelEnd.dropped
                do {
                    try await client.withPTY(request, environment: environment) { output, writer in
                        if !resumed {
                            resumed = true
                            continuation.resume(returning: CitadelPTYWriter(writer: writer))
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
                                    onOutput(Data(bytes))
                                }
                            }
                            end = Self.channelEnd(after: nil)
                        } catch {
                            end = Self.channelEnd(after: error)
                        }
                    }
                } catch {
                    if !resumed {
                        resumed = true
                        continuation.resume(throwing: error)
                    }
                }
                onEnd(end)
            }
        }
    }

    func openPTY(rows: Int, cols: Int, command: String?,
                 onOutput: @escaping @Sendable (Data) -> Void,
                 onEnd: @escaping @Sendable (SSHChannelEnd) -> Void) async throws -> any SSHPTYWriter {
        guard let command else {
            return try await openPTY(rows: rows, cols: cols, onOutput: onOutput, onEnd: onEnd)
        }
        // Citadel opens a PTY only with a shell request, and keeps its
        // writer's initializer to itself: the channel is opened here, as
        // `SSHExec` opens its own.
        return try await SSHCommandPTY.open(
            command, rows: rows, cols: cols, on: channel,
            environment: [("LANG", "en_US.UTF-8")],
            onOutput: onOutput, onEnd: onEnd)
    }

    /// What a PTY read loop that ended with `error` (nil: it finished without
    /// one) says about the channel. Citadel finishes the output stream once
    /// the channel is gone: without an error when it was closed in order —
    /// sshd's close after an exit status of 0 or none — throwing
    /// `CommandFailed` for a non-zero exit status, and throwing whatever took
    /// it down otherwise: `NIOSSHError.tcpShutdown` when the connection under
    /// it closed, an I/O error, a channel failure. Only the first two mean
    /// the server ended the shell; the rest is a connection worth redialling.
    static func channelEnd(after error: Error?) -> SSHChannelEnd {
        guard let error else { return .exited(status: nil) }
        if let failed = error as? SSHClient.CommandFailed {
            return .exited(status: failed.exitCode)
        }
        return .dropped
    }

    func shutdown() async {
        try? await client.close()
    }
}

private struct CitadelPTYWriter: SSHPTYWriter {
    let writer: TTYStdinWriter

    func write(_ data: Data) async throws {
        var buffer = ByteBuffer()
        buffer.writeBytes(data)
        try await writer.write(buffer)
    }

    func resize(cols: Int, rows: Int) async throws {
        try await writer.changeSize(cols: cols, rows: rows, pixelWidth: 0, pixelHeight: 0)
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
    /// Asks the user for the passphrase of the key `connection` signs in
    /// with; `wrong` when the last one given didn't unlock it. nil = cancel.
    typealias KeyPassphraseHandler = @Sendable (_ connection: ServerConnection, _ wrong: Bool) async -> String?

    private let keychain: KeychainService
    private let validator: HostKeyValidator
    private let clientProvider: SSHClientProvider
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
         clientProvider: SSHClientProvider = CitadelSSHClientProvider(),
         protectedDataAvailable: @escaping @Sendable () async -> Bool = SSHService.systemProtectedData) {
        self.keychain = keychain
        self.validator = hostKeyValidator
        self.clientProvider = clientProvider
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
    ///
    /// `onKeyPassphrase` is asked when the connection's key was stored behind
    /// its passphrase (``StoredKey``); left out, such a key fails with
    /// ``SSHError/keyNeedsPassphrase``.
    func connect(_ connection: ServerConnection,
                 overrideSecret: String? = nil,
                 onUnknownHost: @escaping TrustNewHostHandler = { _, _, _ in false },
                 onChangedHost: @escaping AcceptChangedHostHandler = { _, _, _, _ in false },
                 onKeyPassphrase: @escaping KeyPassphraseHandler = { _, _ in nil }) async throws -> SSHSession {
        let attempts: [SignInAttempt]
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
                    throw await credentialReadFailure(error)
                }
                seBlobCache[connection.id] = blob
            }
            let seKey: SecureEnclave.P256.Signing.PrivateKey
            do {
                seKey = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
            } catch {
                throw await secureEnclaveFailure(error)
            }
            attempts = [SignInAttempt(secureEnclave: true) { [username = connection.username] in
                // NIOSSH signs the offer inside the handshake, where a chip
                // that refuses (the device locked) read as a failed sign-in:
                // the chip is asked first, and its refusal is the lock's
                // (B249).
                .custom(PublicKeyOfferDelegate(
                    username: username, key: NIOSSHPrivateKey(secureEnclaveP256Key: seKey),
                    canSign: { _ = try seKey.signature(for: Data("moshpit".utf8)) }))
            }]
        } else {
            var secret = try await resolveSecret(for: connection, override: overrideSecret)
            if connection.authMethod == .key {
                secret = try await signableKey(secret, for: connection, keep: overrideSecret == nil,
                                               onPassphrase: onKeyPassphrase)
            }
            attempts = try signInAttempts(connection: connection, secret: secret)
        }

        let hostKeyValidator = SSHHostKeyValidator.custom(TOFUHostKeyDelegate(
            validator: validator, host: connection.host, port: connection.port))

        // One connection per attempt: Citadel gives a sign-in method a single
        // offer. Only a refused sign-in moves on to the next attempt — the
        // next signature an RSA key can make; anything else (no route, a
        // server that takes no keys) is the answer. A key the user has to
        // judge ends its connection first (`HostKeyNeedsDecision`): the
        // question is put with nothing open and no clock running — Citadel's
        // 10-second sign-in, the server's login grace — and a trusted key is
        // dialled again.
        var index = 0
        var questions = 0
        while true {
            let attempt = attempts[index]
            do {
                let method: SSHAuthenticationMethod
                do {
                    method = try attempt.method()
                } catch {
                    throw SSHError.underlying(error)
                }
                let signedIn = try await dial(connection, authenticationMethod: method,
                                              hostKeyValidator: hostKeyValidator)
                if let signature = attempt.rsaSignature {
                    rsaSignatureThatWorked[Self.hostKey(connection)] = signature
                }
                return SSHSession(connection: connection, client: signedIn.client, channel: signedIn.channel)
            } catch let decision as HostKeyNeedsDecision {
                // A server whose key keeps changing under the question gets
                // asked about it a few times, not forever.
                questions += 1
                let trusted: Bool
                if questions > 3 {
                    trusted = false
                } else if let previous = decision.previous {
                    trusted = await onChangedHost(decision.host, decision.port, decision.fingerprint, previous)
                } else {
                    trusted = await onUnknownHost(decision.host, decision.port, decision.fingerprint)
                }
                guard trusted else { throw SSHError.map(HostKeyRejected()) }
                await validator.trust(host: decision.host, port: decision.port, fingerprint: decision.fingerprint)
            } catch {
                if attempt.secureEnclave,
                   let chip = SSHError.secureEnclaveSignIn(failedWith: error,
                                                           protectedDataAvailable: await protectedDataAvailable()) {
                    if case .credentialLocked = chip {
                        Log.ssh.notice("secure enclave: the sign-in's signature was refused while the device is locked (\(String(describing: error), privacy: .public)) — retried on the unlock")
                    }
                    throw chip
                }
                let mapped = SSHError.map(error)
                if case .authenticationFailed = mapped, index < attempts.count - 1 {
                    index += 1
                    continue
                }
                throw mapped
            }
        }
    }

    /// One way of signing in, tried on a connection of its own.
    struct SignInAttempt {
        /// For an RSA key, the signature it signs with; nil for any other.
        var rsaSignature: RSASignatureAlgorithm?
        /// Signed by a Secure Enclave key, whose chip can refuse.
        var secureEnclave = false
        /// A fresh method each time: Citadel's are used up by one connection.
        let method: () throws -> SSHAuthenticationMethod

        init(rsaSignature: RSASignatureAlgorithm? = nil, secureEnclave: Bool = false,
             method: @escaping () throws -> SSHAuthenticationMethod) {
            self.rsaSignature = rsaSignature
            self.secureEnclave = secureEnclave
            self.method = method
        }
    }

    /// The RSA signature that last signed in to a host ("host:port"), tried
    /// first next time — what a host takes doesn't change between reconnects.
    private var rsaSignatureThatWorked: [String: RSASignatureAlgorithm] = [:]

    private static func hostKey(_ connection: ServerConnection) -> String {
        "\(connection.host):\(connection.port)"
    }

    /// Dial `connection`'s host — through its SOCKS proxy when it has one —
    /// and sign in with `authenticationMethod`.
    private func dial(_ connection: ServerConnection,
                      authenticationMethod: SSHAuthenticationMethod,
                      hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
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
            return try await clientProvider.connect(
                on: channel,
                authenticationMethod: authenticationMethod,
                hostKeyValidator: hostKeyValidator
            )
        }
        return try await clientProvider.connect(
            host: connection.host,
            port: connection.port,
            authenticationMethod: authenticationMethod,
            hostKeyValidator: hostKeyValidator
        )
    }

    // MARK: - Helpers

    /// The key `stored` as the connect path signs with it (``StoredKey``).
    ///
    /// A key kept as it was pasted — PEM, behind a passphrase, Windows line
    /// endings — is converted on its first use, asking for the passphrase
    /// first when it has one, and once the converted key has read back to
    /// the same public key it takes the original's place in the keychain
    /// (`keep`; not for a secret handed in for this call only). Until that
    /// replacement has succeeded the stored item is left as it was: no step
    /// of this can lose the key. A replacement that fails costs nothing but
    /// doing this again next time.
    private func signableKey(_ stored: String, for connection: ServerConnection, keep: Bool,
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
                if keep {
                    await keepConverted(converted, replacing: stored, for: connection)
                }
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
    /// cache it: a connect while this foreground lasts (the mosh sidecar's,
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

    private func resolveSecret(for connection: ServerConnection,
                               override: String?) async throws -> String {
        if let override { return override }
        #if DEBUG
        if let seeded = launchSecrets[connection.id] { return seeded }
        #endif
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

    /// How `connection` signs in with `secret`, in the order to try: one way
    /// for a password or most keys; for an RSA key, one per signature it can
    /// make (``RSASignatureAlgorithm``), since a server refuses the ones it
    /// doesn't take only by refusing the sign-in.
    func signInAttempts(connection: ServerConnection, secret: String) throws -> [SignInAttempt] {
        let username = connection.username
        switch connection.authMethod {
        case .password:
            return [SignInAttempt { .passwordBased(username: username, password: secret) }]

        case .key:
            // The key's numbers come from the key reader every path shares
            // (`moshpit_ssh::keyfile`): `signableKey` has made sure it reads.
            let parts: SigningParts
            do {
                parts = try SSHKeyFactory.signingParts(of: secret)
            } catch {
                throw SSHError.underlying(error)
            }
            switch parts {
            case .rsa(let n, let e, let d, let p, let q):
                // Citadel signs RSA with SHA-1 only, which OpenSSH 8.8 and
                // later refuse: the SHA-2 signatures come from the key's own
                // numbers (RSASHA2Signing.swift), SHA-1 stays Citadel's.
                let key = RSAKeySignIn(username: username, n: n, e: e, d: d, p: p, q: q, openSSHKey: secret)
                let order = RSASignatureAlgorithm.order(worked: rsaSignatureThatWorked[Self.hostKey(connection)])
                return order.map { algorithm in
                    SignInAttempt(rsaSignature: algorithm) { try key.method(algorithm) }
                }
            case .ed25519(let seed):
                do {
                    let key = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
                    return [SignInAttempt { .ed25519(username: username, privateKey: key) }]
                } catch {
                    throw SSHError.underlying(error)
                }
            case .ecdsa(let curve, let scalar):
                // B126: P-384 and P-521 sign in too.
                do {
                    switch curve {
                    case .p256:
                        let key = try P256.Signing.PrivateKey(rawRepresentation: scalar)
                        return [SignInAttempt { .p256(username: username, privateKey: key) }]
                    case .p384:
                        let key = try P384.Signing.PrivateKey(rawRepresentation: scalar)
                        return [SignInAttempt { .p384(username: username, privateKey: key) }]
                    case .p521:
                        let key = try P521.Signing.PrivateKey(rawRepresentation: scalar)
                        return [SignInAttempt { .p521(username: username, privateKey: key) }]
                    }
                } catch {
                    throw SSHError.underlying(error)
                }
            }
        }
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

// MARK: - Exec-channel upload commands

/// The command lines for the no-SFTP upload fallback. Pure builders, unit
/// tested — a quoting bug here writes bytes to the wrong path on someone
/// else's server, so every path goes through one shell-quoter and the tests
/// pin the exact strings.
///
/// Strict POSIX sh: the exec channel runs a non-login shell on whatever
/// /bin/sh the host has (see HostCapabilities' PATH notes), and `base64 -d`
/// vs `-D` differs across ages of BSD — the finish line tries both orders.
enum ExecUploadCommands {
    /// Raw bytes per exec round trip. 48KB of payload is 64KB of base64 —
    /// comfortably inside every sshd/shell argument limit encountered in
    /// the wild, while keeping a 2MB image to ~45 round trips.
    static let chunkBytes = 48 * 1024

    static let successMarker = "MOSHPIT_UPLOAD_OK"

    struct Plan {
        let begin: String
        let append: (String) -> String
        let finish: String
        let finalPath: String
    }

    static func plan(home: String, filename: String) -> Plan {
        let dirParent = home + "/.moshpit"
        let dir = dirParent + "/uploads"
        let finalPath = dir + "/" + filename
        let tempPath = finalPath + ".b64part"
        let qDirParent = quote(dirParent), qDir = quote(dir)
        let qFinal = quote(finalPath), qTemp = quote(tempPath)
        return Plan(
            begin: "mkdir -p \(qDir) && chmod 700 \(qDirParent) \(qDir) && rm -f \(qTemp)",
            // The base64 alphabet (A–Za–z0–9+/=) is inert inside single
            // quotes — no escaping needed for the payload itself.
            append: { chunk in "printf %s '\(chunk)' >> \(qTemp)" },
            finish: "{ base64 -d < \(qTemp) > \(qFinal) 2>/dev/null"
                + " || base64 -D < \(qTemp) > \(qFinal); }"
                + " && chmod 600 \(qFinal) && rm -f \(qTemp) && echo \(successMarker)",
            finalPath: finalPath)
    }

    /// POSIX single-quoting: close, escaped quote, reopen. The only
    /// character that needs anything is the quote itself. The app's one
    /// quoter, ``Shell/quote(_:)``: file names in a host's listing are the
    /// host's to choose.
    static func quote(_ path: String) -> String {
        Shell.quote(path)
    }
}

extension RemoteFileHandle {
    /// A file held open over this app's own SFTP (`SSHSession.openRemoteFile`).
    fileprivate convenience init(file: SFTPFile, size: UInt64) {
        let held = HeldSFTPFile(file)
        self.init(size: size, read: { offset, length in
            var buffer = try await held.file.read(from: offset, length: length)
            return buffer.readBytes(length: buffer.readableBytes) ?? []
        }, close: {
            try? await held.file.close()
        })
    }
}

/// Citadel's file, for the handle's closures (reads overlap on the wire;
/// Citadel serialises nothing here that the old class didn't).
private final class HeldSFTPFile: @unchecked Sendable {
    let file: SFTPFile
    init(_ file: SFTPFile) { self.file = file }
}

/// What the features beside a terminal reach the host through: this app's
/// own connection is one kind (the Swift path's), the Rust session's lent
/// connection the other (``BorrowedHost``).
extension SSHSession: HostConnection {}

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
    /// The credential the Rust core signs in to `connection` with: the same
    /// keychain read the Swift connect makes — Face ID when the item asks for
    /// it, then the in-memory cache (one prompt per foreground, shared by both
    /// paths) — and the same conversion of a key kept as it was pasted, its
    /// passphrase asked through `onKeyPassphrase`. A Secure Enclave key gives
    /// its public half; its signatures come from the chip, never its key.
    func rustCoreCredential(_ connection: ServerConnection,
                            onKeyPassphrase: @escaping KeyPassphraseHandler) async throws -> RustCoreCredential {
        if connection.authMethod == .key, connection.sshKeyAlgorithm == .seP256 {
            guard let ref = connection.keychainRef else { throw SSHError.missingKeychainRef }
            let key = try await secureEnclaveKey(for: connection, ref: ref)
            return .secureEnclave(publicKey: Self.openSSHLine(of: key.publicKey), keyRef: ref)
        }
        let secret = try await resolveSecret(for: connection, override: nil)
        switch connection.authMethod {
        case .password:
            return .password(secret)
        case .key:
            return .key(try await signableKey(secret, for: connection, keep: true,
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

    /// The host key the person trusted on the Rust core's card, trusted here
    /// too: the core wrote it to the keychain item already, and this
    /// process's Swift copy of the map must not write over it with a trust
    /// of its own (``HostKeyValidator`` merges onto what it read once).
    func noteTrustedByRustCore(host: String, port: Int, fingerprint: String) async {
        await validator.trust(host: host, port: port, fingerprint: fingerprint)
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
