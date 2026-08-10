import Foundation

/// Wire codecs for the mosh 1.4 protocol: a minimal hand-rolled protobuf
/// (proto2) encoder/decoder for the two mosh message schemas, plus the
/// non-protobuf fragment framing used by the transport layer.
///
/// Clean-room implementation from the public .proto schemas
/// (transportinstruction.proto, userinput.proto) and wire-format
/// documentation. No SwiftProtobuf dependency.

enum MoshWireError: Error, Equatable {
    case truncatedVarint
    case truncatedField
    case unsupportedWireType(UInt8)
    case truncatedFragment
}

// MARK: - Minimal protobuf primitives

/// Append-only protobuf writer (proto2 wire format).
struct ProtoWriter {
    private(set) var bytes: [UInt8] = []

    var data: Data { Data(bytes) }

    mutating func appendVarint(_ value: UInt64) {
        var v = value
        while v >= 0x80 {
            bytes.append(UInt8(truncatingIfNeeded: v) | 0x80)
            v >>= 7
        }
        bytes.append(UInt8(truncatingIfNeeded: v))
    }

    /// Writes a wire-type-0 (varint) field.
    mutating func appendField(_ number: Int, varint value: UInt64) {
        appendKey(number, wireType: 0)
        appendVarint(value)
    }

    /// Writes a wire-type-2 (length-delimited) field.
    mutating func appendField(_ number: Int, bytes value: [UInt8]) {
        appendKey(number, wireType: 2)
        appendVarint(UInt64(value.count))
        bytes.append(contentsOf: value)
    }

    mutating func appendField(_ number: Int, bytes value: Data) {
        appendField(number, bytes: [UInt8](value))
    }

    private mutating func appendKey(_ number: Int, wireType: UInt64) {
        appendVarint(UInt64(number) << 3 | wireType)
    }
}

/// Forward-only protobuf reader. Unknown fields can be skipped, so decoders
/// stay compatible with schema extensions.
struct ProtoReader {
    private let bytes: [UInt8]
    private var pos = 0

    init(_ data: Data) { self.bytes = [UInt8](data) }
    init(_ bytes: [UInt8]) { self.bytes = bytes }

    var isAtEnd: Bool { pos >= bytes.count }

    mutating func readVarint() throws -> UInt64 {
        var result: UInt64 = 0
        var shift: UInt64 = 0
        while pos < bytes.count {
            let byte = bytes[pos]
            pos += 1
            if shift < 64 {
                result |= UInt64(byte & 0x7F) << shift
            }
            if byte & 0x80 == 0 {
                return result
            }
            shift += 7
            if shift > 63 { throw MoshWireError.truncatedVarint }
        }
        throw MoshWireError.truncatedVarint
    }

    /// Reads a field key, returning the field number and wire type.
    mutating func readKey() throws -> (field: UInt64, wireType: UInt8) {
        let key = try readVarint()
        return (key >> 3, UInt8(key & 0x7))
    }

    mutating func readLengthDelimited() throws -> [UInt8] {
        let length = try readVarint()
        guard length <= UInt64(bytes.count - pos) else { throw MoshWireError.truncatedField }
        let slice = Array(bytes[pos ..< pos + Int(length)])
        pos += Int(length)
        return slice
    }

    /// Skips over a field of the given wire type (for unknown fields).
    mutating func skipField(wireType: UInt8) throws {
        switch wireType {
        case 0:
            _ = try readVarint()
        case 1:
            try advance(8)
        case 2:
            _ = try readLengthDelimited()
        case 5:
            try advance(4)
        default:
            throw MoshWireError.unsupportedWireType(wireType)
        }
    }

    private mutating func advance(_ count: Int) throws {
        guard bytes.count - pos >= count else { throw MoshWireError.truncatedField }
        pos += count
    }
}

// MARK: - TransportBuffers.Instruction (transportinstruction.proto)

/// The mosh transport-layer state-sync instruction.
///
/// ```proto
/// message Instruction {
///   optional uint32 protocol_version = 1;
///   optional uint64 old_num         = 2;
///   optional uint64 new_num         = 3;
///   optional uint64 ack_num         = 4;
///   optional uint64 throwaway_num   = 5;
///   optional bytes  diff            = 6;
///   optional bytes  chaff           = 7;
/// }
/// ```
struct TransportInstruction: Equatable {
    /// mosh 1.4 speaks transport protocol version 2.
    static let currentProtocolVersion: UInt32 = 2

