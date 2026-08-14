import Foundation
import Testing
@testable import Moshpit

/// Round-trip tests for the mosh datagram pipeline pieces that don't need a
/// live server: compression, crypto framing, and bootstrap parsing.
@Suite("Mosh pipeline")
struct MoshPipelineTests {

    // MARK: Compression

    @Test("zlib compress → decompress round-trips arbitrary payloads")
    func compressionRoundTrip() throws {
        for sample in [Data(), Data("hello".utf8), Data(repeating: 0x41, count: 5000),
                       Data((0..<256).map { UInt8($0) })] {
            let packed = try MoshCompression.compress(sample)
            #expect(packed.first == 0x78, "zlib header byte present")
            let unpacked = try MoshCompression.decompress(packed)
            #expect(unpacked == sample)
        }
    }

    @Test("Adler-32 matches the RFC 1950 example")
    func adler32Known() {
        // Adler-32 of "Wikipedia" is 0x11E60398.
        #expect(MoshCompression.adler32(Data("Wikipedia".utf8)) == 0x11E6_0398)
    }

    // MARK: Crypto framing

    @Test("seal → open round-trips a Message and recovers seq/direction")
    func cryptoRoundTrip() throws {
        let key = Data((0..<16).map { UInt8($0) })
        let crypto = try MoshCrypto(key: key)
        let msg = MoshCrypto.Message(timestamp: 0x1234, timestampReply: 0xABCD,
                                     payload: Data("payload-bytes".utf8))
        let packet = try crypto.seal(msg, seq: 42, direction: .toServer)

        // Wire nonce is the first 8 bytes, big-endian, top bit = direction (0).
        #expect(packet.count > 8 + 16)
        let opened = try crypto.open(packet)
        #expect(opened.timestamp == 0x1234)
        #expect(opened.timestampReply == 0xABCD)
        #expect(opened.payload == Data("payload-bytes".utf8))
        #expect(opened.seq == 42)
        #expect(opened.direction == .toServer)
    }

    @Test("a tampered datagram fails authentication")
    func cryptoTamperRejected() throws {
        let key = Data(repeating: 0x5A, count: 16)
        let crypto = try MoshCrypto(key: key)
        var packet = try crypto.seal(
            MoshCrypto.Message(timestamp: 1, timestampReply: 2, payload: Data([0xDE, 0xAD])),
            seq: 7, direction: .toClient)
        packet[packet.count - 1] ^= 0xFF   // flip a tag byte
        #expect(throws: (any Error).self) { try crypto.open(packet) }
    }

    // MARK: Bootstrap parsing

    @Test("MOSH CONNECT line parses into host/port/16-byte key")
    func bootstrapParse() throws {
        // 22-char unpadded base64 = 16 bytes.
        let keyB64 = Data((0..<16).map { UInt8($0) }).base64EncodedString()
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
        let output = """
        \r
        MOSH CONNECT 60001 \(keyB64)\r
        \r
        mosh-server (mosh 1.4.0) [build mosh 1.4.0]\r
        """
        let creds = try MoshBootstrap.parse(output: output, host: "10.0.0.5")
        #expect(creds.host == "10.0.0.5")
        #expect(creds.udpPort == 60001)
        #expect(creds.key.count == 16)
        #expect(creds.key == Data((0..<16).map { UInt8($0) }))
    }

