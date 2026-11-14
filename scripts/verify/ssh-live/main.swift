// ssh-live: the app's sign-in and connect code against a real OpenSSH server
// (scripts/verify/ssh-live/run.sh starts a private one). One command per run.
//
//   ssh-live genkey <out>
//       the app's own RSA-4096 key (SSHKeyFactory.generate) to <out>, its
//       authorized_keys line to <out>.pub
//   ssh-live signin <host> <port> <user> <key file> <how>
//       sign in with an RSA key, run `echo signed-in`, report. <how>:
//         citadel        what the app did before: Citadel's own RSA sign-in
//         rsa-sha2-512, rsa-sha2-256, ssh-rsa
//                        one attempt, that signature (RSASHA2Signing.swift)
//         app            the app's order (RSASignatureAlgorithm.order()), a
//                        connection each, until one signs in
//       prints `signed-in <how it signed> <output>` or `refused <tried>`
//   ssh-live socks-signin <proxy port> <host> <port> <user> <ed25519 key> <how>
//       sign in through the SOCKS proxy on 127.0.0.1:<proxy port>, run `echo
//       signed-in`. <how>:
//         old     what the app did: SOCKSProxyDialer, then Citadel's
//                 SSHClient.connect(on:authenticationMethod:…)
//         app     SOCKSProxyDialer, then SSHDial.signIn(on:…)
//         direct  no proxy: Citadel's own dial, for comparison
//       prints `signed-in <output>`, `connected` (when the run fails after
//       the connect returned), or `error <error>`
//   ssh-live loopback
//       the unit tests' in-process SSH server (Tests/MoshpitTests/Support/
//       LoopbackSSHServer.swift), checked from here: signing in directly and
//       through its SOCKS front, a wrong password, and its scripted execs
//   ssh-live prompts
//       the app's SSHService.connect against the in-process server: the
//       new-host card answered after 12 s, a refused sign-in, a declined key,
//       a changed key, a trusted host — what was asked, what stayed open
//   ssh-live sshd-prompt <port> <user> <key file> <seconds>
//       SSHService.connect to 127.0.0.1:<port> with a key, the new-host card
//       answered (yes) after <seconds>
//   ssh-live citadel-slow-trust <port> <user> <key file> <seconds>
//       before: Citadel's own connect, its host key check answered (yes)
//       after <seconds> from inside the handshake; stays up <seconds> + 4 s
//       so the server can show what became of the connection
//   ssh-live stored-key <port> <user> <key file> [<passphrase>]
//       a key stored exactly as the file has it (as pasted), signed in with
//       through the app's SSHService.connect — the passphrase, if any, given
//       when the connect asks; then what the keychain holds
//   ssh-live locked-keychain <port> <user> <key file> locked|unlocked
//       B211: the key stored behind a keychain that refuses as a locked
//       device's does (errSecInteractionNotAllowed, after a securityd round
//       trip) or gives it, signed in with through the app's SSHService.connect;
//       prints `connected: <output>` or `failed: <the SSHError> after <ms> — <its words>`
//   ssh-live exec-ends <port> <user> <key file>
//       commands that end every way — exit 0, exit 3, killed by SIGKILL and
//       SIGTERM — run through Citadel's executeCommand (before) and the
//       app's SSHSession.executeCommand (after)
//   ssh-live side-features <port> <user> <key file> <dir> <rounds>
//       W14b's baseline: per round, what a feature beside a Rust session did
//       before it borrowed the session's connection — the app's SSHService
//       connect and sign-in of its own — then a probe, a preview (stat and
//       download of <dir>/chart.png), a stream's first MiB of <dir>/clip.mp4,
//       a listing of <dir> and an upload; prints each step's p50
//   ssh-live socks-fake <script>
//       the app's SOCKSProxyDialer against a scripted fake proxy
//       (FakeSOCKS.swift; `unreachable` = a port nobody listens on); prints
//       `connected`, `failed <error>` or `hung` (nothing after 20 s), and
//       the seconds it took

import Citadel
import Crypto
import Foundation
import NIOConcurrencyHelpers
import NIOCore
import NIOPosix
import NIOSSH
import Security

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(2)
}

/// One connection, signed in with `method`; nil when the server refused the
/// sign-in, the error otherwise.
func signIn(host: String, port: Int, method: SSHAuthenticationMethod) async throws -> SSHClient? {
    do {
        return try await SSHClient.connect(
            host: host, port: port, authenticationMethod: method,
            hostKeyValidator: .acceptAnything(), reconnect: .never, connectTimeout: .seconds(15))
    } catch SSHClientError.allAuthenticationOptionsFailed {
        return nil
    }
}