    var protocolVersion: UInt32 = TransportInstruction.currentProtocolVersion
    var oldNum: UInt64 = 0
    var newNum: UInt64 = 0
    var ackNum: UInt64 = 0
    var throwawayNum: UInt64 = 0
    var diff = Data()
    var chaff = Data()

    init(
        protocolVersion: UInt32 = TransportInstruction.currentProtocolVersion,
        oldNum: UInt64 = 0,
        newNum: UInt64 = 0,
        ackNum: UInt64 = 0,
        throwawayNum: UInt64 = 0,
        diff: Data = Data(),
        chaff: Data = Data()
    ) {
        self.protocolVersion = protocolVersion
        self.oldNum = oldNum
        self.newNum = newNum
        self.ackNum = ackNum
        self.throwawayNum = throwawayNum
        self.diff = diff
        self.chaff = chaff
    }

    func encoded() -> Data {
        var writer = ProtoWriter()
        writer.appendField(1, varint: UInt64(protocolVersion))
        writer.appendField(2, varint: oldNum)
        writer.appendField(3, varint: newNum)
        writer.appendField(4, varint: ackNum)
        writer.appendField(5, varint: throwawayNum)
        writer.appendField(6, bytes: diff)
        if !chaff.isEmpty {
            writer.appendField(7, bytes: chaff)
        }
        return writer.data
    }

    init(parsing data: Data) throws {
        var reader = ProtoReader(data)
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            switch (field, wireType) {
            case (1, 0): protocolVersion = UInt32(truncatingIfNeeded: try reader.readVarint())
            case (2, 0): oldNum = try reader.readVarint()
            case (3, 0): newNum = try reader.readVarint()
            case (4, 0): ackNum = try reader.readVarint()
            case (5, 0): throwawayNum = try reader.readVarint()
            case (6, 2): diff = Data(try reader.readLengthDelimited())
            case (7, 2): chaff = Data(try reader.readLengthDelimited())
            default: try reader.skipField(wireType: wireType)
            }
        }
    }
}

// MARK: - ClientBuffers.UserMessage (userinput.proto)

/// One user-input event. On the wire these are proto2 extensions of
/// `ClientBuffers.Instruction`:
///
/// - `Keystroke` extends Instruction at field 2 — a submessage whose
///   field 1 is the raw key bytes.
/// - `ResizeMessage` extends Instruction at field 3 — a submessage with
///   field 1 (varint width) and field 2 (varint height).
enum UserEvent: Equatable {
    case keystroke(keys: Data)
    case resize(width: Int, height: Int)
}

/// `message UserMessage { repeated Instruction instruction = 1; }`
struct UserMessage: Equatable {
    var events: [UserEvent]

    init(events: [UserEvent] = []) {
        self.events = events
    }

    func encoded() -> Data {
        var writer = ProtoWriter()
        for event in events {
            var instruction = ProtoWriter()
            switch event {
            case .keystroke(let keys):
                // userinput.proto: Keystroke extends Instruction at field 2,
                // and its `keys` bytes live at field 4 (wire-confirmed).
                var keystroke = ProtoWriter()
                keystroke.appendField(4, bytes: keys)
                instruction.appendField(2, bytes: keystroke.bytes)
            case .resize(let width, let height):
                // ResizeMessage extends Instruction at field 3; width=5, height=6.
                var resize = ProtoWriter()
                resize.appendField(5, varint: UInt64(bitPattern: Int64(width)))
                resize.appendField(6, varint: UInt64(bitPattern: Int64(height)))
                instruction.appendField(3, bytes: resize.bytes)
            }
            writer.appendField(1, bytes: instruction.bytes)
        }
        return writer.data
    }

