import Foundation

/// Bracketed paste (xterm's DECSET 2004): pasted text sent between
/// `ESC [ 2 0 0 ~` and `ESC [ 2 0 1 ~`, so the program reading it takes it as
/// text — a newline in it is a newline, not Return.
///
/// The end marker is the whole defence, and pasted text can carry one. A
/// program ends the paste at the FIRST `ESC [ 2 0 1 ~` it reads, so a marker
/// inside the text ends it early, and what follows arrives as typed keys: a
/// line of crafted clipboard text after one ran as a command, Return and all
/// (seen live through herdr 0.9.5's frame channel and through tmux 3.6a's
/// `paste-buffer -p`, into zsh). So no paste leaves the app with a marker
/// inside it, whatever the transport and whoever adds the brackets:
/// ``body(_:)`` is the one place they are taken out, and every paste path —
/// herdr's frame channel, tmux's paste buffer, mosh, plain SSH, the terminal
/// view's own Paste and ⌘V — goes through it.
enum BracketedPaste {
    /// `ESC [ 2 0 0 ~`
    static let start = Data([0x1B, 0x5B, 0x32, 0x30, 0x30, 0x7E])
    /// `ESC [ 2 0 1 ~`
    static let end = Data([0x1B, 0x5B, 0x32, 0x30, 0x31, 0x7E])

    /// `text` as UTF-8 with every start and end marker taken out — matched as
    /// bytes, and taken out until none is left.
    ///
    /// Bytes, because that is how the program at the other end matches them:
    /// compared as `Character`s (what `replacingOccurrences` does), a marker
    /// whose `~` carries a combining mark is another string, so it stayed in
    /// the text and ended the paste. Until none is left, because taking one
    /// out joins the bytes on either side, and those can form another:
    /// `ESC[20` `ESC[201~` `1~` is a whole end marker once the middle one is
    /// gone. Removing each marker the moment its last byte is read gets there
    /// in one pass: the bytes kept so far never hold a marker, so the only
    /// one a new byte can complete ends at that byte.
    static func body(_ text: String) -> Data {
        var kept: [UInt8] = []
        kept.reserveCapacity(text.utf8.count)
        for byte in text.utf8 {
            kept.append(byte)
            if byte == 0x7E, endsWithMarker(kept) {
                kept.removeLast(6)
            }
        }
        return Data(kept)
    }

    /// ``body(_:)`` as text, for a path that hands the text on for someone
    /// else to bracket (tmux's `paste-buffer -p`). Taking ASCII out of UTF-8
    /// leaves valid UTF-8, so nothing else about the text changes.
    static func sanitized(_ text: String) -> String {
        String(decoding: body(text), as: UTF8.self)
    }

    /// `text` as one complete bracketed paste.
    static func wrapped(_ text: String) -> Data {
        start + body(text) + end
    }

    /// What a paste of `text` sends: one bracketed paste when the program
    /// asked for them, the bare text when it didn't — markers taken out either
    /// way, since that answer is only the local emulator's last reading of
    /// the program's mode.
    static func payload(_ text: String, bracketed: Bool) -> Data {
        bracketed ? wrapped(text) : body(text)
    }

    /// Whether `bytes` ends with a start or an end marker.
    private static func endsWithMarker(_ bytes: [UInt8]) -> Bool {
        let count = bytes.count
        guard count >= 6 else { return false }
        return bytes[count - 6] == 0x1B && bytes[count - 5] == 0x5B
            && bytes[count - 4] == 0x32 && bytes[count - 3] == 0x30
            && (bytes[count - 2] == 0x30 || bytes[count - 2] == 0x31)
            && bytes[count - 1] == 0x7E
    }
}
