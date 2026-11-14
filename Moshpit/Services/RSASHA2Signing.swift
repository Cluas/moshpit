import Citadel
import Crypto
import _CryptoExtras
import Foundation
import NIOCore
import NIOSSH

// MARK: - Which signature an RSA key signs with

/// The signature an RSA key's sign-in carries — RFC 8332.
///
/// Citadel signs every RSA key with SHA-1 (`ssh-rsa`), and OpenSSH 8.8 and
/// later refuse that out of the box ("signature algorithm ssh-rsa not in
/// PubkeyAcceptedAlgorithms"): an RSA key, the app's own generated RSA-4096
/// ones included, could sign in to no current OpenSSH server. The key itself
/// is fine; only the hash in front of its signature has to change.
enum RSASignatureAlgorithm: String, CaseIterable, Sendable {
    case sha512 = "rsa-sha2-512"
    case sha256 = "rsa-sha2-256"
    /// SHA-1, what Citadel sends: only for a server too old to know the others.
    case sha1 = "ssh-rsa"

    /// The order to try them in, one sign-in each, when the server hasn't said
    /// which it takes: the strongest first, SHA-1 last.
    static let fallbackOrder: [RSASignatureAlgorithm] = [.sha512, .sha256, .sha1]

    /// The algorithms to try, in order.
    ///
    /// `serverSigAlgs` is the server's `server-sig-algs` (RFC 8308) when it
    /// sent one: what it lists is all that is worth trying, in our order.
    /// NIOSSH, under Citadel, neither asks for that extension nor reads one —
    /// an `SSH_MSG_EXT_INFO` would end the connection — so the app passes nil
    /// and falls back to trying each in turn, a sign-in per connection
    /// (Citadel gives a sign-in method one offer). `worked` is the one that
    /// last signed in to this host: tried first, so a reconnect costs one
    /// connection whatever the host takes.
    static func order(serverSigAlgs: [String]? = nil,
                      worked: RSASignatureAlgorithm? = nil) -> [RSASignatureAlgorithm] {
        var order = fallbackOrder
        if let serverSigAlgs {
            let listed = order.filter { serverSigAlgs.contains($0.rawValue) }
            // A list naming none of them (an odd server): try them all anyway
            // rather than fail without asking.
            if !listed.isEmpty { order = listed }
        }
        if let worked, let at = order.firstIndex(of: worked) {
            order.remove(at: at)
            order.insert(worked, at: 0)
        }
        return order
    }
}

/// One RSA key, ready to sign in with any of the three algorithms: a fresh
/// sign-in method per attempt, since Citadel's are used up by one connection.
struct RSAKeySignIn {
    let username: String
    /// The key's numbers, big-endian.
    let n: Data, e: Data, d: Data, p: Data, q: Data
    /// The same key as an unencrypted openssh-key-v1 file, for Citadel's own
    /// SHA-1 signer.
    let openSSHKey: String

    func method(_ algorithm: RSASignatureAlgorithm) throws -> SSHAuthenticationMethod {
        switch algorithm {
        case .sha512:
            return .custom(PublicKeyOfferDelegate(
                username: username,
                key: NIOSSHPrivateKey(custom: try RSASHA2PrivateKey<RSASHA512>(n: n, e: e, d: d, p: p, q: q))))
        case .sha256:
            return .custom(PublicKeyOfferDelegate(
                username: username,
                key: NIOSSHPrivateKey(custom: try RSASHA2PrivateKey<RSASHA256>(n: n, e: e, d: d, p: p, q: q))))
        case .sha1:
            return .rsa(username: username, privateKey: try Insecure.RSA.PrivateKey(sshRsa: openSSHKey))
        }
    }
}

// MARK: - RSA keys that sign with SHA-2

/// The hash an `rsa-sha2-*` signature is made over.
protocol RSASHA2Hash {
    associatedtype Function: HashFunction
    /// The algorithm's name on the wire: "rsa-sha2-512" or "rsa-sha2-256".
    static var algorithmName: String { get }
}

enum RSASHA512: RSASHA2Hash {
    typealias Function = SHA512
    static var algorithmName: String { RSASignatureAlgorithm.sha512.rawValue }
}

enum RSASHA256: RSASHA2Hash {
    typealias Function = SHA256
    static var algorithmName: String { RSASignatureAlgorithm.sha256.rawValue }
}

