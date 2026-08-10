import Foundation

/// mosh's datagram crypto + Message framing (crypto.cc).
///
/// Wire packet = `nonce[8]` ‖ OCB(`ciphertext ‖ tag`). The 8 wire bytes are a
/// big-endian 64-bit value whose top bit is the direction and whose low 63
/// bits are a monotone sequence counter. The OCB nonce is those 8 bytes
/// right-aligned in a 12-byte block (4 leading zeros).
///
/// The OCB plaintext is a `Message`: `timestamp[2] ‖ timestamp_reply[2] ‖
/// payload`, all big-endian. `payload` is the transport fragment bytes.
struct MoshCrypto {
    enum Direction: UInt64 {
        case toServer = 0
        case toClient = 1
    }

    enum CryptoError: Error { case packetTooShort, badDirection }

    private let ocb: OCB3

    /// - Parameter key: the 16-byte AES-128 session key from `MOSH CONNECT`.
    init(key: Data) throws {
        self.ocb = try OCB3(key: key)
    }

    struct Message {
        var timestamp: UInt16
        var timestampReply: UInt16
        var payload: Data
        /// Decoded sequence number (low 63 bits of the nonce).
        var seq: UInt64 = 0
        var direction: Direction = .toClient
    }

    /// Seal a Message for sending. `seq` must be unique & monotone per session.
    func seal(_ message: Message, seq: UInt64, direction: Direction) throws -> Data {
        let nonce64 = (direction.rawValue << 63) | (seq & 0x7FFF_FFFF_FFFF_FFFF)
        let wireNonce = Self.beBytes(nonce64)            // 8 bytes
        let ocbNonce = Data([0, 0, 0, 0]) + wireNonce    // 12 bytes

        var plaintext = Data(capacity: 4 + message.payload.count)
        plaintext.append(contentsOf: Self.beBytes16(message.timestamp))
        plaintext.append(contentsOf: Self.beBytes16(message.timestampReply))
        plaintext.append(message.payload)

        let sealed = try ocb.seal(plaintext: plaintext, nonce: ocbNonce)
        return wireNonce + sealed
    }

    /// Open a received datagram into a Message (with decoded seq/direction).
    func open(_ packet: Data) throws -> Message {
        guard packet.count >= 8 + OCB3.tagLength else { throw CryptoError.packetTooShort }
        let wireNonce = packet.prefix(8)
        let ciphertext = packet.suffix(from: packet.startIndex + 8)
        let ocbNonce = Data([0, 0, 0, 0]) + wireNonce

        let plaintext = try ocb.open(ciphertext: Data(ciphertext), nonce: Data(ocbNonce))
        guard plaintext.count >= 4 else { throw CryptoError.packetTooShort }

        let nonce64 = Self.beValue(Data(wireNonce))
        let ts = Self.beValue16(plaintext.prefix(2))
        let tsReply = Self.beValue16(plaintext.subdata(in: plaintext.startIndex + 2 ..< plaintext.startIndex + 4))
        let payload = plaintext.subdata(in: plaintext.startIndex + 4 ..< plaintext.endIndex)

        return Message(
            timestamp: ts,
            timestampReply: tsReply,
            payload: payload,
            seq: nonce64 & 0x7FFF_FFFF_FFFF_FFFF,
            direction: (nonce64 >> 63) == 1 ? .toClient : .toServer)
    }

    // MARK: - Byte helpers

    private static func beBytes(_ v: UInt64) -> Data {
        var be = v.bigEndian
        return withUnsafeBytes(of: &be) { Data($0) }
    }

    private static func beBytes16(_ v: UInt16) -> [UInt8] {
        [UInt8(v >> 8), UInt8(v & 0xFF)]
    }

    private static func beValue(_ data: Data) -> UInt64 {
        data.reduce(0) { ($0 << 8) | UInt64($1) }
    }

    private static func beValue16<S: Sequence>(_ data: S) -> UInt16 where S.Element == UInt8 {
        data.reduce(0) { ($0 << 8) | UInt16($1) }
    }
}
