// The Swift side of the ssh cross-check (harness/build-swift.sh): the app's
// fingerprint function, Citadel's key detection, SSHKeyFactory's import and
// ExecUploadCommands, over the swift-nio-ssh the app pins. One command per
// run; harness/equivalence.py asks the Rust example the same.
//
//   ssh-swift fingerprint <openssh public key line>
//   ssh-swift live <host> <port>        what a NIOSSH client is shown
//   ssh-swift keytype <private key file>
//   ssh-swift import <private key file> [<public key file>]
//   ssh-swift upload <home> <filename>
//   ssh-swift socks <proxy port> <target host> <target port>
//   ssh-swift keys <cases file>        the key reader, one case per line
//   ssh-swift write-keys <list> <dir>  what the Swift code stores: each
//                                      listed key imported, keys generated

import Foundation
import NIOConcurrencyHelpers
import NIOCore
import NIOPosix
import NIOSSH

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(2)
}

/// `<algorithm> <fingerprint>` for a key, the fingerprint as the app stores it.
func describe(_ key: NIOSSHPublicKey) -> String {
    let algorithm = String(openSSHPublicKey: key).split(separator: " ").first.map(String.init) ?? "?"
    return "\(algorithm) \(HostKeyValidator.fingerprint(of: key))"
}

struct Refused: Error {}
struct Closed: Error {}

/// Takes note of the host key and refuses it: the handshake goes no further.
final class NoteHostKey: NIOSSHClientServerAuthenticationDelegate, @unchecked Sendable {
    let seen: EventLoopPromise<String>
    init(seen: EventLoopPromise<String>) { self.seen = seen }

    func validateHostKey(hostKey: NIOSSHPublicKey, validationCompletePromise: EventLoopPromise<Void>) {
        seen.succeed(describe(hostKey))
        validationCompletePromise.fail(Refused())
    }
}

final class NoCredentials: NIOSSHClientUserAuthenticationDelegate, @unchecked Sendable {
    func nextAuthenticationType(
        availableMethods: NIOSSHAvailableUserAuthenticationMethods,
        nextChallengePromise: EventLoopPromise<NIOSSHUserAuthenticationOffer?>
    ) {
        nextChallengePromise.succeed(nil)
    }
}

/// Fails the promise when the handshake dies before a host key was seen.
final class Watch: ChannelInboundHandler, @unchecked Sendable {
    typealias InboundIn = Any
    let seen: EventLoopPromise<String>
    init(seen: EventLoopPromise<String>) { self.seen = seen }

    func errorCaught(context: ChannelHandlerContext, error: Error) {
        seen.fail(error)
        context.close(promise: nil)
    }

    func channelInactive(context: ChannelHandlerContext) {
        seen.fail(Closed())
    }
}

/// What the host shows a NIOSSH client configured as Citadel configures it
/// for the app (the default algorithms: no RSA host keys).
func live(host: String, port: Int) -> String {
    let group = MultiThreadedEventLoopGroup(numberOfThreads: 1)
    defer { try? group.syncShutdownGracefully() }
    let seen = group.next().makePromise(of: String.self)
    group.next().scheduleTask(in: .seconds(10)) { seen.fail(Closed()) }
    let bootstrap = ClientBootstrap(group: group).channelInitializer { channel in
        channel.eventLoop.makeCompletedFuture {
            let configuration = SSHClientConfiguration(
                userAuthDelegate: NoCredentials(),
                serverAuthDelegate: NoteHostKey(seen: seen))
            try channel.pipeline.syncOperations.addHandler(NIOSSHHandler(
                role: .client(configuration),
                allocator: channel.allocator,
                inboundChildChannelInitializer: nil))
            try channel.pipeline.syncOperations.addHandler(Watch(seen: seen))
        }
    }
    do {
        let channel = try bootstrap.connect(host: host, port: port).wait()
        defer { channel.close(promise: nil) }
        return try seen.futureResult.wait()
    } catch {
        return "kex-failed"
    }
}

func keytype(_ path: String) -> String {
    guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { fail("can't read \(path)") }
    do {
        return "type " + (try SSHKeyDetection.detectPrivateKeyType(from: text)).rawValue
    } catch let error as SSHKeyDetectionError {
        switch error {
        case .invalidKeyFormat: return "error invalidKeyFormat"
        case .unsupportedKeyType(let type): return "error unsupportedKeyType \(type ?? "-")"
        case .invalidPrivateKeyFormat: return "error invalidPrivateKeyFormat"
        case .malformedKey: return "error malformedKey"
        case .encryptedPrivateKey: return "error encryptedPrivateKey"
        case .passphraseRequired: return "error passphraseRequired"
        case .incorrectPassphrase: return "error incorrectPassphrase"
        }
    } catch {
        return "error \(error)"
    }
}

