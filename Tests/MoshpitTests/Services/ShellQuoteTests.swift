import Foundation
import Testing
@testable import Moshpit

/// `Shell.quote`, the app's one shell quoter.
///
/// The six copies it replaced escaped `'` with `replacingOccurrences`, which
/// matches grapheme clusters: an apostrophe that a combining mark, a
/// variation selector or an emoji modifier is attached to was left as it
/// was, closed the quote, and the rest of the value ran on the host. Every
/// check here reads the quoted word back as a POSIX shell does — a model of
/// one everywhere, and a real `/bin/sh` (with dash, bash, zsh and ksh where
/// present) wherever a test can spawn one — and wants the value back, whole,
/// with nothing outside the quotes.
@Suite("shell quoting")
struct ShellQuoteTests {

    // MARK: - Inputs

    /// Scalars that join an apostrophe BEFORE them into one `Character`:
    /// every combining diacritical mark, the other combining blocks,
    /// variation selectors, emoji modifiers, ZWNJ and ZWJ, the keycap mark,
    /// tags, spacing marks, a halfwidth sound mark.
    static let extenders: [Unicode.Scalar] = (
        Array(0x0300...0x036F)
            + [0x0483, 0x0591, 0x0610, 0x064B, 0x0903, 0x093E, 0x0E31, 0x0E33, 0x1AB0, 0x1DC0,
               0x200C, 0x200D, 0x20D0, 0x20E3, 0x302A, 0x3099, 0xFE00, 0xFE0E, 0xFE0F, 0xFE20,
               0xFF9E, 0x1F3FB, 0x1F3FC, 0x1F3FD, 0x1F3FE, 0x1F3FF, 0xE0020, 0xE007F, 0xE0100]
    ).compactMap(Unicode.Scalar.init)

    /// Values that must come back exactly, many carrying an injection that
    /// leaves a file named INJECTED behind if any of it runs.
    static let adversarial: [String] = {
        var values = [
            "", "plain", "it's", "'", "''", "'''", "\\", "\\'", "'\\", "a\\", "x'\\''y",
            "$HOME `whoami` $(touch INJECTED)", "*", "?", "[a]", "~root", "#comment", "-n", "--",
            "a b\tc", "line one\nline two\n", "\r\n", "\"double\"", "; touch INJECTED", "&& touch INJECTED",
            "| touch INJECTED", "> INJECTED", "x'; touch INJECTED; '", "x'\n touch INJECTED\n'",
            "中文 路径/文件 😀 👍🏽 👨‍👩‍👧 🇨🇳 é e\u{301}",
            "\u{202E}gnp.exe", "\u{FEFF}bom", "\u{1B}[201~", "\u{1B}]0;title\u{07}", "\u{7F}\u{01}\u{1F}",
            String((1...0x7F).map { Character(Unicode.Scalar(UInt8($0))) }),
            // A prepended concatenation mark joins the apostrophe AFTER it.
            "\u{600}'; touch INJECTED; '", "x\u{600}'; touch INJECTED; '",
            // A ZWJ emoji sequence, then an apostrophe with a modifier.
            "👩‍💻'🏽; touch INJECTED; '",
        ]
        for mark in extenders {
            values.append("x'\(mark); touch INJECTED; '")
            values.append("'\(mark)")
        }
        return values
    }()

    // MARK: - Against the model

    @Test("A plain value is wrapped; an apostrophe is closed, escaped and reopened")
    func shapes() {
        #expect(Shell.quote("plain") == "'plain'")
        #expect(Shell.quote("it's") == #"'it'\''s'"#)
        #expect(Shell.quote("$HOME `whoami`") == "'$HOME `whoami`'")
        #expect(Shell.quote("") == "''")
        #expect(Shell.quote("'") == #"''\'''"#)
    }

    /// The minimal inputs of the Rust ports' divergences (`quote-unicode` in
    /// crates/herdr, `quote-combining` in crates/install), and their output.
    @Test("An apostrophe with a combining mark after it is still escaped")
    func combiningMark() {
        #expect("'\u{301}".count == 1, "the hazard: one Character, and it isn't \"'\"")
        #expect(Shell.quote("x'\u{301}; echo INJECTED; '") == "'x'\\''\u{301}; echo INJECTED; '\\'''")
        #expect(Shell.quote("it'\u{301}s") == "'it'\\''\u{301}s'")
        #expect(Shell.quote("x'\u{FE0F}; touch /tmp/pwned; '") == "'x'\\''\u{FE0F}; touch /tmp/pwned; '\\'''")
        #expect(Shell.quote("a'\u{1F3FB}b") == "'a'\\''\u{1F3FB}b'")
    }

    @Test("Every grapheme extender after an apostrophe leaves it escaped", arguments: extenders)
    func everyExtender(_ mark: Unicode.Scalar) {
        let value = "x'\(mark); touch INJECTED; '"
        let quoted = Shell.quote(value)
        #expect(quoted == "'x'\\''\(mark); touch INJECTED; '\\'''")
        #expect(PosixShell.unquote(quoted) == value)
    }

    @Test("Every adversarial value reads back as itself, nothing outside the quotes")
    func modelRoundTrip() {
        for value in Self.adversarial {
            #expect(PosixShell.unquote(Shell.quote(value)) == value, "\(value.debugDescription)")
        }
    }

