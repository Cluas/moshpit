import Citadel
import Crypto
import Foundation
import NIOCore
import NIOPosix
import NIOSSH
import Security
import Testing
@testable import Moshpit

/// B129: an RSA key signs in with rsa-sha2-512 / rsa-sha2-256 (RFC 8332),
/// falling back to SHA-1 `ssh-rsa` only for a server that takes nothing
/// newer. Citadel signed every RSA key with SHA-1, which OpenSSH 8.8 and
/// later refuse ("signature algorithm ssh-rsa not in PubkeyAcceptedAlgorithms"),
/// so no RSA key — the app's own generated RSA-4096 ones included — could
/// sign in to a current OpenSSH server. (Against a real sshd:
/// scripts/verify/ssh-live/run.sh rsa.)
@Suite("RSA sign-in with SHA-2 signatures")
struct RSASHA2SigningTests {

    private static let fixture = SSHKeyFixtures.all.first { $0.name == "RSA 2048 · OpenSSH" }!

    private struct Numbers {
        let n: Data, e: Data, d: Data, p: Data, q: Data
        let publicBlob: Data
    }

    private static func numbers() throws -> Numbers {
        guard case .rsa(let n, let e, let d, let p, let q) = try SSHKeyFactory.signingParts(of: fixture.privateKey) else {
            throw SSHKeyImportError.damaged
        }
        return Numbers(n: n, e: e, d: d, p: p, q: q, publicBlob: try SSHKeyFactory.publicKeyBlob(of: fixture.privateKey))
    }

    /// The key blob of the fixture's `.pub` line, as `ssh-keygen` wrote it.
    private static func keygenBlob() throws -> Data {
        let fields = fixture.publicKey.split(separator: " ")
        return try #require(Data(base64Encoded: String(fields[1])))
    }

    // MARK: The wire

    @Test("The public key NIOSSH writes for an rsa-sha2 key is the ssh-rsa blob, as RFC 8332 keeps it")
    func publicKeyBlobSaysSSHRSA() throws {
        let numbers = try Self.numbers()
        #expect(numbers.publicBlob == (try Self.keygenBlob()), "the reader got the key wrong")
        for key in [
            NIOSSHPrivateKey(custom: try RSASHA2PrivateKey<RSASHA512>(
                n: numbers.n, e: numbers.e, d: numbers.d, p: numbers.p, q: numbers.q)),
            NIOSSHPrivateKey(custom: try RSASHA2PrivateKey<RSASHA256>(
                n: numbers.n, e: numbers.e, d: numbers.d, p: numbers.p, q: numbers.q)),
        ] {
            // Alone in a buffer…
            var buffer = ByteBuffer()
            let written = key.publicKey.write(to: &buffer)
            #expect(Data(buffer.readableBytesView) == numbers.publicBlob)
            #expect(written == numbers.publicBlob.count)

            // …and after other bytes, as NIOSSH writes it inside a request:
            // a length it fills in from the count returned, then the key.
            var request = ByteBuffer()
            request.writeBytes(Array("publickey".utf8))
            let lengthAt = request.writerIndex
            request.moveWriterIndex(forwardBy: 4)
            let count = key.publicKey.write(to: &request)
            request.setInteger(UInt32(count), at: lengthAt)
            #expect(request.getString(at: 0, length: 9) == "publickey")
            #expect(request.getInteger(at: lengthAt, as: UInt32.self) == UInt32(numbers.publicBlob.count))
            #expect(request.getData(at: lengthAt + 4, length: request.writerIndex - lengthAt - 4) == numbers.publicBlob)
        }
    }

