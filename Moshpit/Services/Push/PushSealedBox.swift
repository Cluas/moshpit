import CommonCrypto
import CryptoKit
import Foundation

/// The phone's half of the push envelope — the only place the plaintext of a
/// remote agent notification exists.
///
/// A push arrives carrying `aps` (which the relay wrote, and can read) plus an
/// `mp` envelope (which it cannot). The notification service extension opens the
/// envelope here and rewrites the notification's title and body with the real
/// ones before iOS shows it. The relay holds no key and never did: the pairing
/// secret was generated on this device and typed into the user's own server.
///
/// Format v1 is specified once, in `push-relay/sealbox/sealbox.go`, and
/// implemented three times — there, in `scripts/moshpit-push.sh`, and here. All
/// three are pinned to one frozen vector (`scripts/push-vector.sh`); if you
/// change any of them, that vector is how you find out.
///
///     KeHex = hex(HMAC-SHA256(key: ascii(secretHex), msg: "moshpit-push-enc-v1"))
///     KmHex = hex(HMAC-SHA256(key: ascii(secretHex), msg: "moshpit-push-mac-v1"))
///     ct    = base64(AES-256-CBC(key: KeHex, iv: ivHex, pkcs7(plaintext)))
///     mac   = base64(HMAC-SHA256(key: ascii(KmHex), msg: "v1|" + ivHex + "|" + ct))
///
/// CBC rather than the AES-GCM used everywhere else in this app, and hex-string
/// keys rather than raw bytes, are not preferences: the sending end is a POSIX
/// `sh` script with only openssl to hand, `openssl enc` refuses AEAD ciphers
/// outright, and the `-macopt hexkey:` form that would allow raw keys is absent
/// from the LibreSSL macOS ships. Encrypt-then-MAC with the MAC verified before
/// any padding is inspected is what makes that choice safe.
enum PushSealedBox {

    /// One sealed status, as it rides inside the APNs payload's `mp` object.
    struct Envelope: Codable, Equatable {
        var v: Int
        var iv: String
        var ct: String
        var mac: String
    }

    /// What an agent's hook actually said. `conn` is this device's own
    /// connection id, handed to the host at pairing and echoed back — so a
    /// notification arrives already knowing which saved connection it belongs
    /// to, and the lock-screen Allow/Deny path needs no host-name lookup.
    struct Status: Codable, Equatable {
        var conn: String
        var host: String
        var sess: String?
        var pane: String
        var agent: String?
        var state: String
        var title: String?
        var ts: Int
        /// Seconds the episode this status closes ran — for a `done`, the
        /// turn's length. Decides whether the finish is worth a sound. Optional
        /// both ways: older senders omit it, and decoding must not care.
        var dur: Int? = nil
    }

    enum Failure: Error, Equatable {
        case badSecret
        case unsupportedVersion(Int)
        case malformed
        /// The MAC did not verify. In practice this means one of: the wrong
        /// pairing secret (a host paired to a different install), or a tampered
        /// payload. Never distinguish the two to the user — both mean "do not
        /// show this".
        case authenticationFailed
        case decryptFailed(Int32)
    }

    static let version = 1
    private static let encInfo = "moshpit-push-enc-v1"
    private static let macInfo = "moshpit-push-mac-v1"
    private static let macTag = "v1"

    // MARK: - Open

    /// Authenticate and decrypt an envelope, then decode the status inside.
    static func open(_ envelope: Envelope, secretHex: String) throws -> Status {
        let plaintext = try openRaw(envelope, secretHex: secretHex)
        do {
            return try JSONDecoder().decode(Status.self, from: plaintext)
        } catch {
            throw Failure.malformed
        }
    }

    static func openRaw(_ envelope: Envelope, secretHex: String) throws -> Data {
        guard envelope.v == version else { throw Failure.unsupportedVersion(envelope.v) }
        let keys = try deriveKeys(secretHex: secretHex)
        guard let iv = Data(moshpitHex: envelope.iv), iv.count == kCCBlockSizeAES128,
              let ct = Data(base64Encoded: envelope.ct), !ct.isEmpty,
              ct.count % kCCBlockSizeAES128 == 0,
              let mac = Data(base64Encoded: envelope.mac)
        else { throw Failure.malformed }

        // MAC first, ALWAYS. Reaching the unpadding step with unauthenticated
        // ciphertext is what a CBC padding oracle needs, and encrypt-then-MAC
        // exists to deny it that reach. `isValidAuthenticationCode` is the
        // constant-time comparison, and handles a wrong-length tag itself.
        guard HMAC<SHA256>.isValidAuthenticationCode(
                mac,
                authenticating: Data(taggedMessage(ivHex: envelope.iv.lowercased(),
                                                   ctB64: envelope.ct).utf8),
                using: SymmetricKey(data: Data(keys.macHex.utf8)))
        else { throw Failure.authenticationFailed }

        guard let encKey = Data(moshpitHex: keys.encHex), encKey.count == kCCKeySizeAES256 else {
            throw Failure.badSecret
        }
        return try crypt(operation: CCOperation(kCCDecrypt), input: ct, key: encKey, iv: iv)
    }