/// An RSA private key that signs as `rsa-sha2-512` or `rsa-sha2-256`
/// (RFC 8332 §3): PKCS #1 v1.5 over SHA-512 or SHA-256, by swift-crypto's
/// BoringSSL — the same library Citadel signs SHA-1 with.
///
/// NIOSSH takes the name of a key it doesn't know from the key's type, and
/// uses that one name twice in a sign-in: as the algorithm the request names
/// and as the key type inside the public key blob. RFC 8332 wants the
/// algorithm there (`rsa-sha2-512`) but the blob to still say `ssh-rsa`, so
/// the public key puts that right as NIOSSH writes it
/// (``RSASHA2PublicKey/write(to:)``).
final class RSASHA2PrivateKey<Hash: RSASHA2Hash>: NIOSSHPrivateKeyProtocol {
    static var keyPrefix: String { Hash.algorithmName }

    private let key: _RSA.Signing.PrivateKey
    private let rsaPublicKey: RSASHA2PublicKey<Hash>

    var publicKey: NIOSSHPublicKeyProtocol { rsaPublicKey }

    /// From the key's numbers, big-endian: modulus, public and private
    /// exponents, primes.
    init(n: Data, e: Data, d: Data, p: Data, q: Data) throws {
        key = try _RSA.Signing.PrivateKey(n: n, e: e, d: d, p: p, q: q)
        rsaPublicKey = RSASHA2PublicKey(modulus: n, exponent: e)
    }

    func signature<D: DataProtocol>(for data: D) throws -> NIOSSHSignatureProtocol {
        let signature = try key.signature(for: Hash.Function.hash(data: data), padding: .insecurePKCS1v1_5)
        return RSASHA2Signature<Hash>(rawRepresentation: signature.rawRepresentation)
    }
}

/// The public half of an ``RSASHA2PrivateKey``.
final class RSASHA2PublicKey<Hash: RSASHA2Hash>: NIOSSHPublicKeyProtocol {
    static var publicKeyPrefix: String { Hash.algorithmName }

    /// Big-endian magnitudes.
    let modulus: Data
    let exponent: Data

    init(modulus: Data, exponent: Data) {
        self.modulus = Data(modulus.drop(while: { $0 == 0 }))
        self.exponent = Data(exponent.drop(while: { $0 == 0 }))
    }

    /// `mpint e`, `mpint n`: an `ssh-rsa` key without its type.
    var rawRepresentation: Data {
        var buffer = ByteBuffer()
        _ = RSAWire.writeKey(exponent: exponent, modulus: modulus, to: &buffer)
        return Data(buffer.readableBytesView)
    }

    func isValidSignature<D: DataProtocol>(_ signature: NIOSSHSignatureProtocol, for data: D) -> Bool {
        guard let signature = signature as? RSASHA2Signature<Hash>,
              let key = try? _RSA.Signing.PublicKey(n: modulus, e: exponent)
        else { return false }
        return key.isValidSignature(_RSA.Signing.RSASignature(rawRepresentation: signature.rawRepresentation),
                                    for: Hash.Function.hash(data: data), padding: .insecurePKCS1v1_5)
    }

    /// Writes the key as `string "ssh-rsa"`, `mpint e`, `mpint n` — the
    /// `ssh-rsa` key blob, which RFC 8332 keeps for both SHA-2 algorithms.
    ///
    /// NIOSSH calls this right after writing the key's type, ``publicKeyPrefix``
    /// — here the signature algorithm, the name the request needs in front of
    /// the blob. When those bytes are what precedes the writer, they are taken
    /// back and `ssh-rsa` written in their place; the count returned keeps
    /// NIOSSH's own length arithmetic right (it adds this to what it wrote).
    func write(to buffer: inout ByteBuffer) -> Int {
        let name = Array(Self.publicKeyPrefix.utf8)
        let header = 4 + name.count
        var written = 0
        let start = buffer.writerIndex - header
        if start >= buffer.readerIndex,
           buffer.getInteger(at: start, as: UInt32.self) == UInt32(name.count),
           buffer.getBytes(at: start + 4, length: name.count) == name {
            buffer.moveWriterIndex(to: start)
            written -= header
            written += RSAWire.writeString(Array(RSASignatureAlgorithm.sha1.rawValue.utf8), to: &buffer)
        }
        written += RSAWire.writeKey(exponent: exponent, modulus: modulus, to: &buffer)
        return written
    }