    @Test("Each signature names its algorithm, and the request names it in front of the key")
    func algorithmNames() {
        #expect(RSASHA2PrivateKey<RSASHA512>.keyPrefix == "rsa-sha2-512")
        #expect(RSASHA2PublicKey<RSASHA512>.publicKeyPrefix == "rsa-sha2-512")
        #expect(RSASHA2Signature<RSASHA512>.signaturePrefix == "rsa-sha2-512")
        #expect(RSASHA2PrivateKey<RSASHA256>.keyPrefix == "rsa-sha2-256")
        #expect(RSASHA2PublicKey<RSASHA256>.publicKeyPrefix == "rsa-sha2-256")
        #expect(RSASHA2Signature<RSASHA256>.signaturePrefix == "rsa-sha2-256")

        var buffer = ByteBuffer()
        let written = RSASHA2Signature<RSASHA512>(rawRepresentation: Data([1, 2, 3])).write(to: &buffer)
        #expect(Array(buffer.readableBytesView) == [0, 0, 0, 3, 1, 2, 3])
        #expect(written == 7)
    }

    // MARK: The signatures

    /// Checked by Apple's own RSA, not the BoringSSL that made them.
    @Test("rsa-sha2-512 and rsa-sha2-256 signatures are PKCS #1 v1.5 over SHA-512 and SHA-256")
    func signaturesVerifyWithSecurity() throws {
        let numbers = try Self.numbers()
        let publicKey = try Self.securityPublicKey(n: numbers.n, e: numbers.e)
        let message = Data("session id, user, service, publickey, rsa-sha2-512, key".utf8)

        let sha512 = try RSASHA2PrivateKey<RSASHA512>(n: numbers.n, e: numbers.e, d: numbers.d, p: numbers.p, q: numbers.q)
            .signature(for: message).rawRepresentation
        let sha256 = try RSASHA2PrivateKey<RSASHA256>(n: numbers.n, e: numbers.e, d: numbers.d, p: numbers.p, q: numbers.q)
            .signature(for: message).rawRepresentation

        #expect(sha512.count == 256 && sha256.count == 256, "one modulus long")
        #expect(Self.verify(publicKey, .rsaSignatureMessagePKCS1v15SHA512, message, sha512))
        #expect(Self.verify(publicKey, .rsaSignatureMessagePKCS1v15SHA256, message, sha256))
        #expect(!Self.verify(publicKey, .rsaSignatureMessagePKCS1v15SHA256, message, sha512))
        #expect(!Self.verify(publicKey, .rsaSignatureMessagePKCS1v15SHA512, message, sha256))
        #expect(!Self.verify(publicKey, .rsaSignatureMessagePKCS1v15SHA512, message + Data([0]), sha512))

        // And the key's own check agrees.
        let key = RSASHA2PublicKey<RSASHA512>(modulus: numbers.n, exponent: numbers.e)
        #expect(key.isValidSignature(RSASHA2Signature<RSASHA512>(rawRepresentation: sha512), for: message))
        #expect(!key.isValidSignature(RSASHA2Signature<RSASHA512>(rawRepresentation: sha256), for: message))
    }

