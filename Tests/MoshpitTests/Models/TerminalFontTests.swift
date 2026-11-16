import Foundation
import Testing
@testable import Moshpit

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
}
