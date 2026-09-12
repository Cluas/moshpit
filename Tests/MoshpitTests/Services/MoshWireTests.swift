import Foundation
import Testing
@testable import Moshpit

@Suite("Mosh wire codecs")
struct MoshWireTests {

    private func hexString(_ data: Data) -> String {
        data.map { String(format: "%02X", $0) }.joined()
    }

    // MARK: - TransportInstruction

    @Test("TransportInstruction encodes to known proto2 bytes")
    func transportInstructionGoldenBytes() {
        let instruction = TransportInstruction(
            protocolVersion: 2,
            oldNum: 1,
            newNum: 2,
            ackNum: 3,
            throwawayNum: 0,
            diff: Data("hi".utf8)
        )
        // 08 02 | 10 01 | 18 02 | 20 03 | 28 00 | 32 02 68 69
        #expect(hexString(instruction.encoded()) == "0802100118022003280032026869")
    }

    @Test("TransportInstruction round-trips, including multi-byte varints")
    func transportInstructionRoundTrip() throws {
        let instruction = TransportInstruction(
            protocolVersion: .max,
            oldNum: .max,
            newNum: 1 << 40,
            ackNum: 12_345,
            throwawayNum: 7,
            diff: Data([0x00, 0xFF, 0x01]),
            chaff: Data([0x09, 0x09])
        )
        let decoded = try TransportInstruction(parsing: instruction.encoded())
        #expect(decoded == instruction)
    }

    @Test("TransportInstruction decode skips unknown fields")
    func transportInstructionSkipsUnknownFields() throws {
        var writer = ProtoWriter()
        writer.appendField(3, varint: 9)               // new_num
        writer.appendField(12, varint: 999)            // unknown varint
        writer.appendField(13, bytes: [1, 2, 3])       // unknown length-delimited
        writer.appendField(4, varint: 8)               // ack_num
        let decoded = try TransportInstruction(parsing: writer.data)
        #expect(decoded.newNum == 9)
        #expect(decoded.ackNum == 8)
    }