    @Test("The app's own generated RSA-4096 key signs with SHA-2 too")
    func generatedKeySigns() throws {
        let generated = try SSHKeyFactory.generate(algorithm: .rsa4096, comment: "t")
        guard case .rsa(let n, let e, let d, let p, let q) =
                try SSHKeyFactory.signingParts(of: String(decoding: generated.privateBlob, as: UTF8.self)) else {
            Issue.record("not RSA")
            return
        }
        let message = Data("hello".utf8)
        let signature = try RSASHA2PrivateKey<RSASHA512>(n: n, e: e, d: d, p: p, q: q)
            .signature(for: message).rawRepresentation
        #expect(signature.count == 512)
        #expect(Self.verify(try Self.securityPublicKey(n: n, e: e), .rsaSignatureMessagePKCS1v15SHA512,
                            message, signature))
    }

    // MARK: The order

    @Test("Without the server's list: SHA-512, SHA-256, then SHA-1; what worked last goes first")
    func order() {
        #expect(RSASignatureAlgorithm.order() == [.sha512, .sha256, .sha1])
        #expect(RSASignatureAlgorithm.order(worked: .sha256) == [.sha256, .sha512, .sha1])
        #expect(RSASignatureAlgorithm.order(worked: .sha1) == [.sha1, .sha512, .sha256])
        #expect(RSASignatureAlgorithm.order(serverSigAlgs: ["ssh-ed25519", "rsa-sha2-256", "rsa-sha2-512"])
                == [.sha512, .sha256])
        #expect(RSASignatureAlgorithm.order(serverSigAlgs: ["rsa-sha2-256"]) == [.sha256])
        #expect(RSASignatureAlgorithm.order(serverSigAlgs: ["ssh-ed25519"]) == [.sha512, .sha256, .sha1])
    }

    // MARK: The connect path

    @Test("SSHService tries rsa-sha2-512, rsa-sha2-256, then ssh-rsa, one connection each, and stops there")
    func serviceTriesEachSignature() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(Self.fixture.privateKey, forRef: ref, requireBiometry: false)
        let provider = RefusingProvider(error: SSHClientError.allAuthenticationOptionsFailed)
        let service = SSHService(keychain: keychain, hostKeyValidator: .inMemory(), clientProvider: provider)
        let connection = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                          authMethod: .key, keychainRef: ref)

        await #expect(throws: SSHError.self) { _ = try await service.connect(connection) }
        let offered = provider.offered
        #expect(offered.count == 3, "\(offered)")
        guard offered.count == 3 else { return }
        #expect(offered[0].contains("RSASHA512"), "\(offered[0])")
        #expect(offered[1].contains("RSASHA256"), "\(offered[1])")
        #expect(offered[2].contains("Insecure.RSA.PrivateKey"), "\(offered[2])")
        #expect(offered.allSatisfy { $0.hasPrefix("me ") }, "every attempt signs in as the connection's user")
    }

    @Test("A refused RSA sign-in is still reported as one")
    func refusedRSAIsAuthenticationFailed() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(Self.fixture.privateKey, forRef: ref, requireBiometry: false)
        let service = SSHService(keychain: keychain, hostKeyValidator: .inMemory(),
                                 clientProvider: RefusingProvider(error: SSHClientError.allAuthenticationOptionsFailed))
        let connection = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                          authMethod: .key, keychainRef: ref)
        do {
            _ = try await service.connect(connection)
            Issue.record("connected")
        } catch let error as SSHError {
            guard case .authenticationFailed = error else {
                Issue.record("\(error.diagnostic)")
                return
            }
        }
    }

    @Test("Anything but a refused sign-in is the answer: no second connection")
    func otherFailuresStopAtOne() async throws {
        let keychain = KeychainService.inMemory()
        let ref = keychain.generateRef()
        try await keychain.savePrivateKey(Self.fixture.privateKey, forRef: ref, requireBiometry: false)
        let provider = RefusingProvider(error: ChannelError.connectTimeout(.seconds(15)))
        let service = SSHService(keychain: keychain, hostKeyValidator: .inMemory(), clientProvider: provider)
        let connection = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                          authMethod: .key, keychainRef: ref)
        await #expect(throws: SSHError.self) { _ = try await service.connect(connection) }
        #expect(provider.offered.count == 1)
    }

    @Test("An Ed25519 key, or a password, is one connection")
    func otherCredentialsAreOneAttempt() async throws {
        let keychain = KeychainService.inMemory()
        let keyRef = keychain.generateRef()
        let ed25519 = SSHKeyFixtures.all.first { $0.name == "Ed25519 · OpenSSH" }!
        try await keychain.savePrivateKey(ed25519.privateKey, forRef: keyRef, requireBiometry: false)
        let provider = RefusingProvider(error: SSHClientError.allAuthenticationOptionsFailed)
        let service = SSHService(keychain: keychain, hostKeyValidator: .inMemory(), clientProvider: provider)
        let keyed = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                     authMethod: .key, keychainRef: keyRef)
        await #expect(throws: SSHError.self) { _ = try await service.connect(keyed) }
        #expect(provider.offered.count == 1)

        let password = ServerConnection(name: "t", host: "192.0.2.1", port: 22, username: "me",
                                        authMethod: .password, keychainRef: nil)
        await #expect(throws: SSHError.self) { _ = try await service.connect(password, overrideSecret: "pw") }
        #expect(provider.offered.count == 2)
    }

    @Test("A server that takes no keys is told apart at once, not after a 10 s stall")
    func offerWithoutPublicKeyMethodFails() async throws {
        let numbers = try Self.numbers()
        let delegate = PublicKeyOfferDelegate(
            username: "me",
            key: NIOSSHPrivateKey(custom: try RSASHA2PrivateKey<RSASHA512>(
                n: numbers.n, e: numbers.e, d: numbers.d, p: numbers.p, q: numbers.q)))
        let loop = MultiThreadedEventLoopGroup.singleton.any()

        let refused: EventLoopPromise<NIOSSHUserAuthenticationOffer?> = loop.makePromise()
        delegate.nextAuthenticationType(availableMethods: .password, nextChallengePromise: refused)
        await #expect(throws: SSHClientError.self) { _ = try await refused.futureResult.get() }

        let offered: EventLoopPromise<NIOSSHUserAuthenticationOffer?> = loop.makePromise()
        delegate.nextAuthenticationType(availableMethods: .all, nextChallengePromise: offered)
        #expect(try await offered.futureResult.get()?.username == "me")

        let again: EventLoopPromise<NIOSSHUserAuthenticationOffer?> = loop.makePromise()
        delegate.nextAuthenticationType(availableMethods: .all, nextChallengePromise: again)
        await #expect(throws: SSHClientError.self) { _ = try await again.futureResult.get() }
    }

    /// B249: NIOSSH signs a Secure Enclave key's offer inside the
    /// handshake, and the chip refuses while the device is locked; that
    /// read as a failed sign-in, never retried. The chip is asked right
    /// before the offer now: its refusal fails the offer as the chip's
    /// (`SecureEnclaveRefused`), which reads as the lock while protected
    /// data is unavailable — as does a refusal inside NIOSSH's own signing.
    @Test("A Secure Enclave signature refused while the device is locked reads as the lock")
    func secureEnclaveRefusalIsTheLock() async throws {
        let locked = NSError(domain: NSOSStatusErrorDomain, code: Int(errSecInteractionNotAllowed))
        let delegate = PublicKeyOfferDelegate(
            username: "me", key: NIOSSHPrivateKey(p256Key: P256.Signing.PrivateKey()),
            canSign: { throw locked })
        let loop = MultiThreadedEventLoopGroup.singleton.any()
        let offer: EventLoopPromise<NIOSSHUserAuthenticationOffer?> = loop.makePromise()
        delegate.nextAuthenticationType(availableMethods: .all, nextChallengePromise: offer)
        var refusal: Error?
        do {
            _ = try await offer.futureResult.get()
        } catch {
            refusal = error
        }
        let refused = try #require(refusal as? SecureEnclaveRefused, "offered a key the chip won't sign with: \(String(describing: refusal))")

        // What the old path made of it: not the lock, so never retried.
        if case .credentialLocked = SSHError.map(locked) { Issue.record("the old mapping already said locked") }
        func reads(_ error: Error, unlocked: Bool) -> SSHError? {
            SSHError.secureEnclaveSignIn(failedWith: error, protectedDataAvailable: unlocked)
        }
        guard case .credentialLocked? = reads(refused, unlocked: false) else {
            Issue.record("refused while locked: \(String(describing: reads(refused, unlocked: false)))")
            return
        }
        #expect(TerminalViewModel.retryable(SSHError.credentialLocked))
        // NIOSSH's own signature refused in the handshake reads the same.
        guard case .credentialLocked? = reads(CryptoKitError.underlyingCoreCryptoError(error: -25308), unlocked: false) else {
            Issue.record("a CryptoKit refusal while locked isn't the lock")
            return
        }
        // Unlocked, it is the key's own failure, never the lock.
        guard case .underlying? = reads(refused, unlocked: true) else {
            Issue.record("refused while unlocked: \(String(describing: reads(refused, unlocked: true)))")
            return
        }
        // The server's no is a sign-in's, locked or not.
        #expect(reads(SSHClientError.allAuthenticationOptionsFailed, unlocked: false) == nil)

        // A chip that signs: the key is offered, once.
        let signing = PublicKeyOfferDelegate(
            username: "me", key: NIOSSHPrivateKey(p256Key: P256.Signing.PrivateKey()), canSign: {})
        let offered: EventLoopPromise<NIOSSHUserAuthenticationOffer?> = loop.makePromise()
        signing.nextAuthenticationType(availableMethods: .all, nextChallengePromise: offered)
        #expect(try await offered.futureResult.get()?.username == "me")
    }

    // MARK: Helpers

    /// An RSA public key Security can verify with, from its numbers.
    private static func securityPublicKey(n: Data, e: Data) throws -> SecKey {
        func integer(_ magnitude: Data) -> Data {
            var bytes = Data(magnitude.drop(while: { $0 == 0 }))
            if let first = bytes.first, first & 0x80 != 0 { bytes.insert(0, at: bytes.startIndex) }
            return der(tag: 0x02, bytes)
        }
        let pkcs1 = der(tag: 0x30, integer(n) + integer(e))
        var error: Unmanaged<CFError>?
        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeRSA,
            kSecAttrKeyClass as String: kSecAttrKeyClassPublic,
        ]
        guard let key = SecKeyCreateWithData(pkcs1 as CFData, attributes as CFDictionary, &error) else {
            throw error!.takeRetainedValue() as Error
        }
        return key
    }

    private static func der(tag: UInt8, _ body: Data) -> Data {
        var out = Data([tag])
        if body.count < 0x80 {
            out.append(UInt8(body.count))
        } else {
            let length = withUnsafeBytes(of: UInt32(body.count).bigEndian, Array.init).drop(while: { $0 == 0 })
            out.append(0x80 | UInt8(length.count))
            out.append(contentsOf: length)
        }
        return out + body
    }

    private static func verify(_ key: SecKey, _ algorithm: SecKeyAlgorithm, _ message: Data, _ signature: Data) -> Bool {
        SecKeyVerifySignature(key, algorithm, message as CFData, signature as CFData, nil)
    }
}

