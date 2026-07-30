import Foundation
import Testing
@testable import Ringdown

@Suite("OCB3 (RFC 7253, AEAD_AES_128_OCB_TAGLEN128)")
struct OCB3Tests {

    // MARK: - Helpers

    private static func hex(_ string: String) -> Data {
        var out = Data()
        var index = string.startIndex
        while index < string.endIndex {
            let next = string.index(index, offsetBy: 2)
            out.append(UInt8(string[index..<next], radix: 16)!)
            index = next
        }
        return out
    }

    private static func hexString(_ data: Data) -> String {
        data.map { String(format: "%02X", $0) }.joined()
    }

    /// RFC 7253 Appendix A: (nonce, associated data, plaintext, ciphertext||tag),
    /// K = 000102030405060708090A0B0C0D0E0F, 128-bit tag.
    static let rfcVectors: [(n: String, a: String, p: String, c: String)] = [
        ("BBAA99887766554433221100", "", "",
         "785407BFFFC8AD9EDCC5520AC9111EE6"),
        ("BBAA99887766554433221101", "0001020304050607", "0001020304050607",
         "6820B3657B6F615A5725BDA0D3B4EB3A257C9AF1F8F03009"),
        ("BBAA99887766554433221102", "0001020304050607", "",
         "81017F8203F081277152FADE694A0A00"),
        ("BBAA99887766554433221103", "", "0001020304050607",
         "45DD69F8F5AAE72414054CD1F35D82760B2CD00D2F99BFA9"),
        ("BBAA99887766554433221104", "000102030405060708090A0B0C0D0E0F",
         "000102030405060708090A0B0C0D0E0F",
         "571D535B60B277188BE5147170A9A22C3AD7A4FF3835B8C5701C1CCEC8FC3358"),
        ("BBAA99887766554433221105", "000102030405060708090A0B0C0D0E0F", "",
         "8CF761B6902EF764462AD86498CA6B97"),
        ("BBAA99887766554433221106", "", "000102030405060708090A0B0C0D0E0F",
         "5CE88EC2E0692706A915C00AEB8B2396F40E1C743F52436BDF06D8FA1ECA343D"),
        ("BBAA99887766554433221107",
         "000102030405060708090A0B0C0D0E0F1011121314151617",
         "000102030405060708090A0B0C0D0E0F1011121314151617",
         "1CA2207308C87C010756104D8840CE1952F09673A448A122C92C62241051F57356D7F3C90BB0E07F"),
        ("BBAA99887766554433221108",
         "000102030405060708090A0B0C0D0E0F1011121314151617", "",
         "6DC225A071FC1B9F7C69F93B0F1E10DE"),
        ("BBAA99887766554433221109", "",
         "000102030405060708090A0B0C0D0E0F1011121314151617",
         "221BD0DE7FA6FE993ECCD769460A0AF2D6CDED0C395B1C3CE725F32494B9F914D85C0B1EB38357FF"),
        ("BBAA9988776655443322110A",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
         "BD6F6C496201C69296C11EFD138A467ABD3C707924B964DEAFFC40319AF5A48540FBBA186C5553C68AD9F592A79A4240"),
        ("BBAA9988776655443322110B",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F", "",
         "FE80690BEE8A485D11F32965BC9D2A32"),
        ("BBAA9988776655443322110C", "",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
         "2942BFC773BDA23CABC6ACFD9BFD5835BD300F0973792EF46040C53F1432BCDFB5E1DDE3BC18A5F840B52E653444D5DF"),
        ("BBAA9988776655443322110D",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
         "D5CA91748410C1751FF8A2F618255B68A0A12E093FF454606E59F9C1D0DDC54B65E8628E568BAD7AED07BA06A4A69483A7035490C5769E60"),
        ("BBAA9988776655443322110E",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627", "",
         "C5CD9D1850C141E358649994EE701B68"),
        ("BBAA9988776655443322110F", "",
         "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
         "4412923493C57D5DE0D700F753CCE0D1D2D95060122E9F15A5DDBFC5787E50B5CC55EE507BCB084E479AD363AC366B95A98CA5F3000B1479"),
    ]

    private static let rfcKey = hex("000102030405060708090A0B0C0D0E0F")

    // MARK: - RFC 7253 Appendix A vectors

    @Test("seal matches RFC 7253 Appendix A ciphertext", arguments: 0..<rfcVectors.count)
    func sealMatchesRFCVector(index: Int) throws {
        let v = Self.rfcVectors[index]
        let ocb = try OCB3(key: Self.rfcKey)
        let sealed = try ocb.seal(
            plaintext: Self.hex(v.p),
            nonce: Self.hex(v.n),
            additionalData: Self.hex(v.a)
        )
        #expect(Self.hexString(sealed) == v.c)
    }