func importKey(_ path: String, publicPath: String?) -> String {
    guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { fail("can't read \(path)") }
    let line = publicPath.flatMap { try? String(contentsOfFile: $0, encoding: .utf8) }
    do {
        let key = try SSHKeyFactory.importKey(privatePEM: text, publicKeyLine: line, comment: "")
        return "\(key.algorithm.rawValue)\t\(key.fingerprint)\t\(key.publicKeyLine)"
    } catch {
        return "error"
    }
}

/// What the app's SOCKSProxyDialer does with a proxy on the loopback:
/// `connected`, `error`, or `hung` when it neither connects nor fails within
/// `patience` seconds.
func socks(proxyPort: Int, targetHost: String, targetPort: Int, patience: Double) -> String {
    let done = DispatchSemaphore(value: 0)
    let outcome = NIOLockedValueBox("hung")
    Task {
        do {
            let channel = try await SOCKSProxyDialer.connect(
                proxyHost: "127.0.0.1", proxyPort: proxyPort,
                targetHost: targetHost, targetPort: targetPort)
            outcome.withLockedValue { $0 = "connected" }
            try? await channel.close()
        } catch {
            outcome.withLockedValue { $0 = "error" }
        }
        done.signal()
    }
    _ = done.wait(timeout: .now() + patience)
    return outcome.withLockedValue { $0 }
}

// MARK: - The key reader, in batch (harness/keys_crosscheck.py)

func importError(_ error: Error) -> String {
    switch error as? SSHKeyImportError {
    case .unrecognized?: return "error Unrecognized"
    case .damaged?: return "error Damaged"
    case .passphraseRequired?: return "error NeedsPassphrase"
    case .wrongPassphrase?: return "error WrongPassphrase"
    case .securityKey?: return "error SecurityKey"
    case .unsupportedType(let what)?: return "error Unsupported \(what)"
    case .unsupportedEncryption(let what)?: return "error UnsupportedEncryption \(what)"
    case nil: return "error \(error)"
    }
}

/// The key a stored text reads back as: its fingerprint and own comment.
func readBack(_ text: String) -> String {
    guard let contents = try? SSHPrivateKeyFile.read(text, passphrase: nil) else { return "unreadable" }
    return "\(SSHKeyFactory.fingerprint(of: contents.material.publicKeyBlob)) [\(contents.comment)]"
}

/// One case: `id <TAB> op <TAB> file <TAB> passphrase|- <TAB> public line file|- <TAB> comment`.
func keyCase(_ fields: [String]) -> String {
    let (op, path, passphrase) = (fields[1], fields[2], fields[3] == "-" ? nil : fields[3])
    guard let data = FileManager.default.contents(atPath: path) else { fail("can't read \(path)") }
    let text = String(decoding: data, as: UTF8.self)
    switch op {
    case "import":
        let line = fields[4] == "-" ? nil : (try? String(contentsOfFile: fields[4], encoding: .utf8))
        do {
            let key = try SSHKeyFactory.importKey(privatePEM: text, passphrase: passphrase,
                                                  publicKeyLine: line, comment: fields[5])
            let stored = String(decoding: key.privateBlob, as: UTF8.self)
            return ["ok", key.algorithm.rawValue, key.fingerprint, key.publicKeyLine,
                    key.bits.map(String.init) ?? "-", readBack(stored)].joined(separator: "\t")
        } catch {
            return importError(error)
        }
    case "isencrypted":
        return SSHPrivateKeyFile.isEncrypted(text) ? "true" : "false"
    case "assess":
        do {
            let contents: SSHPrivateKeyFile.Contents
            do {
                contents = try SSHPrivateKeyFile.read(text, passphrase: passphrase)
            } catch SSHKeyImportError.passphraseRequired {
                return "needsPassphrase"
            }
            if !SSHPrivateKeyFile.isEncrypted(text), (try? SSHKeyDetection.detectPrivateKeyType(from: text)) != nil {
                return "usable"
            }
            let converted = contents.material.openSSHPrivateKey(comment: contents.comment)
            let again = try SSHPrivateKeyFile.read(converted, passphrase: nil)
            guard again.material.publicKeyBlob == contents.material.publicKeyBlob,
                  (try? SSHKeyDetection.detectPrivateKeyType(from: converted)) != nil else {
                return "error Damaged"
            }
            return "converted \(readBack(converted))"
        } catch {
            return importError(error)
        }
    case "describe":
        guard let key = SSHKeyFactory.describeStored(data) else { return "none" }
        return [key.algorithm.rawValue, key.fingerprint, key.publicKeyLine,
                key.bits.map(String.init) ?? "-"].joined(separator: "\t")
    default:
        fail("no key op \(op)")
    }
}