/// Refuses every connection with `error`, after noting which key each sign-in
/// method offered (its reflection: the key type's name is in it).
private final class RefusingProvider: SSHClientProvider, @unchecked Sendable {
    private let error: Error
    private let lock = NSLock()
    private var seen: [String] = []

    init(error: Error) { self.error = error }

    var offered: [String] { lock.withLock { seen } }

    private func note(_ method: SSHAuthenticationMethod) async {
        let promise: EventLoopPromise<NIOSSHUserAuthenticationOffer?> =
            MultiThreadedEventLoopGroup.singleton.any().makePromise()
        method.nextAuthenticationType(availableMethods: .all, nextChallengePromise: promise)
        let description: String
        if let offer = try? await promise.futureResult.get() {
            if case .privateKey(let request) = offer.offer {
                description = "\(offer.username) " + String(reflecting: request.privateKey)
            } else {
                description = "\(offer.username) " + String(reflecting: offer.offer)
            }
        } else {
            description = "no offer"
        }
        lock.withLock { seen.append(description) }
    }

    func connect(host: String, port: Int, authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        await note(authenticationMethod)
        throw error
    }

    func connect(on channel: Channel, authenticationMethod: SSHAuthenticationMethod,
                 hostKeyValidator: SSHHostKeyValidator) async throws -> SignedInClient {
        await note(authenticationMethod)
        throw error
    }
}