    @Test("open recovers RFC 7253 Appendix A plaintext", arguments: 0..<rfcVectors.count)
    func openMatchesRFCVector(index: Int) throws {
        let v = Self.rfcVectors[index]
        let ocb = try OCB3(key: Self.rfcKey)
        let opened = try ocb.open(
            ciphertext: Self.hex(v.c),
            nonce: Self.hex(v.n),
            additionalData: Self.hex(v.a)
        )
        #expect(Self.hexString(opened) == v.p)
    }

    @Test("open rejects a tampered tag", arguments: 0..<rfcVectors.count)
    func openRejectsTamperedCiphertext(index: Int) throws {
        let v = Self.rfcVectors[index]
        let ocb = try OCB3(key: Self.rfcKey)
        var bad = Self.hex(v.c)
        bad[bad.count - 1] ^= 0x01
        #expect(throws: OCB3.OCB3Error.authenticationFailed) {
            _ = try ocb.open(
                ciphertext: bad,
                nonce: Self.hex(v.n),
                additionalData: Self.hex(v.a)
            )
        }
    }

    @Test("RFC 7253 'wider variety of inputs' iteration test")
    func rfcIterationTest() throws {
        // K = zeros(KEYLEN-8) || num2str(TAGLEN, 8); KEYLEN 128, TAGLEN 128.
        let key = Data(repeating: 0, count: 15) + Data([128])
        let ocb = try OCB3(key: key)

        func nonce(_ n: UInt64) -> Data {
            var data = Data(repeating: 0, count: 12)
            var value = n
            for i in stride(from: 11, through: 4, by: -1) {
                data[i] = UInt8(value & 0xFF)
                value >>= 8
            }
            return data
        }

        var accumulated = Data()
        for i in 0..<128 {
            let s = Data(repeating: 0, count: i)
            accumulated += try ocb.seal(plaintext: s, nonce: nonce(UInt64(3 * i + 1)), additionalData: s)
            accumulated += try ocb.seal(plaintext: s, nonce: nonce(UInt64(3 * i + 2)))
            accumulated += try ocb.seal(plaintext: Data(), nonce: nonce(UInt64(3 * i + 3)), additionalData: s)
        }
        #expect(accumulated.count == 22_400)

        let output = try ocb.seal(plaintext: Data(), nonce: nonce(385), additionalData: accumulated)
        #expect(Self.hexString(output) == "67E944D23256C5E0B6C61FA22FDF1EA2")
    }

    // MARK: - Mosh usage shape (no associated data)

    @Test("seal/open round-trip with empty associated data, the mosh configuration")
    func moshShapeRoundTrip() throws {
        let ocb = try OCB3(key: Data(repeating: 0xA5, count: 16))
        let nonce = Data([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])
        let payload = Data((0..<1400).map { UInt8($0 % 256) })
        let sealed = try ocb.seal(plaintext: payload, nonce: nonce)
        #expect(sealed.count == payload.count + 16)
        #expect(try ocb.open(ciphertext: sealed, nonce: nonce) == payload)
    }

    @Test("open with the wrong nonce fails authentication")
    func wrongNonceFails() throws {
        let ocb = try OCB3(key: Data(repeating: 0x42, count: 16))
        let sealed = try ocb.seal(
            plaintext: Data("hello".utf8),
            nonce: Data(repeating: 0, count: 12)
        )
        var otherNonce = Data(repeating: 0, count: 12)
        otherNonce[11] = 1
        #expect(throws: OCB3.OCB3Error.authenticationFailed) {
            _ = try ocb.open(ciphertext: sealed, nonce: otherNonce)
        }
    }

    // MARK: - Input validation

    @Test("rejects keys that are not 16 bytes")
    func rejectsBadKeyLength() {
        #expect(throws: OCB3.OCB3Error.invalidKeyLength) {
            _ = try OCB3(key: Data(repeating: 0, count: 32))
        }
    }

    @Test("rejects nonces that are not 12 bytes")
    func rejectsBadNonceLength() throws {
        let ocb = try OCB3(key: Data(repeating: 0, count: 16))
        #expect(throws: OCB3.OCB3Error.invalidNonceLength) {
            _ = try ocb.seal(plaintext: Data(), nonce: Data(repeating: 0, count: 8))
        }
    }

    @Test("rejects ciphertext shorter than the tag")
    func rejectsShortCiphertext() throws {
        let ocb = try OCB3(key: Data(repeating: 0, count: 16))
        #expect(throws: OCB3.OCB3Error.ciphertextTooShort) {
            _ = try ocb.open(ciphertext: Data(repeating: 0, count: 15), nonce: Data(repeating: 0, count: 12))
        }
    }
}
