import Foundation
import Testing
@testable import Moshpit

/// `BracketedPaste`, the one place paste markers come out of pasted text.
///
/// A program ends a bracketed paste at the first `ESC[201~` it reads, so a
/// marker left in the text ends the paste early and the rest is typed:
/// Return and all. Two ways one used to survive — a combining mark on its
/// `~` (the removal compared `Character`s), and a marker assembled by
/// removing another (one pass) — and the mosh, plain SSH, tmux and terminal
/// view paths removed none at all.
@Suite("bracketed paste")
struct BracketedPasteTests {

    static let start = "\u{1B}[200~"
    static let end = "\u{1B}[201~"

    private func text(_ data: Data) -> String { String(decoding: data, as: UTF8.self) }

    /// Every place `marker` occurs in `data`, as bytes.
    private func occurrences(of marker: Data, in data: Data) -> Int {
        guard data.count >= marker.count else { return 0 }
        let bytes = [UInt8](data), pattern = [UInt8](marker)
        return (0...(bytes.count - pattern.count)).filter { Array(bytes[$0..<($0 + pattern.count)]) == pattern }.count
    }

    /// Removal the slow, obvious way: every marker, as bytes, pass after
    /// pass until a pass finds none. What `body` must agree with.
    private func naive(_ text: String) -> Data {
        var bytes = [UInt8](text.utf8)
        let markers = [[UInt8](BracketedPaste.start), [UInt8](BracketedPaste.end)]
        while true {
            var out: [UInt8] = []
            var index = 0
            var removed = false
            while index < bytes.count {
                if let marker = markers.first(where: { index + $0.count <= bytes.count
                                                        && Array(bytes[index..<(index + $0.count)]) == $0 }) {
                    index += marker.count
                    removed = true
                } else {
                    out.append(bytes[index])
                    index += 1
                }
            }
            bytes = out
            if !removed { return Data(bytes) }
        }
    }

    @Test("Plain text passes through, wrapped once")
    func plain() {
        #expect(BracketedPaste.body("first line\nsecond line") == Data("first line\nsecond line".utf8))
        #expect(BracketedPaste.wrapped("/tmp/shot.png") == Data("\(Self.start)/tmp/shot.png\(Self.end)".utf8))
        #expect(BracketedPaste.wrapped("") == Data("\(Self.start)\(Self.end)".utf8))
        #expect(BracketedPaste.start == Data(Self.start.utf8))
        #expect(BracketedPaste.end == Data(Self.end.utf8))
    }

    @Test("End and start markers in the text are taken out")
    func markers() {
        #expect(text(BracketedPaste.body("one\(Self.end)two")) == "onetwo")
        #expect(text(BracketedPaste.body("one\(Self.start)two")) == "onetwo")
        #expect(text(BracketedPaste.body("\(Self.start)\(Self.end)\(Self.end)")) == "")
    }

    /// `paste-marker-unicode`: the removal compared `Character`s, and
    /// `~` + U+0301 is one, so this marker stayed.
    @Test("A marker whose ~ carries a combining mark is taken out")
    func combiningMark() {
        let pasted = "ls\(Self.end)\u{301}\necho INJECTED\n"
        #expect(text(BracketedPaste.body(pasted)) == "ls\u{301}\necho INJECTED\n")
        let wrapped = BracketedPaste.wrapped(pasted)
        #expect(occurrences(of: BracketedPaste.end, in: wrapped) == 1)
        #expect(wrapped.suffix(6) == BracketedPaste.end)
        for mark in ["\u{301}", "\u{FE0F}", "\u{1F3FB}", "\u{200D}", "\u{20E3}"] {
            #expect(text(BracketedPaste.body("a\(Self.end)\(mark)b")) == "a\(mark)b")
            #expect(text(BracketedPaste.body("a\(Self.start)\(mark)b")) == "a\(mark)b")
        }
    }

    /// `paste-end-marker`: one pass took the middle marker out and left
    /// `ESC[20` + `1~`, a whole one.
    @Test("A marker assembled by taking another out is taken out too")
    func assembled() {
        #expect(text(BracketedPaste.body("a\u{1B}[20\(Self.end)1~b")) == "ab")
        #expect(text(BracketedPaste.body("\u{1B}[20\(Self.end)1~\necho INJECTED\n")) == "\necho INJECTED\n")
        #expect(text(BracketedPaste.body("\u{1B}[20\u{1B}[20\(Self.end)1~1~")) == "")
        // Across kinds: removing an end marker assembles a start marker.
        #expect(text(BracketedPaste.body("\u{1B}[20\(Self.end)0~x")) == "x")
        #expect(text(BracketedPaste.body("\u{1B}\(Self.start)[201~")) == "")
    }