    @Test("decode throws on truncated input")
    func transportInstructionTruncated() {
        // Key for field 6 (bytes) declaring 200 bytes of payload, but none present.
        let bad = Data([0x32, 0xC8, 0x01])
        #expect(throws: MoshWireError.truncatedField) {
            _ = try TransportInstruction(parsing: bad)
        }
    }

    // MARK: - UserMessage

    @Test("keystroke encodes as Instruction ext field 2 with keys at field 4")
    func keystrokeGoldenBytes() {
        let message = UserMessage(events: [.keystroke(keys: Data("a".utf8))])
        // UserMessage: f1 LEN { Instruction: f2 LEN { Keystroke: f4 LEN "a" } }
        #expect(hexString(message.encoded()) == "0A051203220161")
    }

    @Test("resize encodes as Instruction ext field 3 with width=5, height=6")
    func resizeGoldenBytes() {
        let message = UserMessage(events: [.resize(width: 80, height: 24)])
        // f1 LEN { f3 LEN { f5 varint 80, f6 varint 24 } }
        #expect(hexString(message.encoded()) == "0A061A0428503018")
    }

    @Test("UserMessage round-trips mixed keystrokes and resizes")
    func userMessageRoundTrip() throws {
        let message = UserMessage(events: [
            .keystroke(keys: Data("ls -la\n".utf8)),
            .resize(width: 132, height: 43),
            .keystroke(keys: Data([0x03])), // ^C
        ])
        let decoded = try UserMessage(parsing: message.encoded())
        #expect(decoded == message)
    }

    @Test("UserMessage decode ignores instructions with unknown extensions")
    func userMessageIgnoresUnknownExtensions() throws {
        var unknownInstruction = ProtoWriter()
        unknownInstruction.appendField(99, bytes: [0x01]) // future extension
        var writer = ProtoWriter()
        writer.appendField(1, bytes: unknownInstruction.bytes)
        var keystroke = ProtoWriter()
        keystroke.appendField(4, bytes: [UInt8]("x".utf8))   // Keystroke.keys = field 4
        var instruction = ProtoWriter()
        instruction.appendField(2, bytes: keystroke.bytes)   // Keystroke = ext field 2
        writer.appendField(1, bytes: instruction.bytes)

        let decoded = try UserMessage(parsing: writer.data)
        #expect(decoded.events == [.keystroke(keys: Data("x".utf8))])
    }

    // MARK: - TransportFragment

    @Test("fragment header is 8-byte BE id plus 2-byte BE num with final bit")
    func fragmentGoldenHeader() {
        let fragment = TransportFragment(
            id: 0x0102030405060708,
            fragmentNum: 1,
            isFinal: true,
            contents: Data()
        )
        #expect(hexString(fragment.encoded()) == "01020304050607088001")
    }

    @Test("fragment encode/parse round-trips")
    func fragmentRoundTrip() throws {
        let fragment = TransportFragment(
            id: .max,
            fragmentNum: 0x7FFF,
            isFinal: false,
            contents: Data([1, 2, 3])
        )
        let decoded = try TransportFragment(parsing: fragment.encoded())
        #expect(decoded == fragment)
    }

    @Test("parsing rejects datagrams shorter than the 10-byte header")
    func fragmentTooShort() {
        #expect(throws: MoshWireError.truncatedFragment) {
            _ = try TransportFragment(parsing: Data(repeating: 0, count: 9))
        }
    }

    @Test("fragmenting splits payloads at mtu minus header and marks the last final")
    func fragmenting() {
        let payload = Data((0..<2500).map { UInt8($0 % 251) })
        let fragments = TransportFragment.fragment(payload: payload, id: 42, mtu: 1280)
        #expect(fragments.count == 2)
        #expect(fragments[0].contents.count == 1270)
        #expect(fragments[0].isFinal == false)
        #expect(fragments[1].isFinal == true)
        #expect(fragments.map(\.fragmentNum) == [0, 1])
    }

    @Test("empty payload still produces one final fragment")
    func fragmentingEmptyPayload() {
        let fragments = TransportFragment.fragment(payload: Data(), id: 7, mtu: 1280)
        #expect(fragments.count == 1)
        #expect(fragments[0].isFinal == true)
        #expect(fragments[0].contents.isEmpty)
    }

    @Test("assembler rebuilds the payload from in-order fragments")
    func assemblerRebuildsPayload() throws {
        let payload = Data((0..<5000).map { UInt8($0 % 256) })
        let fragments = TransportFragment.fragment(payload: payload, id: 9, mtu: 1280)
        var assembler = FragmentAssembler()
        var result: Data?
        for fragment in fragments {
            #expect(result == nil)
            result = assembler.receive(try TransportFragment(parsing: fragment.encoded()))
        }
        #expect(result == payload)
    }

    @Test("assembler drops fragments from stale instruction ids")
    func assemblerDropsStaleIds() {
        var assembler = FragmentAssembler()
        let newer = assembler.receive(TransportFragment(id: 5, fragmentNum: 0, isFinal: true, contents: Data([1])))
        #expect(newer == Data([1]))
        let stale = assembler.receive(TransportFragment(id: 4, fragmentNum: 0, isFinal: true, contents: Data([2])))
        #expect(stale == nil)
    }

    @Test("assembler resets partial state when a newer instruction arrives")
    func assemblerResetsOnNewerId() {
        var assembler = FragmentAssembler()
        #expect(assembler.receive(TransportFragment(id: 1, fragmentNum: 0, isFinal: false, contents: Data([1]))) == nil)
        // Newer instruction supersedes the unfinished one.
        let result = assembler.receive(TransportFragment(id: 2, fragmentNum: 0, isFinal: true, contents: Data([9])))
        #expect(result == Data([9]))
    }
}