    /// Parses a UserMessage, ignoring instructions carrying unknown
    /// extensions (forward compatibility, mirroring proto2 semantics).
    init(parsing data: Data) throws {
        var events: [UserEvent] = []
        var reader = ProtoReader(data)
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            guard field == 1, wireType == 2 else {
                try reader.skipField(wireType: wireType)
                continue
            }
            if let event = try Self.parseInstruction(reader.readLengthDelimited()) {
                events.append(event)
            }
        }
        self.events = events
    }

    private static func parseInstruction(_ bytes: [UInt8]) throws -> UserEvent? {
        var reader = ProtoReader(bytes)
        var event: UserEvent?
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            switch (field, wireType) {
            case (2, 2):
                event = .keystroke(keys: try parseKeystroke(reader.readLengthDelimited()))
            case (3, 2):
                event = try parseResize(reader.readLengthDelimited())
            default:
                try reader.skipField(wireType: wireType)
            }
        }
        return event
    }

    private static func parseKeystroke(_ bytes: [UInt8]) throws -> Data {
        var reader = ProtoReader(bytes)
        var keys = Data()
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            if field == 4, wireType == 2 {       // Keystroke.keys = field 4
                keys = Data(try reader.readLengthDelimited())
            } else {
                try reader.skipField(wireType: wireType)
            }
        }
        return keys
    }

    private static func parseResize(_ bytes: [UInt8]) throws -> UserEvent {
        var reader = ProtoReader(bytes)
        var width = 0
        var height = 0
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            switch (field, wireType) {
            case (5, 0): width = Int(Int64(bitPattern: try reader.readVarint()))   // width = field 5
            case (6, 0): height = Int(Int64(bitPattern: try reader.readVarint()))  // height = field 6
            default: try reader.skipField(wireType: wireType)
            }
        }
        return .resize(width: width, height: height)
    }
}

// MARK: - HostBuffers.HostMessage (hostinput.proto)

/// The host→client state-sync payload carried in `Instruction.diff`. Mirrors
/// `UserMessage`'s extension layout, but the events the server sends are:
///
/// - `HostBytes` (extension field 2) — `hoststring` (field 4) is a chunk of
///   raw terminal output (ANSI) produced by mosh's `Display::new_frame`.
/// - `ResizeMessage` (extension field 3) — width/height (rare, server-driven).
/// - `EchoAck` (extension field 4) — predictive-echo bookkeeping (ignored).
///
/// We only need the host bytes to drive SwiftTerm.
struct HostMessage {
    /// Concatenated terminal output across all HostBytes instructions, in order.
    var hostBytes: Data = Data()
    var resize: (width: Int, height: Int)?

    init(parsing data: Data) throws {
        var reader = ProtoReader(data)
        var bytes = Data()
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            guard field == 1, wireType == 2 else {
                try reader.skipField(wireType: wireType)
                continue
            }
            try Self.parseInstruction(reader.readLengthDelimited(), into: &bytes, resize: &resize)
        }
        self.hostBytes = bytes
    }

    private static func parseInstruction(_ raw: [UInt8], into bytes: inout Data,
                                         resize: inout (width: Int, height: Int)?) throws {
        var reader = ProtoReader(raw)
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            switch (field, wireType) {
            case (2, 2): // HostBytes extends Instruction at field 2 (hostinput.proto)
                bytes.append(try parseHostBytes(reader.readLengthDelimited()))
            case (3, 2): // ResizeMessage at field 3
                resize = try parseResize(reader.readLengthDelimited())
            default:
                try reader.skipField(wireType: wireType)
            }
        }
    }

    private static func parseHostBytes(_ raw: [UInt8]) throws -> Data {
        var reader = ProtoReader(raw)
        var s = Data()
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            if field == 4, wireType == 2 {       // HostBytes.hoststring = field 4
                s = Data(try reader.readLengthDelimited())
            } else {
                try reader.skipField(wireType: wireType)
            }
        }
        return s
    }

    private static func parseResize(_ raw: [UInt8]) throws -> (width: Int, height: Int) {
        var reader = ProtoReader(raw)
        var w = 0, h = 0
        while !reader.isAtEnd {
            let (field, wireType) = try reader.readKey()
            switch (field, wireType) {
            case (5, 0): w = Int(Int64(bitPattern: try reader.readVarint()))   // width = field 5
            case (6, 0): h = Int(Int64(bitPattern: try reader.readVarint()))   // height = field 6
            default: try reader.skipField(wireType: wireType)
            }
        }
        return (w, h)
    }
}

