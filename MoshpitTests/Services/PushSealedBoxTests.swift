import Foundation
import Testing
@testable import Moshpit

/// The push envelope, from the phone's side.
///
/// Format v1 is implemented three times — `scripts/moshpit-push.sh` (openssl, on
/// the dev host), `push-relay/sealbox` (Go), and `PushSealedBox` (here) — and the
/// only thing keeping them honest is one frozen vector. If a change breaks
/// agreement, exactly one of these suites goes red; without them, every
/// notification in the field would silently become undecryptable and the app
/// would keep showing the generic fallback with nothing to explain why.
@Suite("Push sealed box")
struct PushSealedBoxTests {

    /// The same secret the Go tests and scripts/push-vector.sh use.
    static let secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

    // MARK: - The frozen cross-language vector

    /// Produced by the exact openssl pipeline the host script runs, with the IV
    /// pinned so the bytes are reproducible. Regenerate with
    /// `scripts/push-vector.sh` and update BOTH this and
    /// `push-relay/sealbox/sealbox_test.go` — a vector only one side agrees with
    /// tests nothing.
    enum Vector {
        static let iv = "000102030405060708090a0b0c0d0e0f"
        static let ct = "21HMutLOqDHXUpjykOXFikZCpepvsm5jV5f/9b8UHg7qOFw7ihvJXuhQMQ9iG/gbmStNwvoMQDCC6nukKAF2bmyrRIX4+lriKtR+xgIN1WWSeBXfoh2nk0KhJvMsvXqd9xJIt+ShRcbZH+NePfCXwHiOcRQ/Q3ZO89pgX/d3TO8yI0UEsVNyclSjlgqri/d/9l5q9KB/4xFJ/+mxZFLcm56XO8DAXn6nQt2mQ9Z7Dt6bBqw68BRhNdH05E7O3OqR"
        static let mac = "ZiSLy5cGr2XOMlJiX3ep8bVSAcmERPDSMpHnysktnBA="
        static let encHex = "4ff81bf9801df5440677a30d2622a56d9ecfc556ea57c383cb057bde865e058d"
        static let macHex = "7fe30bcca8aa9e48de360f5d4cd48f1f83dfc68345ae6861f5cd449f5247ca31"
        static let plaintext = #"{"conn":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","host":"m1-pro","sess":"work","pane":"%3","agent":"claude","state":"attention","title":"Bash: rm -rf \"build\" 构建","ts":1755900000}"#

        static var envelope: PushSealedBox.Envelope {
            PushSealedBox.Envelope(v: 1, iv: iv, ct: ct, mac: mac)
        }
    }

    @Test("key derivation matches openssl byte for byte")
    func derivation() throws {
        let keys = try PushSealedBox.deriveKeys(secretHex: Self.secret)
        #expect(keys.encHex == Vector.encHex)
        #expect(keys.macHex == Vector.macHex)
        #expect(keys.encHex != keys.macHex)
    }

    @Test("opens an envelope sealed by the host shell script")
    func opensOpenSSLVector() throws {
        let status = try PushSealedBox.open(Vector.envelope, secretHex: Self.secret)
        #expect(status.conn == "3F2504E0-4F89-11D3-9A0C-0305E82C3301")
        #expect(status.host == "m1-pro")
        #expect(status.sess == "work")
        #expect(status.pane == "%3")
        #expect(status.agent == "claude")
        #expect(status.state == "attention")
        #expect(status.title == #"Bash: rm -rf "build" 构建"#)
        #expect(status.ts == 1_755_900_000)
    }

    @Test("reproduces the openssl bytes for the same input and IV")
    func reproducesVector() throws {
        let sealed = try PushSealedBox.seal(Data(Vector.plaintext.utf8),
                                           secretHex: Self.secret, ivHex: Vector.iv)
        #expect(sealed.ct == Vector.ct)
        #expect(sealed.mac == Vector.mac)
    }

    // MARK: - Format behaviour