    // MARK: - Seal (tests and the pairing self-check)

    /// Seal a payload. Production sealing happens on the dev host in shell; this
    /// exists so the format can be tested from both ends in one process.
    static func seal(_ plaintext: Data, secretHex: String, ivHex: String? = nil) throws -> Envelope {
        let keys = try deriveKeys(secretHex: secretHex)
        let iv: Data
        if let ivHex {
            guard let d = Data(moshpitHex: ivHex), d.count == kCCBlockSizeAES128 else {
                throw Failure.malformed
            }
            iv = d
        } else {
            var bytes = [UInt8](repeating: 0, count: kCCBlockSizeAES128)
            guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
                throw Failure.malformed
            }
            iv = Data(bytes)
        }
        guard let encKey = Data(moshpitHex: keys.encHex) else { throw Failure.badSecret }
        let ct = try crypt(operation: CCOperation(kCCEncrypt), input: plaintext, key: encKey, iv: iv)
        let ctB64 = ct.base64EncodedString()
        let ivHexOut = iv.moshpitHexString
        return Envelope(v: version, iv: ivHexOut, ct: ctB64,
                        mac: tag(macKeyHex: keys.macHex, ivHex: ivHexOut, ctB64: ctB64).base64EncodedString())
    }

    // MARK: - Internals

    struct Keys: Equatable {
        let encHex: String
        let macHex: String
    }

    /// Split a pairing secret into its two subkeys.
    ///
    /// The HMAC key is the ASCII of the hex STRING, not the 32 bytes it encodes
    /// — see the type comment. The secret is lowercased first because the shell
    /// does the same: otherwise a secret typed in uppercase derives different
    /// keys on the two ends and every message fails its MAC.
    static func deriveKeys(secretHex: String) throws -> Keys {
        let normalised = secretHex.lowercased()
        guard normalised.count == 64, Data(moshpitHex: normalised) != nil else {
            throw Failure.badSecret
        }
        let key = SymmetricKey(data: Data(normalised.utf8))
        func sub(_ info: String) -> String {
            Data(HMAC<SHA256>.authenticationCode(for: Data(info.utf8), using: key)).moshpitHexString
        }
        return Keys(encHex: sub(encInfo), macHex: sub(macInfo))
    }

    private static func taggedMessage(ivHex: String, ctB64: String) -> String {
        "\(macTag)|\(ivHex)|\(ctB64)"
    }

    private static func tag(macKeyHex: String, ivHex: String, ctB64: String) -> Data {
        Data(HMAC<SHA256>.authenticationCode(
            for: Data(taggedMessage(ivHex: ivHex, ctB64: ctB64).utf8),
            using: SymmetricKey(data: Data(macKeyHex.utf8))))
    }

    private static func crypt(operation: CCOperation, input: Data, key: Data, iv: Data) throws -> Data {
        var out = Data(count: input.count + kCCBlockSizeAES128)
        var moved = 0
        let status: CCCryptorStatus = out.withUnsafeMutableBytes { outBuf in
            input.withUnsafeBytes { inBuf in
                key.withUnsafeBytes { keyBuf in
                    iv.withUnsafeBytes { ivBuf in
                        CCCrypt(operation,
                                CCAlgorithm(kCCAlgorithmAES),
                                CCOptions(kCCOptionPKCS7Padding),
                                keyBuf.baseAddress, key.count,
                                ivBuf.baseAddress,
                                inBuf.baseAddress, input.count,
                                outBuf.baseAddress, outBuf.count,
                                &moved)
                    }
                }
            }
        }
        guard status == kCCSuccess else { throw Failure.decryptFailed(status) }
        return out.prefix(moved)
    }
}

// MARK: - Hex

extension Data {
    /// Lowercase hex. Named to avoid colliding with any other `hexString` the
    /// app or its dependencies may define.
    var moshpitHexString: String {
        map { String(format: "%02x", $0) }.joined()
    }

    /// Strict hex decode — nil on odd length or any non-hex character, so a
    /// malformed field can never be silently truncated into a valid-looking key.
    init?(moshpitHex hex: String) {
        guard hex.count % 2 == 0 else { return nil }
        var out = Data(capacity: hex.count / 2)
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
            out.append(byte)
            index = next
        }
        self = out
    }
}
