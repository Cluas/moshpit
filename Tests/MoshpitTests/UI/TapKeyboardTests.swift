import Testing
@testable import Moshpit

/// The tap-splitting rule: WHERE a tap lands decides whether it means "I want
/// to type" (the cursor's own rows — Claude Code's input box, a shell prompt)
/// or is just a tap (history, a link, a mouse-aware program's own buttons).
@Suite("Tap → keyboard decision")
struct TapKeyboardTests {

    @Test("a tap on the cursor's rows asks for the keyboard")
    func inputAreaRaises() {
        // Claude Code's input box: cursor on the text line, border rows around.
        #expect(TerminalScrollGesture.tapWantsKeyboard(tapRow: 40, cursorRow: 40,
                                                       readingScrollback: false))
        #expect(TerminalScrollGesture.tapWantsKeyboard(tapRow: 38, cursorRow: 40,
                                                       readingScrollback: false))
        #expect(TerminalScrollGesture.tapWantsKeyboard(tapRow: 42, cursorRow: 40,
                                                       readingScrollback: false))
    }

    @Test("a tap on history is just a tap — jump-to-bottom must not cost a keyboard")
    func historyDoesNot() {
        #expect(!TerminalScrollGesture.tapWantsKeyboard(tapRow: 10, cursorRow: 40,
                                                        readingScrollback: false))
        #expect(!TerminalScrollGesture.tapWantsKeyboard(tapRow: 37, cursorRow: 40,
                                                        readingScrollback: false))
    }

    @Test("while scrolled up, even the cursor's viewport rows are history")
    func scrollbackSuppresses() {
        // The cursor is off-screen below; whatever sits under its viewport row
        // is old output, not a prompt.
        #expect(!TerminalScrollGesture.tapWantsKeyboard(tapRow: 40, cursorRow: 40,
                                                        readingScrollback: true))
    }
}