func run(_ client: SSHClient) async throws -> String {
    let output = try await client.executeCommand("echo signed-in", mergeStreams: true)
    return String(buffer: output).trimmingCharacters(in: .whitespacesAndNewlines)
}

// Unbuffered: a run that hangs and is killed still shows how far it got.
setvbuf(stdout, nil, _IONBF, 0)
let arguments = Array(CommandLine.arguments.dropFirst())
guard let command = arguments.first else { fail("usage: see main.swift") }

switch command {
case "genkey":
    guard arguments.count == 2 else { fail("genkey <out>") }
    let key = try SSHKeyFactory.generate(algorithm: .rsa4096, comment: "moshpit-app-rsa4096")
    try key.privateBlob.write(to: URL(fileURLWithPath: arguments[1]))
    try Data((key.publicKeyLine + "\n").utf8).write(to: URL(fileURLWithPath: arguments[1] + ".pub"))
    print("\(key.algorithm.rawValue) \(key.bits ?? 0) \(key.fingerprint)")

case "signin":
    guard arguments.count == 6, let port = Int(arguments[2]) else {
        fail("signin <host> <port> <user> <key file> <how>")
    }
    let (host, user, how) = (arguments[1], arguments[3], arguments[5])
    let text = try String(contentsOfFile: arguments[4], encoding: .utf8)
    // What the app stores: the key as an unencrypted openssh-key-v1 file,
    // its numbers read as SSHService reads them (the Rust key reader).
    guard case .rsa(let n, let e, let d, let p, let q) = try SSHKeyFactory.signingParts(of: text) else {
        fail("not an RSA key")
    }
    let stored = text
    let key = RSAKeySignIn(username: user, n: n, e: e, d: d, p: p, q: q, openSSHKey: stored)

    var tried: [String] = []
    let attempts: [(String, () throws -> SSHAuthenticationMethod)]
    switch how {
    case "citadel":
        attempts = [("citadel", { .rsa(username: user, privateKey: try Insecure.RSA.PrivateKey(sshRsa: stored)) })]
    case "app":
        attempts = RSASignatureAlgorithm.order().map { algorithm in (algorithm.rawValue, { try key.method(algorithm) }) }
    default:
        guard let algorithm = RSASignatureAlgorithm(rawValue: how) else { fail("unknown <how> \(how)") }
        attempts = [(how, { try key.method(algorithm) })]
    }
    for (name, method) in attempts {
        tried.append(name)
        do {
            guard let client = try await signIn(host: host, port: port, method: try method()) else { continue }
            let output = try await run(client)
            try? await client.close()
            print("signed-in \(name) \(output)")
            exit(0)
        } catch {
            print("error \(name) \(String(reflecting: error))")
            exit(1)
        }
    }
    print("refused \(tried.joined(separator: ","))")
    exit(1)

case "socks-signin":
    guard arguments.count == 7, let proxyPort = Int(arguments[1]), let port = Int(arguments[3]) else {
        fail("socks-signin <proxy port> <host> <port> <user> <ed25519 key> <how>")
    }
    let (host, user, how) = (arguments[2], arguments[4], arguments[6])
    let key = try Curve25519.Signing.PrivateKey(sshEd25519: try String(contentsOfFile: arguments[5], encoding: .utf8))
    let method = SSHAuthenticationMethod.ed25519(username: user, privateKey: key)
    let client: SSHClient
    do {
        switch how {
        case "direct":
            client = try await SSHClient.connect(host: host, port: port, authenticationMethod: method,
                                                 hostKeyValidator: .acceptAnything(), reconnect: .never)
        case "old":
            let channel = try await SOCKSProxyDialer.connect(proxyHost: "127.0.0.1", proxyPort: proxyPort,
                                                             targetHost: host, targetPort: port)
            client = try await SSHClient.connect(on: channel, authenticationMethod: method,
                                                 hostKeyValidator: .acceptAnything())
        case "app":
            let channel = try await SOCKSProxyDialer.connect(proxyHost: "127.0.0.1", proxyPort: proxyPort,
                                                             targetHost: host, targetPort: port)
            client = try await SSHDial.signIn(on: channel, authenticationMethod: method,
                                              hostKeyValidator: .acceptAnything())
        default:
            fail("unknown <how> \(how)")
        }
    } catch {
        print("error \(String(reflecting: error))")
        exit(1)
    }
    do {
        print("signed-in \(try await run(client))")
        exit(0)
    } catch {
        print("connected, then \(String(reflecting: error))")
        exit(1)
    }

case "loopback":
    func check(_ what: String, _ body: () async throws -> String) async {
        print("\(what) …")
        do { print("\(what): \(try await body())") } catch { print("\(what): error \(String(reflecting: error))") }
    }
    print("starting the servers")
    let execs: [String: LoopbackSSHServer.Exec] = [
        "echo hi": .exits(output: "hi\n", status: 0),
        "fails": .exits(output: "x", status: 3),
        "drops": .closes(output: "partial"),
        "killed": .killed(output: "partial", signal: "KILL"),
    ]
    let direct = try await LoopbackSSHServer.start(.init(execs: execs))
    let proxied = try await LoopbackSSHServer.start(.init(socks: true, execs: execs))
    await check("direct, right password") {
        let client = try await SSHClient.connect(host: "127.0.0.1", port: direct.port,
                                                 authenticationMethod: .passwordBased(username: "me", password: "right"),
                                                 hostKeyValidator: .acceptAnything(), reconnect: .never)
        var lines: [String] = []
        for command in ["echo hi", "fails", "drops", "killed"] {
            do {
                let out = try await client.executeCommand(command, mergeStreams: true)
                lines.append("\(command) -> ok \(String(buffer: out).debugDescription)")
            } catch {
                lines.append("\(command) -> error \(error)")
            }
        }
        try? await client.close()
        return lines.joined(separator: "; ")
    }
    await check("direct, wrong password") {
        _ = try await SSHClient.connect(host: "127.0.0.1", port: direct.port,
                                        authenticationMethod: .passwordBased(username: "me", password: "wrong"),
                                        hostKeyValidator: .acceptAnything(), reconnect: .never)
        return "connected"
    }
    await check("proxied, right password") {
        let channel = try await SOCKSProxyDialer.connect(proxyHost: "127.0.0.1", proxyPort: proxied.port,
                                                         targetHost: "ssh.example.com", targetPort: 22)
        let client = try await SSHDial.signIn(on: channel,
                                              authenticationMethod: .passwordBased(username: "me", password: "right"),
                                              hostKeyValidator: .acceptAnything())
        let out = try await client.executeCommand("echo hi", mergeStreams: true)
        try? await client.close()
        return "ok \(String(buffer: out).debugDescription)"
    }
    await check("proxied, wrong password") {
        let channel = try await SOCKSProxyDialer.connect(proxyHost: "127.0.0.1", proxyPort: proxied.port,
                                                         targetHost: "ssh.example.com", targetPort: 22)
        _ = try await SSHDial.signIn(on: channel,
                                     authenticationMethod: .passwordBased(username: "me", password: "wrong"),
                                     hostKeyValidator: .acceptAnything())
        return "connected"
    }
    await check("the app's session") {
        let session = try await SSHService(keychain: .inMemory(), hostKeyValidator: .inMemory()).connect(
            ServerConnection(name: "t", host: "127.0.0.1", port: direct.port, username: "me",
                             authMethod: .password, keychainRef: nil),
            overrideSecret: "right", onUnknownHost: { _, _, _ in true })
        var lines: [String] = []
        for command in ["echo hi", "fails", "drops", "killed"] {
            do {
                let out = try await session.executeCommand(command)
                lines.append("\(command) -> ok \(String(decoding: out, as: UTF8.self).debugDescription)")
            } catch SSHError.underlying(let error) {
                lines.append("\(command) -> error \(error)")
            }
        }
        await session.close()
        return lines.joined(separator: "; ")
    }
    let closed = await direct.waitUntil(3) { $0.stillOpen == 0 }
    print("direct server: accepted \(direct.accepted), still open \(direct.stillOpen) (all closed: \(closed))")
    await direct.stop()
    await proxied.stop()
    exit(0)

case "prompts":
    let start = Date()
    func elapsed() -> String { String(format: "%.1f s", Date().timeIntervalSince(start)) }
    func connection(_ server: LoopbackSSHServer) -> ServerConnection {
        ServerConnection(name: "t", host: "127.0.0.1", port: server.port, username: "me",
                         authMethod: .password, keychainRef: nil)
    }
    // The new-host card, answered after 12 s.
    do {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory()
        let opened = NIOLockedValueBox(-1)
        let began = Date()
        do {
            let session = try await SSHService(keychain: .inMemory(), hostKeyValidator: validator).connect(
                connection(server), overrideSecret: "right",
                onUnknownHost: { _, _, _ in
                    _ = await server.waitUntil(3) { $0.stillOpen == 0 }
                    opened.withLockedValue { $0 = server.stillOpen }
                    try? await Task.sleep(for: .seconds(12))
                    return true
                })
            print("slow card: connected after \(String(format: "%.1f", Date().timeIntervalSince(began))) s; "
                  + "connections open while asking: \(opened.withLockedValue { $0 }); "
                  + "trusted: \(await validator.snapshot()["127.0.0.1:\(server.port)"] == server.fingerprint); "
                  + "accepted \(server.accepted)")
            await session.close()
        } catch {
            print("slow card: error \((error as? SSHError)?.diagnostic ?? String(reflecting: error)) after "
                  + String(format: "%.1f", Date().timeIntervalSince(began)) + " s")
        }
        await server.stop()
    }
    // A refused sign-in.
    do {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory(seed: ["127.0.0.1:\(server.port)": server.fingerprint])
        do {
            _ = try await SSHService(keychain: .inMemory(), hostKeyValidator: validator)
                .connect(connection(server), overrideSecret: "wrong")
            print("wrong password: connected?!")
        } catch {
            let closed = await server.waitUntil(3) { $0.stillOpen == 0 }
            print("wrong password: \((error as? SSHError)?.diagnostic ?? "\(error)"); closed afterwards: \(closed)")
        }
        await server.stop()
    }
    // A declined key.
    do {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory()
        do {
            _ = try await SSHService(keychain: .inMemory(), hostKeyValidator: validator)
                .connect(connection(server), overrideSecret: "right", onUnknownHost: { _, _, _ in false })
            print("declined: connected?!")
        } catch {
            let closed = await server.waitUntil(3) { $0.stillOpen == 0 }
            print("declined: \((error as? SSHError)?.diagnostic ?? "\(error)"); trusted: "
                  + "\(await validator.snapshot()); accepted \(server.accepted); closed: \(closed)")
        }
        await server.stop()
    }
    // A changed key.
    do {
        let server = try await LoopbackSSHServer.start()
        let validator = HostKeyValidator.inMemory(seed: ["127.0.0.1:\(server.port)": "SHA256:old"])
        let asked = NIOLockedValueBox("")
        let session = try await SSHService(keychain: .inMemory(), hostKeyValidator: validator).connect(
            connection(server), overrideSecret: "right",
            onChangedHost: { _, _, new, old in asked.withLockedValue { $0 = "\(old) → \(new)" }; return true })
        print("changed key: asked \(asked.withLockedValue { $0 } == "SHA256:old → \(server.fingerprint)"); "
              + "now trusted: \(await validator.snapshot()["127.0.0.1:\(server.port)"] == server.fingerprint)")
        await session.close()
        await server.stop()
    }
    print("done in \(elapsed())")
    exit(0)

case "sshd-prompt", "citadel-slow-trust":
    guard arguments.count == 5, let port = Int(arguments[1]), let seconds = Double(arguments[4]) else {
        fail("\(command) <port> <user> <key file> <seconds>")
    }
    let (user, keyText) = (arguments[2], try String(contentsOfFile: arguments[3], encoding: .utf8))
    let began = Date()
    func at() -> String { String(format: "%.1f s", Date().timeIntervalSince(began)) }
    if command == "sshd-prompt" {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(keyText, forRef: ref, requireBiometry: false)
        let connection = ServerConnection(name: "t", host: "127.0.0.1", port: port, username: user,
                                          authMethod: .key, keychainRef: ref)
        do {
            let session = try await SSHService(keychain: keychain, hostKeyValidator: .inMemory()).connect(
                connection, onUnknownHost: { _, _, _ in
                    print("card up at \(at())")
                    try? await Task.sleep(for: .seconds(seconds))
                    print("trusted at \(at())")
                    return true
                })
            let output = try await session.executeCommand("echo signed-in")
            print("signed-in at \(at()): \(String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines))")
            await session.close()
            exit(0)
        } catch {
            print("error at \(at()): \((error as? SSHError)?.diagnostic ?? "\(error)")")
            exit(1)
        }
    }
    final class SlowYes: NIOSSHClientServerAuthenticationDelegate, @unchecked Sendable {
        let seconds: Double
        let began: Date
        init(seconds: Double, began: Date) { self.seconds = seconds; self.began = began }
        func validateHostKey(hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>) {
            Task {
                try? await Task.sleep(for: .seconds(seconds))
                print(String(format: "the host key check said yes at %.1f s", Date().timeIntervalSince(began)))
                validationCompletePromise.succeed(())
            }
        }
    }
    let key = try Curve25519.Signing.PrivateKey(sshEd25519: keyText)
    do {
        _ = try await SSHClient.connect(host: "127.0.0.1", port: port,
                                        authenticationMethod: .ed25519(username: user, privateKey: key),
                                        hostKeyValidator: .custom(SlowYes(seconds: seconds, began: began)),
                                        reconnect: .never, connectTimeout: .seconds(15))
        print("signed-in at \(at())")
    } catch {
        print("error at \(at()): \(String(reflecting: error))")
    }
    try? await Task.sleep(for: .seconds(seconds + 4 - Date().timeIntervalSince(began)))
    exit(0)

case "stored-key":
    guard arguments.count == 4 || arguments.count == 5, let port = Int(arguments[1]) else {
        fail("stored-key <port> <user> <key file> [<passphrase>]")
    }
    let pasted = try String(contentsOfFile: arguments[3], encoding: .utf8)
    let passphrase = arguments.count == 5 ? arguments[4] : nil
    // Every connect path reads it as stored: StoredKey's `.usable`.
    func readsAsStored(_ key: String) -> Bool { (try? StoredKey.assess(key)) == .usable }
    // The key's public half, decrypted with `passphrase` when it needs one.
    func fingerprint(_ key: String, _ passphrase: String?) -> String? {
        try? SSHKeyFactory.importKey(privatePEM: key, passphrase: passphrase, publicKeyLine: nil, comment: "").fingerprint
    }
    print("as stored, the connect path reads it: \(readsAsStored(pasted))")
    let keychain = KeychainService.inMemory()
    let ref = keychain.generateRef()
    try await keychain.savePrivateKey(pasted, forRef: ref, requireBiometry: false)
    let connection = ServerConnection(name: "t", host: "127.0.0.1", port: port, username: arguments[2],
                                      authMethod: .key, keychainRef: ref)
    let asked = NIOLockedValueBox(0)
    do {
        let session = try await SSHService(keychain: keychain, hostKeyValidator: .inMemory()).connect(
            connection, onUnknownHost: { _, _, _ in true },
            onKeyPassphrase: { _, _ in asked.withLockedValue { $0 += 1 }; return passphrase })
        let output = try await session.executeCommand("echo signed-in")
        await session.close()
        let now = try await keychain.loadPrivateKey(forRef: ref)
        let same = fingerprint(now, nil) == fingerprint(pasted, passphrase)
        print("\(String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)); "
              + "passphrase asked \(asked.withLockedValue { $0 })×; keychain now "
              + (now == pasted ? "as pasted" : "converted, read as stored: \(readsAsStored(now)), same key: \(same)"))
        exit(0)
    } catch {
        print("error: \((error as? SSHError)?.description ?? String(reflecting: error))")
        exit(1)
    }

case "locked-keychain":
    guard arguments.count == 5, let port = Int(arguments[1]),
          ["locked", "unlocked"].contains(arguments[4]) else {
        fail("locked-keychain <port> <user> <key file> locked|unlocked")
    }
    let pasted = try String(contentsOfFile: arguments[3], encoding: .utf8)
    /// The in-memory keychain, behind the lock the real one has.
    final class LockableKeychain: KeychainBackend, @unchecked Sendable {
        let inner = InMemoryKeychainBackend()
        let locked: NIOLockedValueBox<Bool>
        init(locked: Bool) { self.locked = NIOLockedValueBox(locked) }
        func save(secret: Data, account: String, requireBiometry: Bool) throws {
            try inner.save(secret: secret, account: account, requireBiometry: requireBiometry)
        }
        func load(account: String, prompt: String) throws -> Data {
            Thread.sleep(forTimeInterval: 0.015)
            if locked.withLockedValue({ $0 }) { throw KeychainError(osStatus: errSecInteractionNotAllowed) }
            return try inner.load(account: account, prompt: prompt)
        }
        func delete(account: String) throws { try inner.delete(account: account) }
        func exists(account: String) -> Bool { inner.exists(account: account) }
    }
    let keychain = LockableKeychain(locked: arguments[4] == "locked")
    try keychain.save(secret: Data(pasted.utf8), account: "moshpit.secret.b211", requireBiometry: false)
    let service = SSHService(keychain: KeychainService(backend: keychain), hostKeyValidator: .inMemory(),
                             protectedDataAvailable: { !keychain.locked.withLockedValue { $0 } })
    let connection = ServerConnection(name: "t", host: "127.0.0.1", port: port, username: arguments[2],
                                      authMethod: .key, keychainRef: "moshpit.secret.b211")
    let began = Date()
    do {
        let session = try await service.connect(connection, onUnknownHost: { _, _, _ in true })
        let output = try await session.executeCommand("echo signed-in")
        await session.close()
        print("connected: \(String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines))")
    } catch let error as SSHError {
        let kind: String
        switch error {
        case .credentialLocked: kind = "credentialLocked"
        case .credentialUnavailable: kind = "credentialUnavailable"
        default: kind = error.diagnostic
        }
        print("failed: \(kind) after \(Int(Date().timeIntervalSince(began) * 1000)) ms — \(error.description)")
    }
    exit(0)

case "exec-ends":
    guard arguments.count == 4, let port = Int(arguments[1]) else { fail("exec-ends <port> <user> <key file>") }
    let keyText = try String(contentsOfFile: arguments[3], encoding: .utf8)
    let commands = ["echo ok", "printf partial; exit 3", "printf partial; kill -KILL $$", "printf partial; kill -TERM $$"]
    let key = try Curve25519.Signing.PrivateKey(sshEd25519: keyText)
    let client = try await SSHClient.connect(host: "127.0.0.1", port: port,
                                             authenticationMethod: .ed25519(username: arguments[2], privateKey: key),
                                             hostKeyValidator: .acceptAnything(), reconnect: .never)
    for command in commands {
        do {
            let out = try await client.executeCommand(command, mergeStreams: true)
            print("before \(command): ok \(String(buffer: out).debugDescription)")
        } catch {
            print("before \(command): error \(error)")
        }
    }
    try? await client.close()
    let keychain = KeychainService.inMemory()
    let ref = keychain.generateRef()
    try await keychain.savePrivateKey(keyText, forRef: ref, requireBiometry: false)
    let session = try await SSHService(keychain: keychain, hostKeyValidator: .inMemory()).connect(
        ServerConnection(name: "t", host: "127.0.0.1", port: port, username: arguments[2],
                         authMethod: .key, keychainRef: ref),
        onUnknownHost: { _, _, _ in true })
    for command in commands {
        do {
            let out = try await session.executeCommand(command)
            print("after \(command): ok \(String(decoding: out, as: UTF8.self).debugDescription)")
        } catch SSHError.underlying(let error) {
            print("after \(command): error \(error)")
        } catch {
            print("after \(command): error \(error)")
        }
    }
    await session.close()
    exit(0)

case "socks-fake":
    guard arguments.count == 2 else { fail("socks-fake <script>") }
    var port: Int
    var proxy: FakeSOCKSProxy?
    if arguments[1] == "unreachable" {
        let free = try await FakeSOCKSProxy.start(.silent)
        port = free.port
        await free.stop()
    } else {
        guard let script = FakeSOCKSScript(rawValue: arguments[1]) else { fail("unknown script \(arguments[1])") }
        proxy = try await FakeSOCKSProxy.start(script)
        port = proxy!.port
    }
    let started = Date()
    // Not a task group: a group waits for every child, and a hung dial
    // never finishes. Whichever answers first wins; the process then exits
    // with the loser still parked.
    let outcome = await withCheckedContinuation { (continuation: CheckedContinuation<String, Never>) in
        let once = NIOLockedValueBox(false)
        let answer: @Sendable (String) -> Void = { value in
            if once.withLockedValue({ done in defer { done = true }; return !done }) {
                continuation.resume(returning: value)
            }
        }
        Task {
            do {
                let channel = try await SOCKSProxyDialer.connect(
                    proxyHost: "127.0.0.1", proxyPort: port, targetHost: "ssh.example.com", targetPort: 22)
                try? await channel.close()
                answer("connected")
            } catch {
                answer("failed \(error)")
            }
        }
        Task {
            try? await Task.sleep(for: .seconds(20))
            answer("hung")
        }
    }
    print(outcome + String(format: " (%.2f s)", Date().timeIntervalSince(started)))
    await proxy?.stop()
    exit(0)

case "side-features":
    // W14b's baseline: what a feature beside a Rust session did before it
    // borrowed the session's connection — `acquireFileTransferSSH`'s dial,
    // a Swift SSHService connect and sign-in of its own — then the calls the
    // features make on it. Per round: sign in, probe, preview (stat and
    // download), a stream's first MiB, a folder listing, an image upload.
    guard arguments.count == 6, let port = Int(arguments[1]), let rounds = Int(arguments[5]) else {
        fail("side-features <port> <user> <key file> <dir> <rounds>")
    }
    let keyText = try String(contentsOfFile: arguments[3], encoding: .utf8)
    let dir = arguments[4]
    let keychain = KeychainService.inMemory()
    let ref = keychain.generateRef()
    try await keychain.savePrivateKey(keyText, forRef: ref, requireBiometry: false)
    let connection = ServerConnection(name: "t", host: "127.0.0.1", port: port, username: arguments[2],
                                      authMethod: .key, keychainRef: ref)
    let service = SSHService(keychain: keychain, hostKeyValidator: .inMemory())
    let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("side-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: true)
    let image = Data((0..<(256 * 1024)).map { UInt8(truncatingIfNeeded: ($0 &* 2654435761) >> 13) })
    var times: [String: [Double]] = [:]
    func timed<T>(_ name: String, _ work: () async throws -> T) async throws -> T {
        let started = Date()
        let value = try await work()
        times[name, default: []].append(Date().timeIntervalSince(started) * 1000)
        return value
    }
    let probe = "sh -c 'for p in \"$@\"; do if [ -e \"$HOME/$p\" ]; then printf \"%s\\n\" \"$p\"; fi; done' moshpit-probe chart.png notes missing.txt"
    for round in 0..<rounds {
        let ssh = try await timed("sign in") {
            try await service.connect(connection, onUnknownHost: { _, _, _ in true })
        }
        let probed = try await timed("probe") { try await ssh.executeCommand(probe) }
        guard String(decoding: probed, as: UTF8.self) == "chart.png\nnotes\n" else {
            fail("probe: \(String(decoding: probed, as: UTF8.self).debugDescription)")
        }
        try await timed("preview") {
            let entry = try await ssh.remoteStat(dir + "/chart.png")
            let local = scratch.appendingPathComponent("chart-\(round).png")
            try await ssh.downloadRemoteFile(entry.path, to: local, progress: nil)
            guard (try FileManager.default.attributesOfItem(atPath: local.path)[.size] as? UInt64) == entry.size else {
                fail("preview: short copy")
            }
        }
        try await timed("stream 1 MiB") {
            let file = try await ssh.openRemoteFile(dir + "/clip.mp4")
            let got = try await WindowedRead.pull(
                size: 1 << 20, chunk: 64 * 1024, window: 16,
                read: { offset, length in try await file.read(from: offset, length: length) },
                write: { _, _ in })
            await file.close()
            guard got == 1 << 20 else { fail("stream: \(got)") }
        }
        try await timed("folder") {
            guard try await !ssh.listRemoteDirectory(dir).isEmpty else { fail("folder: empty") }
        }
        let path = try await timed("upload 256 KiB") {
            try await ssh.uploadToUploadsDirectory(image, named: "IMG-side-\(round).jpg", progress: nil)
        }
        guard (try? Data(contentsOf: URL(fileURLWithPath: path))) == image else { fail("upload: \(path)") }
        await ssh.close()
    }
    try? FileManager.default.removeItem(at: scratch)
    for name in ["sign in", "probe", "preview", "stream 1 MiB", "folder", "upload 256 KiB"] {
        let samples = (times[name] ?? []).sorted()
        guard !samples.isEmpty else { continue }
        print(String(format: "dialled %15@: p50 %6.2f ms  max %6.2f ms", name as NSString,
                     samples[samples.count / 2], samples.last ?? 0))
    }
    let totals = (0..<rounds).map { round in times.values.reduce(0) { $0 + ($1.count > round ? $1[round] : 0) } }.sorted()
    print(String(format: "dialled %15@: p50 %6.2f ms", "all of it" as NSString, totals[totals.count / 2]))
    exit(0)

default:
    fail("unknown command \(command)")
}