// MARK: - Transport fragments (not protobuf)

/// mosh fragments each (compressed) TransportInstruction across datagrams.
/// The fragment framing is a fixed 10-byte header followed by the payload
/// slice:
///
///     8 bytes  big-endian instruction id
///     2 bytes  big-endian fragment_num; the top bit (0x8000) marks the
///              final fragment of the instruction
struct TransportFragment: Equatable {
    static let headerSize = 10
    private static let finalBit: UInt16 = 0x8000

    let id: UInt64
    /// 15-bit fragment index within the instruction (0-based).
    let fragmentNum: UInt16
    let isFinal: Bool
    let contents: Data

    init(id: UInt64, fragmentNum: UInt16, isFinal: Bool, contents: Data) {
        self.id = id
        self.fragmentNum = fragmentNum & ~Self.finalBit
        self.isFinal = isFinal
        self.contents = contents
    }

    func encoded() -> Data {
        var out = Data(capacity: Self.headerSize + contents.count)
        var beId = id.bigEndian
        withUnsafeBytes(of: &beId) { out.append(contentsOf: $0) }
        var beNum = (fragmentNum | (isFinal ? Self.finalBit : 0)).bigEndian
        withUnsafeBytes(of: &beNum) { out.append(contentsOf: $0) }
        out.append(contents)
        return out
    }

    init(parsing data: Data) throws {
        guard data.count >= Self.headerSize else { throw MoshWireError.truncatedFragment }
        let bytes = [UInt8](data)
        var id: UInt64 = 0
        for i in 0..<8 {
            id = id << 8 | UInt64(bytes[i])
        }
        let rawNum = UInt16(bytes[8]) << 8 | UInt16(bytes[9])
        self.id = id
        self.fragmentNum = rawNum & ~Self.finalBit
        self.isFinal = rawNum & Self.finalBit != 0
        self.contents = Data(bytes[Self.headerSize...])
    }

    /// Splits an instruction payload into fragments that each fit in `mtu`
    /// bytes once the 10-byte header is added. Always yields at least one
    /// fragment so empty payloads still produce a (final) fragment.
    static func fragment(payload: Data, id: UInt64, mtu: Int) -> [TransportFragment] {
        let chunkSize = max(1, mtu - headerSize)
        var fragments: [TransportFragment] = []
        var index = payload.startIndex
        var num: UInt16 = 0
        repeat {
            let end = payload.index(index, offsetBy: chunkSize, limitedBy: payload.endIndex) ?? payload.endIndex
            fragments.append(TransportFragment(
                id: id,
                fragmentNum: num,
                isFinal: end == payload.endIndex,
                contents: Data(payload[index..<end])
            ))
            index = end
            num += 1
        } while index < payload.endIndex
        return fragments
    }
}

/// Reassembles `TransportFragment`s back into instruction payloads.
/// Fragments from instructions older than the newest seen id are dropped;
/// a newer id resets any partially assembled state (mosh transport
/// semantics: only the latest instruction matters).
struct FragmentAssembler {
    private var currentId: UInt64?
    private var pieces: [UInt16: Data] = [:]
    private var finalNum: UInt16?

    /// Feed one fragment; returns the full payload once every fragment of
    /// the current instruction has arrived, otherwise nil.
    mutating func receive(_ fragment: TransportFragment) -> Data? {
        if let current = currentId {
            if fragment.id < current {
                return nil // stale instruction
            }
            if fragment.id > current {
                reset(to: fragment.id)
            }
        } else {
            reset(to: fragment.id)
        }

        pieces[fragment.fragmentNum] = fragment.contents
        if fragment.isFinal {
            finalNum = fragment.fragmentNum
        }

        guard let finalNum, pieces.count == Int(finalNum) + 1 else { return nil }
        var payload = Data()
        for num in 0...finalNum {
            guard let piece = pieces[num] else { return nil } // duplicate filled gap
            payload.append(piece)
        }
        // Done with this instruction; keep id so stale retransmits are dropped.
        pieces.removeAll()
        self.finalNum = nil
        return payload
    }

    private mutating func reset(to id: UInt64) {
        currentId = id
        pieces.removeAll()
        finalNum = nil
    }
}
