import CryptoKit
import Foundation
import Testing
@testable import Moshpit

/// The format promise, from the other side: what the key reader writes now
/// (`moshpit_ssh::keyfile`, through the app), 1.0.8 reads.
///
/// 1.0.8 (8c10490a) signs in with a stored key through Citadel — its key
/// detection, then `Insecure.RSA.PrivateKey(sshRsa:)` or
/// `Curve25519.Signing.PrivateKey(sshEd25519:)` — and its own P-256 reader,
/// `OpenSSHECDSAKey`, copied below verbatim (renamed only). Citadel left the
/// test bundle with the Swift transport (E4-SSH): its detection is copied
/// step for step (``StoredKey/olderBuildKeyType(_:)``), and an Ed25519 or
/// RSA key's container is read here as the unencrypted openssh-key-v1 file
/// both of Citadel's readers take — the Ed25519 key's seed must make its
/// public key — and by the key reader every connect path signs with now.
/// Every key in `Fixtures/keys-written-by-rust` (written by the Rust tests'
/// `write_keys_for_older_builds`, and checked by them to read back) must come
/// out as the key its fingerprint names. P-384 and P-521, which 1.0.8 never
/// signed with, must be detected as what they are — 1.0.8 then says it
/// can't use them — and not mistaken for anything else.
@Suite("Keys written now, read by 1.0.8")
struct StoredKeyCompatibilityTests {
    struct Written: Sendable, CustomTestStringConvertible {
        let file: String
        let fingerprint: String
        let algorithm: String
        var testDescription: String { file }
    }

    static let written: [Written] = {
        guard let manifest = try? TestFixtures.string("keys-written-by-rust/manifest.tsv") else { return [] }
        return manifest.split(separator: "\n").filter { !$0.hasPrefix("#") }.map { line in
            let fields = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
            return Written(file: fields[0], fingerprint: fields[1], algorithm: fields[2])
        }
    }()

    private static func sshString(_ data: Data) -> Data {
        var length = UInt32(data.count).bigEndian
        return Data(bytes: &length, count: 4) + data
    }

    @Test("Every key the Rust reader wrote is there")
    func allWritten() {
        #expect(Self.written.count == 21)
    }

    @Test("1.0.8's readers read each key the Rust reader writes as that key", arguments: written)
    func olderBuildReads(_ key: Written) throws {
        let text = try TestFixtures.string("keys-written-by-rust/\(key.file)")
        let detected = StoredKey.olderBuildKeyType(text)
        let blob: Data
        switch detected {
        case "ssh-ed25519":
            let container = try #require(OpenSSHContainer(text), "not an unencrypted openssh-key-v1 file")
            #expect(container.fields.count == 2)
            let secret = try #require(container.fields.last)
            #expect(secret.count == 64, "seed and public key")
            let pair = try Curve25519.Signing.PrivateKey(rawRepresentation: secret.prefix(32))
            #expect(pair.publicKey.rawRepresentation == secret.suffix(32))
            blob = Self.sshString(Data("ssh-ed25519".utf8)) + Self.sshString(pair.publicKey.rawRepresentation)
            #expect(container.publicBlob == blob)
        case "ssh-rsa":
            // n, e, d, iqmp, p, q: the six numbers Citadel's reader takes.
            let container = try #require(OpenSSHContainer(text), "not an unencrypted openssh-key-v1 file")
            #expect(container.fields.count == 6)
            blob = container.publicBlob
            let e = container.fields[1], n = container.fields[0]
            #expect(blob == Self.sshString(Data("ssh-rsa".utf8)) + Self.sshString(e) + Self.sshString(n))
            #expect(try SSHKeyFactory.publicKeyBlob(of: text) == blob)
            _ = try sshKeySigningParts(text: text)
        case "ecdsa-sha2-nistp256":
            let pair = try OpenSSHECDSAKey108.p256PrivateKey(fromPEM: text)
            blob = Self.sshString(Data("ecdsa-sha2-nistp256".utf8)) + Self.sshString(Data("nistp256".utf8))
                + Self.sshString(pair.publicKey.x963Representation)
            let message = Data("1.0.8".utf8)
            #expect(pair.publicKey.isValidSignature(try pair.signature(for: message), for: message))
        case "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521":
            // 1.0.8 can't sign with these and says so (`unsupportedKeyType`);
            // this build does, from the same text (B126).
            #expect(key.algorithm == "ECDSA")
            blob = try SSHKeyFactory.publicKeyBlob(of: text)
            #expect(try StoredKey.assess(text) == .usable)
        default:
            Issue.record("\(key.file): detected as \(String(describing: detected))")
            return
        }
        #expect(SSHKeyFactory.fingerprint(of: blob) == key.fingerprint, "not the key that was written")
        #expect(SSHKeyAlgorithm(rawValue: key.algorithm) != nil)
    }

