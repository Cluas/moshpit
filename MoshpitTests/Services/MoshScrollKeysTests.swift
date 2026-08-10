import Foundation
import Testing
@testable import Moshpit

/// Mosh+tmux scrollback is driven by sending copy-mode keystrokes to the
/// mosh-rendered tmux client (the -CC sidecar's copy-mode doesn't repaint that
/// separate client). This suite pins the EXACT bytes our code emits;
/// `scripts/verify-tmux-scroll.py` independently proves those same bytes scroll
/// a real tmux 3.6 client. Together they verify Bug B end-to-end without a
/// device: code emits X (here) ∧ X scrolls real tmux (the script) ⇒ it works.
@Suite("Mosh copy-mode scroll keys")
struct MoshScrollKeysTests {

    typealias S = SessionHub.ActiveSession
    private let cb = Data([0x02])                       // C-b
    private let pageUp = Data([0x1b, 0x5b, 0x35, 0x7e]) // ESC [ 5 ~
    private let pageDown = Data([0x1b, 0x5b, 0x36, 0x7e])
    private let lbracket: UInt8 = 0x5b                  // '['

    @Test("scroll up while live enters copy-mode: prefix + [ + PageUp")
    func upEntersCopyMode() {
        let keys = S.moshCopyKeys(lines: 3, prefix: cb, alreadyInCopyMode: false)
        #expect(keys == cb + Data([lbracket]) + pageUp)
    }

    @Test("scroll up while already in copy-mode: just PageUp (no re-entry)")
    func upWhileInCopyMode() {
        #expect(S.moshCopyKeys(lines: 1, prefix: cb, alreadyInCopyMode: true) == pageUp)
    }

    @Test("scroll down in copy-mode is PageDown; while live it's a no-op")
    func down() {
        #expect(S.moshCopyKeys(lines: -2, prefix: cb, alreadyInCopyMode: true) == pageDown)
        #expect(S.moshCopyKeys(lines: -2, prefix: cb, alreadyInCopyMode: false) == nil)
    }

    @Test("zero lines sends nothing")
    func zero() {
        #expect(S.moshCopyKeys(lines: 0, prefix: cb, alreadyInCopyMode: false) == nil)
        #expect(S.moshCopyKeys(lines: 0, prefix: cb, alreadyInCopyMode: true) == nil)
    }

    @Test("a remapped prefix is honored (C-a)")
    func remappedPrefix() {
        let ca = Data([0x01])
        #expect(S.moshCopyKeys(lines: 1, prefix: ca, alreadyInCopyMode: false)
                == ca + Data([lbracket]) + pageUp)
    }

    @Test("exit key is q")
    func exitKey() {
        #expect(S.moshCopyExitKey == Data([0x71]))
    }

    @Test("typing in copy-mode prepends q so the key leaves copy-mode and reaches the shell")
    func inputExitsCopyMode() {
        let key = Data("ls\r".utf8)
        // In copy-mode (after a scroll): q + the keystrokes — fixes "can't type
        // / can't exit" once scrolled. scripts/verify-tmux-scroll.py proves q exits.
        #expect(S.moshInputKeys(key, inCopyMode: true) == Data([0x71]) + key)
        // Live: passed through untouched.
        #expect(S.moshInputKeys(key, inCopyMode: false) == key)
    }

    // MARK: prefix parsing (show-options -gqv prefix → bytes)

    @Test("Ctrl-letter prefixes parse to their control byte")
    func prefixBytesControl() {
        #expect(S.prefixBytes(from: "C-b") == Data([0x02]))
        #expect(S.prefixBytes(from: "C-a") == Data([0x01]))
        #expect(S.prefixBytes(from: " C-a ") == Data([0x01]))   // trimmed
        #expect(S.prefixBytes(from: "C-A") == Data([0x01]))     // case-insensitive
    }

    @Test("unknown / non-control prefix falls back to C-b")
    func prefixBytesFallback() {
        #expect(S.prefixBytes(from: "") == Data([0x02]))
        #expect(S.prefixBytes(from: "M-a") == Data([0x02]))
        #expect(S.prefixBytes(from: "garbage") == Data([0x02]))
    }

    // MARK: wheel events (mouse-app scroll over tmux — Claude Code, less --mouse)

    @Test("wheel up emits SGR button 64 at the given cell")
    func wheelUp() {
        #expect(S.wheelBytes(lines: 1, col: 40, row: 12) == Data("\u{1b}[<64;40;12M".utf8))
    }

    @Test("wheel down emits SGR button 65")
    func wheelDown() {
        #expect(S.wheelBytes(lines: -1, col: 40, row: 12) == Data("\u{1b}[<65;40;12M".utf8))
    }

    @Test("multiple lines repeat the event, clamped to 6 notches")
    func wheelClamp() {
        #expect(S.wheelBytes(lines: 3, col: 1, row: 1)
                == Data(String(repeating: "\u{1b}[<64;1;1M", count: 3).utf8))
        #expect(S.wheelBytes(lines: 50, col: 1, row: 1)
                == Data(String(repeating: "\u{1b}[<64;1;1M", count: 6).utf8))
    }

    @Test("zero lines sends nothing; cell coords floor to a 1-based cell")
    func wheelEdge() {
        #expect(S.wheelBytes(lines: 0, col: 5, row: 5) == Data())
        #expect(S.wheelBytes(lines: -1, col: 0, row: 0) == Data("\u{1b}[<65;1;1M".utf8))
    }
}

/// Horizontal-swipe pane/window switching cycles ordered ids with wrap-around.
/// `TmuxSessionController.switchPaneOrWindow` feeds pane ids (when the window
/// has splits) or window ids into this pure helper.
@Suite("Swipe pane/window cycling")
struct SwitchCyclingTests {
    typealias C = TmuxSessionController

    @Test("forward and backward move one step")
    func step() {
        #expect(C.cycled(["a", "b", "c"], from: "a", forward: true) == "b")
        #expect(C.cycled(["a", "b", "c"], from: "b", forward: false) == "a")
    }

    @Test("wraps around both ends")
    func wrap() {
        #expect(C.cycled(["a", "b", "c"], from: "c", forward: true) == "a")
        #expect(C.cycled(["a", "b", "c"], from: "a", forward: false) == "c")
    }

    @Test("nothing to switch returns nil")
    func none() {
        #expect(C.cycled(["a"], from: "a", forward: true) == nil)       // single item
        #expect(C.cycled([], from: nil, forward: true) == nil)          // empty
        #expect(C.cycled(["a", "b"], from: "x", forward: true) == nil)  // active not present
        #expect(C.cycled(["a", "b"], from: nil, forward: true) == nil)  // no active
    }
}
