import Foundation

/// Finds real bells in a byte stream without parsing the rest of it.
///
/// While a tmux window's pin is handed back, pane output is deliberately not fed
/// to SwiftTerm (see `TmuxSessionController.handlePaneOutput`) — so its parser
/// isn't there to raise the bell. The bell still has to get through: it is the
/// "an agent needs you" signal the whole background-notification story rests on.
///
/// Scanning for a bare `0x07` is not good enough, and shipping that scan is what
/// fired "agent needs your input" several times a second: **OSC strings are
/// terminated by BEL**, and a coding agent sets the terminal title (`ESC ] 0 ; …
/// BEL`) on essentially every redraw. So this tracks whether the stream is
/// inside a string sequence — OSC/DCS/APC/PM/SOS, introduced by `ESC` followed
/// by `]`, `P`, `_`, `^`, `X`, and ended by BEL or ST (`ESC \`) — and reports a
/// BEL only outside one, which is what SwiftTerm's own parser does.
///
/// State carries across calls because tmux can split a sequence across
/// `%output` chunks.
struct BellDetector {
    /// True while inside an OSC/DCS/APC/PM/SOS string, where a BEL is a
    /// terminator rather than a bell.
    private var inString = false
    /// True immediately after an `ESC`, when the next byte selects the sequence.
    private var afterEscape = false

    /// Feed a chunk. Returns true if it contained at least one real bell.
    mutating func containsBell(_ data: Data) -> Bool {
        var rang = false
        for byte in data {
            if afterEscape {
                afterEscape = false
                switch byte {
                case 0x5D, 0x50, 0x5F, 0x5E, 0x58:   // ] P _ ^ X — a string opens
                    inString = true
                case 0x5C:                            // \ — ST closes one
                    inString = false
                default:
                    break
                }
                continue
            }
            switch byte {
            case 0x1B:                                // ESC
                afterEscape = true
            case 0x07:                                // BEL
                if inString {
                    inString = false                  // it terminated the string
                } else {
                    rang = true
                }
            default:
                break
            }
        }
        return rang
    }

    /// Forget any half-parsed sequence. Used when output starts or stops being
    /// fed elsewhere, so a sequence split across that boundary can't leave this
    /// stuck believing it is inside a string.
    mutating func reset() {
        inString = false
        afterEscape = false
    }
}