    @Test("round-trips payloads of every padding shape")
    func roundTrip() throws {
        for plaintext in ["", "x", String(repeating: "a", count: 16),
                          String(repeating: "字", count: 200)] {
            let sealed = try PushSealedBox.seal(Data(plaintext.utf8), secretHex: Self.secret)
            let opened = try PushSealedBox.openRaw(sealed, secretHex: Self.secret)
            #expect(String(data: opened, encoding: .utf8) == plaintext)
        }
    }

    @Test("a fresh IV is used for every seal")
    func freshIV() throws {
        let a = try PushSealedBox.seal(Data("same".utf8), secretHex: Self.secret)
        let b = try PushSealedBox.seal(Data("same".utf8), secretHex: Self.secret)
        #expect(a.iv != b.iv)
        #expect(a.ct != b.ct)
    }

    @Test("the secret is case-insensitive, as the shell's tr makes it")
    func secretCase() throws {
        let opened = try PushSealedBox.open(Vector.envelope,
                                            secretHex: Self.secret.uppercased())
        #expect(opened.host == "m1-pro")
    }

    @Test("tampering is rejected, and at the MAC rather than the padding")
    func tampering() throws {
        func flip(_ b64: String) -> String {
            var bytes = [UInt8](Data(base64Encoded: b64)!)
            bytes[0] ^= 0x01
            return Data(bytes).base64EncodedString()
        }
        let cases: [String: PushSealedBox.Envelope] = [
            "flipped ciphertext": .init(v: 1, iv: Vector.iv, ct: flip(Vector.ct), mac: Vector.mac),
            "flipped MAC": .init(v: 1, iv: Vector.iv, ct: Vector.ct, mac: flip(Vector.mac)),
            "swapped IV": .init(v: 1, iv: String(repeating: "00", count: 16), ct: Vector.ct, mac: Vector.mac),
            "stripped MAC": .init(v: 1, iv: Vector.iv, ct: Vector.ct, mac: ""),
            "future version": .init(v: 2, iv: Vector.iv, ct: Vector.ct, mac: Vector.mac),
            "empty ciphertext": .init(v: 1, iv: Vector.iv, ct: "", mac: Vector.mac),
            "short IV": .init(v: 1, iv: "00", ct: Vector.ct, mac: Vector.mac),
            "non-block ciphertext": .init(v: 1, iv: Vector.iv,
                                          ct: Data("abc".utf8).base64EncodedString(),
                                          mac: Vector.mac),
        ]
        for (name, envelope) in cases {
            #expect(throws: (any Error).self, "\(name) was accepted") {
                try PushSealedBox.open(envelope, secretHex: Self.secret)
            }
        }
    }

    @Test("the wrong secret fails authentication, never decryption")
    func wrongSecret() {
        // A padding error here would mean the ciphertext was decrypted before it
        // was authenticated — the shape a CBC padding oracle needs.
        #expect(throws: PushSealedBox.Failure.authenticationFailed) {
            try PushSealedBox.open(Vector.envelope, secretHex: String(repeating: "ff", count: 32))
        }
    }

    @Test("malformed secrets are refused outright")
    func badSecrets() {
        for secret in ["", "abc", String(repeating: "z", count: 64), String(repeating: "a", count: 63)] {
            #expect(throws: PushSealedBox.Failure.badSecret) {
                try PushSealedBox.deriveKeys(secretHex: secret)
            }
        }
    }

    @Test("plaintext that is not a status is an error, not an empty status")
    func nonStatusPlaintext() throws {
        let sealed = try PushSealedBox.seal(Data("not json".utf8), secretHex: Self.secret)
        #expect(throws: PushSealedBox.Failure.malformed) {
            try PushSealedBox.open(sealed, secretHex: Self.secret)
        }
    }

    @Test("hex decoding refuses anything but clean pairs")
    func hexStrictness() {
        #expect(Data(moshpitHex: "00ff") != nil)
        #expect(Data(moshpitHex: "0f0") == nil)      // odd length
        #expect(Data(moshpitHex: "00zz") == nil)     // non-hex
        #expect(Data(moshpitHex: "00 ff") == nil)    // whitespace
        #expect(Data([0x00, 0xff]).moshpitHexString == "00ff")
    }
}