    static func read(from buffer: inout ByteBuffer) throws -> RSASHA2PublicKey<Hash> {
        guard let exponent = RSAWire.readString(&buffer), let modulus = RSAWire.readString(&buffer) else {
            throw RSAWire.Malformed()
        }
        return RSASHA2PublicKey(modulus: modulus, exponent: exponent)
    }
}

/// An `rsa-sha2-*` signature: `string algorithm`, `string signature` — NIOSSH
/// writes the first, this the second.
struct RSASHA2Signature<Hash: RSASHA2Hash>: NIOSSHSignatureProtocol {
    static var signaturePrefix: String { Hash.algorithmName }

    let rawRepresentation: Data

    func write(to buffer: inout ByteBuffer) -> Int {
        RSAWire.writeString(Array(rawRepresentation), to: &buffer)
    }

    static func read(from buffer: inout ByteBuffer) throws -> RSASHA2Signature<Hash> {
        guard let blob = RSAWire.readString(&buffer) else { throw RSAWire.Malformed() }
        return RSASHA2Signature(rawRepresentation: blob)
    }
}

/// RFC 4251 §5 framing for the types above.
enum RSAWire {
    struct Malformed: Error {}

    @discardableResult
    static func writeString(_ bytes: [UInt8], to buffer: inout ByteBuffer) -> Int {
        buffer.writeInteger(UInt32(bytes.count)) + buffer.writeBytes(bytes)
    }

    /// A positive integer, big-endian, as an mpint: no leading zero bytes,
    /// one put back when the top bit would read as a sign.
    @discardableResult
    static func writeMPInt(_ magnitude: Data, to buffer: inout ByteBuffer) -> Int {
        var bytes = Array(magnitude.drop(while: { $0 == 0 }))
        if let first = bytes.first, first & 0x80 != 0 { bytes.insert(0, at: 0) }
        return writeString(bytes, to: &buffer)
    }

    static func writeKey(exponent: Data, modulus: Data, to buffer: inout ByteBuffer) -> Int {
        writeMPInt(exponent, to: &buffer) + writeMPInt(modulus, to: &buffer)
    }

    static func readString(_ buffer: inout ByteBuffer) -> Data? {
        guard let length = buffer.getInteger(at: buffer.readerIndex, as: UInt32.self),
              buffer.readableBytes >= 4 + Int(length) else { return nil }
        buffer.moveReaderIndex(forwardBy: 4)
        return buffer.readData(length: Int(length))
    }
}

// MARK: - Offering one key

/// Offers one private key for publickey sign-in, once — through Citadel's
/// `.custom` sign-in method, which asks it exactly once.
///
/// A server that takes no public keys gets an error, not "no offer": NIOSSH
/// does nothing with a nil offer, and the handshake then sat out Citadel's
/// 10-second sign-in clock to fail as "couldn't reach the server".
///
/// `canSign`, for a key whose signature can be refused (a Secure Enclave
/// key while the device is locked), is asked right before the offer: NIOSSH
/// signs the offer itself, inside its handshake, and a signature refused
/// there reached the app as a failed sign-in, not as the lock (B249). Its
/// failure fails the offer as ``SecureEnclaveRefused``.
final class PublicKeyOfferDelegate: NIOSSHClientUserAuthenticationDelegate {
    private let username: String
    private let key: NIOSSHPrivateKey
    private let canSign: (() throws -> Void)?
    private var offered = false

    init(username: String, key: NIOSSHPrivateKey, canSign: (() throws -> Void)? = nil) {
        self.username = username
        self.key = key
        self.canSign = canSign
    }

    func nextAuthenticationType(
        availableMethods: NIOSSHAvailableUserAuthenticationMethods,
        nextChallengePromise: EventLoopPromise<NIOSSHUserAuthenticationOffer?>
    ) {
        guard !offered else {
            nextChallengePromise.fail(SSHClientError.allAuthenticationOptionsFailed)
            return
        }
        guard availableMethods.contains(.publicKey) else {
            nextChallengePromise.fail(SSHClientError.unsupportedPrivateKeyAuthentication)
            return
        }
        offered = true
        if let canSign {
            do {
                try canSign()
            } catch {
                nextChallengePromise.fail(SecureEnclaveRefused(error: error))
                return
            }
        }
        nextChallengePromise.succeed(NIOSSHUserAuthenticationOffer(
            username: username, serviceName: "", offer: .privateKey(.init(privateKey: key))))
    }
}

/// The Secure Enclave wouldn't sign a sign-in's offer (``PublicKeyOfferDelegate``).
struct SecureEnclaveRefused: Error {
    let error: Error
}
