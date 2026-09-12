import CommonCrypto
import Foundation

/// AES-128-OCB3 authenticated encryption per RFC 7253, with a fixed
/// 128-bit tag and 96-bit nonce (the AEAD_AES_128_OCB_TAGLEN128 parameter
/// set). This is the cipher used by the mosh datagram layer, which always
/// passes an empty associated-data string.
///
/// Clean-room implementation written from the RFC 7253 specification.
/// Block encryption is delegated to CommonCrypto AES-ECB.
struct OCB3 {

    enum OCB3Error: Error, Equatable {
        case invalidKeyLength
        case invalidNonceLength
        case ciphertextTooShort
        case authenticationFailed
        case cipherFailure(Int32)
    }

    static let keyLength = 16
    static let nonceLength = 12
    static let tagLength = 16

    private let key: [UInt8]
    /// L_* = ENCIPHER(K, zeros(128))
    private let lStar: [UInt8]
    /// L_$ = double(L_*)
    private let lDollar: [UInt8]
    /// L_0 = double(L_$), L_i = double(L_{i-1}); indexed by ntz(blockIndex).
    private let l: [[UInt8]]

    // MARK: - Init

    /// - Parameter key: 16-byte AES-128 key.
    init(key: Data) throws {
        guard key.count == Self.keyLength else { throw OCB3Error.invalidKeyLength }
        let keyBytes = [UInt8](key)
        self.key = keyBytes

        // Key-dependent L table (RFC 7253 section 4).
        let star = try Self.aes(encrypt: true, key: keyBytes, block: [UInt8](repeating: 0, count: 16))
        let dollar = Self.double(star)
        var table: [[UInt8]] = []
        table.reserveCapacity(64)
        var current = Self.double(dollar)
        for _ in 0..<64 {
            table.append(current)
            current = Self.double(current)
        }
        self.lStar = star
        self.lDollar = dollar
        self.l = table
    }

    // MARK: - Public API

    /// Encrypts and authenticates `plaintext`, returning `ciphertext || tag`.
    /// - Parameters:
    ///   - plaintext: message of any length (may be empty).
    ///   - nonce: 12 bytes; MUST be unique per (key, direction).
    ///   - additionalData: authenticated-but-unencrypted data (mosh uses none).
    func seal(plaintext: Data, nonce: Data, additionalData: Data = Data()) throws -> Data {
        guard nonce.count == Self.nonceLength else { throw OCB3Error.invalidNonceLength }
        let p = [UInt8](plaintext)
        var offset = try initialOffset(nonce: [UInt8](nonce))
        var checksum = [UInt8](repeating: 0, count: 16)
        var out = [UInt8]()
        out.reserveCapacity(p.count + Self.tagLength)

        let fullBlocks = p.count / 16
        for blockIndex in 0..<fullBlocks {
            let i = blockIndex + 1
            Self.xor(&offset, l[i.trailingZeroBitCount])
            var block = Array(p[blockIndex * 16 ..< blockIndex * 16 + 16])
            Self.xor(&checksum, block)
            Self.xor(&block, offset)
            var c = try Self.aes(encrypt: true, key: key, block: block)
            Self.xor(&c, offset)
            out.append(contentsOf: c)
        }

        let remainder = p.count % 16
        if remainder > 0 {
            Self.xor(&offset, lStar)
            let pad = try Self.aes(encrypt: true, key: key, block: offset)
            let tail = p.suffix(remainder)
            for (j, byte) in tail.enumerated() {
                out.append(byte ^ pad[j])
            }
            // Checksum_* = Checksum_m xor (P_* || 1 || zeros)
            var padded = [UInt8](repeating: 0, count: 16)
            padded.replaceSubrange(0..<remainder, with: tail)
            padded[remainder] = 0x80
            Self.xor(&checksum, padded)
        }

        out.append(contentsOf: try tag(checksum: checksum, offset: offset, additionalData: additionalData))
        return Data(out)
    }

    /// Decrypts and verifies `ciphertext || tag`. Throws `.authenticationFailed`
    /// if the tag does not verify; no plaintext is returned in that case.
    func open(ciphertext: Data, nonce: Data, additionalData: Data = Data()) throws -> Data {
        guard nonce.count == Self.nonceLength else { throw OCB3Error.invalidNonceLength }
        guard ciphertext.count >= Self.tagLength else { throw OCB3Error.ciphertextTooShort }
        let bytes = [UInt8](ciphertext)
        let c = Array(bytes[0 ..< bytes.count - Self.tagLength])
        let expectedTag = Array(bytes.suffix(Self.tagLength))

        var offset = try initialOffset(nonce: [UInt8](nonce))
        var checksum = [UInt8](repeating: 0, count: 16)
        var plaintext = [UInt8]()
        plaintext.reserveCapacity(c.count)

        let fullBlocks = c.count / 16
        for blockIndex in 0..<fullBlocks {
            let i = blockIndex + 1
            Self.xor(&offset, l[i.trailingZeroBitCount])
            var block = Array(c[blockIndex * 16 ..< blockIndex * 16 + 16])
            Self.xor(&block, offset)
            var p = try Self.aes(encrypt: false, key: key, block: block)
            Self.xor(&p, offset)
            Self.xor(&checksum, p)
            plaintext.append(contentsOf: p)
        }

        let remainder = c.count % 16
        if remainder > 0 {
            Self.xor(&offset, lStar)
            let pad = try Self.aes(encrypt: true, key: key, block: offset)
            var padded = [UInt8](repeating: 0, count: 16)
            for (j, byte) in c.suffix(remainder).enumerated() {
                let pByte = byte ^ pad[j]
                plaintext.append(pByte)
                padded[j] = pByte
            }
            padded[remainder] = 0x80
            Self.xor(&checksum, padded)
        }

        let computed = try tag(checksum: checksum, offset: offset, additionalData: additionalData)
        var diff: UInt8 = 0
        for i in 0..<Self.tagLength {
            diff |= computed[i] ^ expectedTag[i]
        }
        guard diff == 0 else { throw OCB3Error.authenticationFailed }
        return Data(plaintext)
    }