/// What the app's Swift code stored in the keychain (before the move to
/// Rust): each key in `list` (`path <TAB> passphrase|-` lines) imported,
/// and Ed25519 and RSA-4096 keys generated, written to `dir` with a
/// manifest of what the record showed (tests/fixtures/keys/swift-written).
func writeKeys(list: String, dir: String) {
    var manifest = "# Written by harness/swift/main.swift write-keys over the app's Swift key code of 81c121a1: file, fingerprint, algorithm, public key line, bits.\n"
    func write(_ name: String, _ key: SSHKeyFactory.Generated) {
        FileManager.default.createFile(atPath: dir + "/" + name, contents: key.privateBlob)
        manifest += [name, key.fingerprint, key.algorithm.rawValue, key.publicKeyLine,
                     key.bits.map(String.init) ?? "-"].joined(separator: "\t") + "\n"
    }
    guard let lines = try? String(contentsOfFile: list, encoding: .utf8) else { fail("can't read \(list)") }
    for line in lines.split(separator: "\n") {
        let fields = line.split(separator: "\t").map(String.init)
        guard let text = try? String(contentsOfFile: fields[0], encoding: .utf8) else { fail("can't read \(fields[0])") }
        do {
            let key = try SSHKeyFactory.importKey(privatePEM: text, passphrase: fields[1] == "-" ? nil : fields[1],
                                                  publicKeyLine: nil, comment: "")
            write("imported-" + (fields[0] as NSString).lastPathComponent, key)
        } catch {
            fail("\(fields[0]): \(error)")
        }
    }
    for index in 1 ... 3 {
        do { write("generated-ed25519-\(index)", try SSHKeyFactory.generate(algorithm: .ed25519, comment: "made by swift")) }
        catch { fail("\(error)") }
    }
    do { write("generated-rsa4096", try SSHKeyFactory.generate(algorithm: .rsa4096, comment: "made by swift")) }
    catch { fail("\(error)") }
    FileManager.default.createFile(atPath: dir + "/manifest.tsv", contents: Data(manifest.utf8))
}

let args = CommandLine.arguments.dropFirst()
switch args.first {
case "fingerprint":
    guard args.count == 2 else { fail("fingerprint <line>") }
    let line = args[args.startIndex + 1]
    if let key = try? NIOSSHPublicKey(openSSHPublicKey: line) {
        print(describe(key))
    } else {
        print("error")
    }
case "live":
    guard args.count == 3, let port = Int(args[args.startIndex + 2]) else { fail("live <host> <port>") }
    print(live(host: args[args.startIndex + 1], port: port))
case "keytype":
    guard args.count == 2 else { fail("keytype <file>") }
    print(keytype(args[args.startIndex + 1]))
case "import":
    guard args.count == 2 || args.count == 3 else { fail("import <file> [<public file>]") }
    print(importKey(args[args.startIndex + 1], publicPath: args.count == 3 ? args[args.startIndex + 2] : nil))
case "upload":
    guard args.count == 3 else { fail("upload <home> <filename>") }
    let plan = ExecUploadCommands.plan(home: args[args.startIndex + 1], filename: args[args.startIndex + 2])
    print("begin\t\(plan.begin)")
    print("append\t\(plan.append("QUJD"))")
    print("finish\t\(plan.finish)")
    print("final\t\(plan.finalPath)")
case "socks":
    guard args.count == 4, let proxy = Int(args[args.startIndex + 1]),
          let port = Int(args[args.startIndex + 3]) else { fail("socks <proxy port> <host> <port>") }
    // Past the app's own 15 s budget for the proxy and its handshake (B130).
    print(socks(proxyPort: proxy, targetHost: args[args.startIndex + 2], targetPort: port, patience: 20))
case "keys":
    guard args.count == 2, let cases = try? String(contentsOfFile: args[args.startIndex + 1], encoding: .utf8)
    else { fail("keys <cases file>") }
    for line in cases.split(separator: "\n") {
        let fields = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
        print("\(fields[0])\t\(keyCase(fields))")
    }
case "write-keys":
    guard args.count == 3 else { fail("write-keys <list> <dir>") }
    writeKeys(list: args[args.startIndex + 1], dir: args[args.startIndex + 2])
default:
    fail("usage: ssh-swift fingerprint|live|keytype|import|upload|socks|keys|write-keys …")
}