    /// The record side of the same promise (B32): a key imported now is
    /// recorded in a form 1.0.8 decodes, P-384 and P-521 among them.
    @Test("A key imported now is recorded the way 1.0.8 decodes records")
    func recordsStayReadable() throws {
        for key in Self.written {
            let text = try TestFixtures.string("keys-written-by-rust/\(key.file)")
            let imported = try SSHKeyFactory.importKey(privatePEM: text, publicKeyLine: nil, comment: "")
            #expect(imported.algorithm.olderBuildsName.rawValue != "ECDSA", "\(key.file)")
            #expect(imported.fingerprint == key.fingerprint, "\(key.file)")
        }
    }
}

// MARK: - The container Citadel's readers take

/// An unencrypted openssh-key-v1 file holding one key, read as Citadel's
/// `sshEd25519:` and `sshRsa:` readers read it: cipher and KDF "none", no
/// KDF options, one key, the public blob, then the private section — two
/// equal check numbers, the key type, its fields (`fields`: the type's own
/// strings, after the type), a comment and the padding 1, 2, 3….
private struct OpenSSHContainer {
    let type: String
    let publicBlob: Data
    let fields: [Data]

    init?(_ text: String) {
        var body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let begin = "-----BEGIN OPENSSH PRIVATE KEY-----", end = "-----END OPENSSH PRIVATE KEY-----"
        guard body.hasPrefix(begin), body.hasSuffix(end) else { return nil }
        body.removeLast(end.count)
        body.removeFirst(begin.count)
        guard let raw = Data(base64Encoded: body.replacingOccurrences(of: "\n", with: "")) else { return nil }
        var outer = Reader([UInt8](raw))
        guard outer.bytes(15) == Array("openssh-key-v1\0".utf8),
              outer.string() == Array("none".utf8), outer.string() == Array("none".utf8),
              outer.string() == [], outer.uint32() == 1,
              let publicBlob = outer.string(), let section = outer.string(), outer.done else { return nil }
        var inner = Reader(section)
        guard let check = inner.uint32(), inner.uint32() == check,
              let typeBytes = inner.string(), let type = String(bytes: typeBytes, encoding: .utf8) else { return nil }
        let count = ["ssh-ed25519": 2, "ssh-rsa": 6][type]
        guard let count else { return nil }
        var fields: [Data] = []
        for _ in 0..<count {
            guard let field = inner.string() else { return nil }
            fields.append(Data(field))
        }
        guard inner.string() != nil else { return nil }   // the comment
        var pad: UInt8 = 1
        while let byte = inner.bytes(1)?.first {
            guard byte == pad else { return nil }
            pad += 1
        }
        self.type = type
        self.publicBlob = Data(publicBlob)
        self.fields = fields
    }

    private struct Reader {
        let data: [UInt8]
        var at = 0
        init(_ data: [UInt8]) { self.data = data }
        var done: Bool { at == data.count }

        mutating func bytes(_ count: Int) -> [UInt8]? {
            guard count >= 0, at + count <= data.count else { return nil }
            defer { at += count }
            return Array(data[at..<at + count])
        }

        mutating func uint32() -> Int? {
            bytes(4)?.reduce(0) { $0 << 8 | Int($1) }
        }

        mutating func string() -> [UInt8]? {
            guard let length = uint32() else { return nil }
            return bytes(length)
        }
    }
}

// MARK: - 1.0.8's P-256 reader, verbatim (8c10490a, Moshpit/Services/SSHService.swift)

private enum OpenSSHECDSAKey108 {
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