    // MARK: - Core algorithm pieces

    /// Tag = ENCIPHER(K, Checksum xor Offset xor L_$) xor HASH(K, A)
    private func tag(checksum: [UInt8], offset: [UInt8], additionalData: Data) throws -> [UInt8] {
        var final = checksum
        Self.xor(&final, offset)
        Self.xor(&final, lDollar)
        var t = try Self.aes(encrypt: true, key: key, block: final)
        Self.xor(&t, try hash(additionalData))
        return t
    }

    /// Offset_0 derivation from the 96-bit nonce (RFC 7253 section 4.2).
    private func initialOffset(nonce: [UInt8]) throws -> [UInt8] {
        // Nonce = num2str(TAGLEN mod 128, 7) || zeros(120 - bitlen(N)) || 1 || N
        // With TAGLEN = 128 and a 96-bit N this is: 0x00 0x00 0x00 0x01 || N.
        var nonceBlock: [UInt8] = [0x00, 0x00, 0x00, 0x01] + nonce
        let bottom = Int(nonceBlock[15] & 0x3F)
        nonceBlock[15] &= 0xC0
        let ktop = try Self.aes(encrypt: true, key: key, block: nonceBlock)
        // Stretch = Ktop || (Ktop[1..64] xor Ktop[9..72])
        var stretch = ktop
        for i in 0..<8 {
            stretch.append(ktop[i] ^ ktop[i + 1])
        }
        // Offset_0 = Stretch[1+bottom..128+bottom] (bit-indexed from 1)
        let byteShift = bottom / 8
        let bitShift = bottom % 8
        var offset = [UInt8](repeating: 0, count: 16)
        for i in 0..<16 {
            if bitShift == 0 {
                offset[i] = stretch[i + byteShift]
            } else {
                offset[i] = (stretch[i + byteShift] << bitShift)
                    | (stretch[i + byteShift + 1] >> (8 - bitShift))
            }
        }
        return offset
    }

    /// HASH(K, A) per RFC 7253 section 4.1.
    private func hash(_ additionalData: Data) throws -> [UInt8] {
        var sum = [UInt8](repeating: 0, count: 16)
        guard !additionalData.isEmpty else { return sum }
        let a = [UInt8](additionalData)
        var offset = [UInt8](repeating: 0, count: 16)

        let fullBlocks = a.count / 16
        for blockIndex in 0..<fullBlocks {
            let i = blockIndex + 1
            Self.xor(&offset, l[i.trailingZeroBitCount])
            var block = Array(a[blockIndex * 16 ..< blockIndex * 16 + 16])
            Self.xor(&block, offset)
            Self.xor(&sum, try Self.aes(encrypt: true, key: key, block: block))
        }

        let remainder = a.count % 16
        if remainder > 0 {
            Self.xor(&offset, lStar)
            var block = [UInt8](repeating: 0, count: 16)
            block.replaceSubrange(0..<remainder, with: a.suffix(remainder))
            block[remainder] = 0x80
            Self.xor(&block, offset)
            Self.xor(&sum, try Self.aes(encrypt: true, key: key, block: block))
        }
        return sum
    }

    // MARK: - Primitives

    /// Doubling in GF(2^128): left shift by one bit, conditionally xor 0x87
    /// into the low byte (RFC 7253 section 2).
    private static func double(_ block: [UInt8]) -> [UInt8] {
        var out = [UInt8](repeating: 0, count: 16)
        for i in 0..<15 {
            out[i] = (block[i] << 1) | (block[i + 1] >> 7)
        }
        out[15] = block[15] << 1
        if block[0] & 0x80 != 0 {
            out[15] ^= 0x87
        }
        return out
    }

    private static func xor(_ a: inout [UInt8], _ b: [UInt8]) {
        for i in 0..<16 {
            a[i] ^= b[i]
        }
    }

    /// Single-block raw AES (ECB mode, no padding) via CommonCrypto.
    private static func aes(encrypt: Bool, key: [UInt8], block: [UInt8]) throws -> [UInt8] {
        var out = [UInt8](repeating: 0, count: 16)
        var moved = 0
        let status = CCCrypt(
            CCOperation(encrypt ? kCCEncrypt : kCCDecrypt),
            CCAlgorithm(kCCAlgorithmAES),
            CCOptions(kCCOptionECBMode),
            key, key.count,
            nil,
            block, block.count,
            &out, out.count,
            &moved
        )
        guard status == kCCSuccess, moved == 16 else {
            throw OCB3Error.cipherFailure(Int32(status))
        }
        return out
    }
}