    /// B138: fish takes `\'` and `\\` as escapes inside single quotes, so a
    /// value ending in `\` left fish's quote open — the next word ran on —
    /// and `\\` came back as `\` (fish 3.7.1 and 4.6.0, in containers).
    @Test("Every adversarial value reads back as itself in fish too")
    func fishRoundTrip() {
        let backslashes = ["a\\", "\\\\", "c:\\path\\", "\\'", "x\\'; echo INJECTED; '", "a\\nb"]
        for value in Self.adversarial + backslashes {
            #expect(PosixShell.unquoteAsFish(Shell.quote(value)) == value, "\(value.debugDescription)")
            #expect(PosixShell.unquote(Shell.quote(value)) == value, "\(value.debugDescription)")
        }
        #expect(Shell.quote("a\\") == #"'a'\\''"#)
    }

    @Test("A script reaches sh as one word, whatever the login shell")
    func portable() {
        let script = #"herdr api snapshot; echo "__moshpit_rc=$?" 'a\'"#
        let line = Shell.portable(script)
        #expect(line.hasPrefix("sh -c '"))
        #expect(PosixShell.shPayload(line) == script)
        #expect(PosixShell.unquoteAsFish(String(line.dropFirst("sh -c ".count))) == script)
    }

    /// A NUL can't be one byte of a command line (it ends the C string); the
    /// quoter leaves it where it is, inside the quotes, for the shell to
    /// refuse the line — see `aNulRunsNothing`.
    @Test("A NUL stays inside the quotes")
    func nul() {
        #expect(Shell.quote("a\u{0}b") == "'a\u{0}b'")
        #expect(PosixShell.unquote(Shell.quote("a\u{0}'b")) == "a\u{0}'b")
    }

    @Test("Random values over the hazards read back as themselves")
    func randomRoundTrip() {
        let pieces: [String] = ["'", "\\", "\"", "$", "`", ";", " ", "\n", "\r", "\t", "#", "*", "~", "a", "é",
                                "\u{301}", "\u{FE0F}", "\u{1F3FB}", "\u{200D}", "\u{600}", "\u{903}", "😀",
                                "\u{1B}[201~", "中", "\u{0}"]
        var rng = SeededGenerator(seed: 0x5EED_0019)
        for _ in 0..<4_000 {
            let value = (0..<Int.random(in: 0...12, using: &rng))
                .map { _ in pieces.randomElement(using: &rng)! }.joined()
            #expect(PosixShell.unquote(Shell.quote(value)) == value, "\(value.debugDescription)")
            #expect(PosixShell.unquoteAsFish(Shell.quote(value)) == value, "\(value.debugDescription)")
        }
    }

    @Test("A very long value is quoted whole, in linear time")
    func veryLong() {
        let value = String(repeating: "a'\u{301}b \u{FE0F}'\n", count: 50_000)
        let started = ContinuousClock.now
        let quoted = Shell.quote(value)
        #expect(ContinuousClock.now - started < .seconds(10))
        #expect(quoted.utf8.count == value.utf8.count + 2 + 3 * 100_000)
        #expect(PosixShell.unquote(quoted) == value)
    }

    // MARK: - Against a real shell

    /// The values in one `printf '%s\0' …` from a scratch directory, so a
    /// value that got out of its quotes leaves `INJECTED` there.
    @Test("/bin/sh, dash, bash, zsh and ksh print every value back byte for byte, and run none of it",
          .enabled(if: PosixShell.available))
    func realShellRoundTrip() throws {
        let directory = try PosixShell.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let values = Self.adversarial.filter { !$0.unicodeScalars.contains("\u{0}") }
        let command = "cd \(Shell.quote(directory.path)) && printf '%s\\0' "
            + values.map(Shell.quote).joined(separator: " ")
        for shell in ["/bin/sh"] + PosixShell.otherShells {
            let result = try #require(PosixShell.run(command, shell: shell))
            #expect(result.exitCode == 0, "\(shell): \(result.text)")
            let printed = result.output.split(separator: 0, omittingEmptySubsequences: false).dropLast()
            #expect(printed.count == values.count, "\(shell)")
            for (value, output) in zip(values, printed) {
                #expect(Data(output) == Data(value.utf8), "\(shell): \(value.debugDescription)")
            }
            #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("INJECTED").path),
                    "\(shell) ran part of a quoted value")
        }
    }

    @Test("A long value comes back from /bin/sh whole", .enabled(if: PosixShell.available))
    func realShellLongValue() throws {
        let directory = try PosixShell.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        // ~140 KB quoted: inside ARG_MAX, past any line buffer.
        let value = String(repeating: "a'\u{301}b; touch INJECTED; '\n", count: 4_000)
        let command = "cd \(Shell.quote(directory.path)) && printf %s \(Shell.quote(value))"
        let result = try #require(PosixShell.run(command))
        #expect(result.exitCode == 0)
        #expect(result.output == Data(value.utf8))
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("INJECTED").path))
    }

    /// A command line is a C string: it ends at the NUL, inside the quotes,
    /// which leaves them open — the shell refuses the line, so neither the
    /// value's tail nor anything after it runs.
    @Test("A NUL ends the line inside the quotes: the shell refuses it, nothing runs",
          .enabled(if: PosixShell.available))
    func aNulRunsNothing() throws {
        let directory = try PosixShell.temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let command = "cd \(Shell.quote(directory.path)) && printf %s \(Shell.quote("a\u{0}'; touch INJECTED; '"))"
            + "; touch AFTER"
        let result = try #require(PosixShell.run(command))
        #expect(result.exitCode != 0)
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("INJECTED").path))
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("AFTER").path))
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