    @Test("missing MOSH CONNECT line throws")
    func bootstrapNoLine() {
        #expect(throws: (any Error).self) {
            try MoshBootstrap.parse(output: "permission denied\n", host: "h")
        }
    }

    /// A well-formed connect line carrying `port`, for the range tests below.
    private static func connectLine(port: String) -> String {
        let keyB64 = Data((0..<16).map { UInt8($0) }).base64EncodedString()
            .trimmingCharacters(in: CharacterSet(charactersIn: "="))
        return "MOSH CONNECT \(port) \(keyB64)\r\n"
    }

    @Test("a port that parses but cannot be a port is an error, not a crash",
          arguments: ["0", "-1", "65536", "70000", "99999999999"])
    func bootstrapRejectsUnusablePort(_ port: String) {
        // The out-of-range ones used to reach `MoshTransport.init` and trap in
        // `UInt16(udpPort)` — an actual crash at connect time, reachable without
        // a hostile server: a banner interleaving with mosh-server's line shifts
        // which token lands in `parts[2]`, and a PID or timestamp parses as an
        // Int perfectly well. "0" is here for completeness rather than crash
        // safety (`NWEndpoint.Port(rawValue: 0)` is `Optional(0)`, not nil) — it
        // simply cannot connect, so it should fail here and say so.
        #expect(throws: MoshBootstrap.BootstrapError.self) {
            try MoshBootstrap.parse(output: Self.connectLine(port: port), host: "h")
        }
    }

    @Test("the edges of the usable range still parse",
          arguments: [("1", UInt16(1)), ("65535", UInt16(65535)), ("60001", UInt16(60001))])
    func bootstrapAcceptsPortRange(_ input: String, _ expected: UInt16) throws {
        let creds = try MoshBootstrap.parse(output: Self.connectLine(port: input), host: "h")
        #expect(creds.udpPort == expected)
    }

    @Test("a non-numeric port field is still the missing-line error")
    func bootstrapNonNumericPort() {
        // Unchanged behaviour, pinned so the new range guard doesn't quietly
        // reclassify it: this one never had a number to range-check.
        #expect(throws: MoshBootstrap.BootstrapError.self) {
            try MoshBootstrap.parse(output: Self.connectLine(port: "sixty"), host: "h")
        }
    }

    // MARK: Bootstrap command construction

    /// The probe's PATH extension, prepended for bare binary names so the
    /// non-interactive exec channel finds Homebrew installs (see `command`).
    private static let pathPrefix = "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" "

    @Test("configured UDP port range reaches mosh-server as -p start:end")
    func bootstrapCommandForwardsPortRange() {
        let cmd = MoshBootstrap.command(serverBinary: "mosh-server", locale: "en_US.UTF-8",
                                        portRangeStart: 60001, portRangeEnd: 60999)
        #expect(cmd == Self.pathPrefix + "mosh-server new -s -c 256 -p 60001:60999 -l LANG=en_US.UTF-8")
    }

    @Test("no configured range omits -p (mosh-server default range)")
    func bootstrapCommandDefaultRange() {
        let cmd = MoshBootstrap.command(serverBinary: "mosh-server", locale: "en_US.UTF-8",
                                        portRangeStart: nil, portRangeEnd: nil)
        #expect(cmd == Self.pathPrefix + "mosh-server new -s -c 256 -l LANG=en_US.UTF-8")
    }

    @Test("explicit server path is trusted verbatim — no PATH prefix")
    func bootstrapCommandExplicitPathUnprefixed() {
        let cmd = MoshBootstrap.command(serverBinary: "/opt/homebrew/bin/mosh-server",
                                        locale: "en_US.UTF-8",
                                        portRangeStart: nil, portRangeEnd: nil)
        #expect(cmd == "/opt/homebrew/bin/mosh-server new -s -c 256 -l LANG=en_US.UTF-8")
    }

    @Test("degenerate ranges degrade instead of erroring remotely")
    func bootstrapCommandDegenerateRanges() {
        // Inverted / equal end → single-port form.
        #expect(MoshBootstrap.command(serverBinary: "m", locale: "L",
                                      portRangeStart: 60050, portRangeEnd: 60001)
                == Self.pathPrefix + "m new -s -c 256 -p 60050 -l LANG=L")
        // Out-of-range start → no -p at all.
        #expect(MoshBootstrap.command(serverBinary: "m", locale: "L",
                                      portRangeStart: 0, portRangeEnd: 61000)
                == Self.pathPrefix + "m new -s -c 256 -l LANG=L")
        #expect(MoshBootstrap.command(serverBinary: "m", locale: "L",
                                      portRangeStart: 70000, portRangeEnd: 71000)
                == Self.pathPrefix + "m new -s -c 256 -l LANG=L")
    }

    @Test("noConnectLine error escapes control chars and caps excerpt length")
    func bootstrapErrorSanitizesOutput() {
        // A hostile remote embeds an ANSI escape (ESC[2J clears the screen) plus
        // a huge junk payload. Neither should survive into the error string.
        let hostile = "\u{1B}[2J\u{07}oops" + String(repeating: "A", count: 500)
        let message = MoshBootstrap.BootstrapError.noConnectLine(hostile).description

        // No raw control bytes leak through.
        #expect(!message.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) })
        // The ESC and BEL are rendered as visible escapes instead.
        #expect(message.contains("\\u{001B}"))
        #expect(message.contains("\\u{0007}"))
        // Ordinary text is preserved.
        #expect(message.contains("oops"))
        // The 500-char junk tail is truncated well under the raw length.
        #expect(message.contains("…"))
        #expect(message.count < 200)
    }

    // MARK: Fragment + instruction end-to-end (compressed)

    @Test("instruction → compress → fragment → reassemble → decompress → parse")
    func transportInstructionPipeline() throws {
        var inst = TransportInstruction()
        inst.oldNum = 3
        inst.newNum = 9
        inst.ackNum = 5
        inst.diff = UserMessage(events: [.keystroke(keys: Data("ls -la\r".utf8))]).encoded()

        let compressed = try MoshCompression.compress(inst.encoded())
        let fragments = TransportFragment.fragment(payload: compressed, id: 1, mtu: 1300)

        var assembler = FragmentAssembler()
        var reassembled: Data?
        for fragment in fragments {
            let bytes = fragment.encoded()
            let parsed = try TransportFragment(parsing: bytes)
            if let done = assembler.receive(parsed) { reassembled = done }
        }

        let payload = try #require(reassembled)
        let restored = try TransportInstruction(parsing: MoshCompression.decompress(payload))
        #expect(restored.oldNum == 3)
        #expect(restored.newNum == 9)
        #expect(restored.ackNum == 5)
        let user = try UserMessage(parsing: restored.diff)
        #expect(user.events == [.keystroke(keys: Data("ls -la\r".utf8))])
    }
}
