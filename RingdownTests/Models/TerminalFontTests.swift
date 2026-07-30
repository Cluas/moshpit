import Foundation
import Testing
@testable import Ringdown

@Suite("TerminalFont")
struct TerminalFontTests {

    @Test("system id yields a monospaced system font at the requested size")
    func systemFont() {
        let f = TerminalFont.font(id: "system", size: 15)
        #expect(f.pointSize == 15)
    }

    @Test("a real family loads at the requested size")
    func namedFamily() {
        let f = TerminalFont.font(id: "Menlo", size: 13)
        #expect(f.pointSize == 13)
        #expect(f.fontName.localizedCaseInsensitiveContains("menlo"))
    }

    @Test("an unknown family falls back to the system font, never nil")
    func unknownFallback() {
        let f = TerminalFont.font(id: "NoSuchFont-XYZ", size: 12)
        #expect(f.pointSize == 12)   // resolved to a valid font, not a crash
    }

    @Test("label lookup maps ids to display names with a safe default")
    func labels() {
        #expect(TerminalFont.label(for: "system") == "SF Mono")
        #expect(TerminalFont.label(for: "Menlo") == "Menlo")
        #expect(TerminalFont.label(for: "garbage") == "SF Mono")
    }
}

@Suite("UDPPortRange validation")
struct UDPPortRangeTests {

    @Test("a valid range parses to its integer pair")
    func valid() throws {
        let r = UDPPortRange.validate(from: " 60000 ", to: "61000")
        #expect(try r.get() == (60000, 61000))
    }

    @Test("equal endpoints are allowed (single port)")
    func equal() throws {
        #expect(try UDPPortRange.validate(from: "5000", to: "5000").get() == (5000, 5000))
    }

    @Test("non-numeric input is rejected")
    func nonNumeric() {
        #expect(UDPPortRange.validate(from: "abc", to: "61000") == .failure(.nonNumeric))
    }

    @Test("out-of-bounds ports are rejected")
    func outOfBounds() {
        #expect(UDPPortRange.validate(from: "0", to: "61000") == .failure(.outOfBounds))
        #expect(UDPPortRange.validate(from: "1", to: "70000") == .failure(.outOfBounds))
    }

    @Test("inverted range (from > to) is rejected")
    func inverted() {
        #expect(UDPPortRange.validate(from: "61000", to: "60000") == .failure(.inverted))
    }
}

extension Result where Success == (Int, Int), Failure == UDPPortRange.ValidationError {
    /// Tuple isn't Equatable, so compare via a helper for the tests above.
    static func == (lhs: Self, rhs: Self) -> Bool {
        switch (lhs, rhs) {
        case (.failure(let a), .failure(let b)): return a == b
        default: return false
        }
    }
}
