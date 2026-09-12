import Foundation
import Testing
@testable import Moshpit

/// The bell scanner used while pane output bypasses SwiftTerm's parser. Its
/// whole reason to be careful is that OSC strings END in BEL: a coding agent
/// retitles the terminal on every redraw, and counting those as bells fired an
/// "agent needs your input" notification several times a second.
@Suite("Bell detection in raw pane output")
struct BellDetectorTests {

    private func rang(_ chunks: [String]) -> Bool {
        var detector = BellDetector()
        var result = false
        for chunk in chunks {
            if detector.containsBell(Data(chunk.utf8)) { result = true }
        }
        return result
    }

    @Test("a bare BEL is a bell")
    func bareBell() {
        #expect(rang(["\u{07}"]))
        #expect(rang(["done\u{07}\r\n"]))
    }

    @Test("an OSC title terminated by BEL is NOT a bell")
    func oscTitleIsNotABell() {
        #expect(!rang(["\u{1b}]0;claude — moshpit\u{07}"]))
        #expect(!rang(["\u{1b}]2;~/code/moshpit\u{07}some text"]))
        // The real flood: a redraw that retitles several times in one chunk.
        #expect(!rang([String(repeating: "\u{1b}]0;t\u{07}", count: 20)]))
    }

    @Test("an OSC closed by ST leaves no state behind to swallow a later bell")
    func stTerminatedString() {
        #expect(rang(["\u{1b}]0;title\u{1b}\\\u{07}"]))
        #expect(!rang(["\u{1b}]0;title\u{1b}\\"]))
    }

    @Test("a bell after an OSC still rings")
    func bellAfterOSC() {
        #expect(rang(["\u{1b}]0;title\u{07}", "\u{07}"]))
    }

    @Test("a sequence split across chunks keeps its state")
    func splitAcrossChunks() {
        // The BEL that closes the title arrives in the NEXT chunk.
        #expect(!rang(["\u{1b}]0;a long ti", "tle\u{07}"]))
        // …and the ESC ] itself can be split too.
        #expect(!rang(["\u{1b}", "]0;title\u{07}"]))
    }

    @Test("DCS / APC / PM / SOS strings swallow their BEL like OSC does")
    func otherStringSequences() {
        for intro in ["P", "_", "^", "X"] {
            #expect(!rang(["\u{1b}\(intro)payload\u{07}"]),
                    "ESC \(intro) opens a string; its BEL is a terminator")
        }
    }

    @Test("reset forgets a half-parsed sequence")
    func resetClearsState() {
        var detector = BellDetector()
        _ = detector.containsBell(Data("\u{1b}]0;unterminated".utf8))
        detector.reset()
        // Hoisted out of `#expect` — the macro captures its body in a closure,
        // where a mutating call on a local `var` is not allowed.
        let rangAfterReset = detector.containsBell(Data("\u{07}".utf8))
        #expect(rangAfterReset,
                "after a reset the stream is no longer treated as inside a string")
    }
}