    @Test("Bytes that only look like a marker are left alone")
    func nearMisses() {
        for kept in ["\u{1B}[201", "\u{1B}[202~", "[201~", "\u{1B}[2O1~", "\u{1B}\u{1B}", "\u{1B}[200",
                     "~\u{1B}[20 1~", "\u{1B}[201\u{301}~"] {
            #expect(text(BracketedPaste.body(kept)) == kept, "\(kept.debugDescription)")
        }
    }

    @Test("A payload is wrapped only when the program asked, and sanitized either way")
    func payload() {
        let pasted = "ls\(Self.end)\necho INJECTED\n"
        #expect(BracketedPaste.payload(pasted, bracketed: true)
                == Data("\(Self.start)ls\necho INJECTED\n\(Self.end)".utf8))
        #expect(BracketedPaste.payload(pasted, bracketed: false) == Data("ls\recho INJECTED\r".utf8),
                "unbracketed, a line break is Return (B574)")
    }

    /// B574: a paste the program didn't ask to have bracketed is typed: its
    /// line breaks go out as Return, CRLF and a lone LF as CR, as xterm,
    /// iTerm and Terminal.app send them (a CRLF text sent as is doubled
    /// every line break in a program that reads raw). Bracketed: as it is.
    @Test("B574: an unbracketed paste's line breaks are Return; a bracketed one is byte for byte")
    func unbracketedLineBreaksAreReturn() {
        #expect(BracketedPaste.payload("a\r\nb\nc\rd", bracketed: false) == Data("a\rb\rc\rd".utf8))
        #expect(BracketedPaste.payload("\r\n\r\n", bracketed: false) == Data("\r\r".utf8))
        #expect(BracketedPaste.payload("中文 no breaks", bracketed: false) == Data("中文 no breaks".utf8))
        #expect(BracketedPaste.payload("a\r\nb\nc", bracketed: true)
                == Data("\(Self.start)a\r\nb\nc\(Self.end)".utf8))
    }

    @Test("Random pieces of markers come out as the repeated removal says, never leaving one")
    func randomAgainstRepeatedRemoval() {
        let pieces = [Self.end, Self.start, "\u{1B}[20", "\u{1B}[201", "\u{1B}[200", "1~", "0~", "~", "\u{1B}",
                      "[", "2", "0", "1", "x", "\u{301}", "\n", "é"]
        var rng = SeededGenerator(seed: 0x9A57_E019)
        for _ in 0..<5_000 {
            let pasted = (0..<Int.random(in: 0...14, using: &rng))
                .map { _ in pieces.randomElement(using: &rng)! }.joined()
            let body = BracketedPaste.body(pasted)
            #expect(body == naive(pasted), "\(pasted.debugDescription)")
            #expect(occurrences(of: BracketedPaste.end, in: body) == 0, "\(pasted.debugDescription)")
            #expect(occurrences(of: BracketedPaste.start, in: body) == 0, "\(pasted.debugDescription)")
            let wrapped = BracketedPaste.wrapped(pasted)
            #expect(occurrences(of: BracketedPaste.end, in: wrapped) == 1)
            #expect(occurrences(of: BracketedPaste.start, in: wrapped) == 1)
        }
    }

    /// Deep nesting is what makes pass-after-pass removal quadratic.
    @Test("Deeply nested markers come out in one linear pass")
    func deepNesting() {
        let depth = 50_000
        let pasted = String(repeating: "\u{1B}[20", count: depth) + Self.end + String(repeating: "1~", count: depth)
            + "\necho INJECTED\n"
        let started = ContinuousClock.now
        let body = BracketedPaste.body(pasted)
        #expect(ContinuousClock.now - started < .seconds(10))
        #expect(text(body) == "\necho INJECTED\n")
    }
}

/// SplitMix64, seeded, so a random case that fails fails again.
private struct SeededGenerator: RandomNumberGenerator {
    private var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}
