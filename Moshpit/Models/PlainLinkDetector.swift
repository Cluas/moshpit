import Foundation
import SwiftTerm

/// Makes bare `http(s)://` URLs tappable even when the remote program never
/// wrapped them in a real OSC-8 hyperlink — e.g. Claude Code CLI only does
/// that for a URL standing alone on its own line, not one mentioned inline
/// within a sentence or inside a recap/summary box. Without this, those
/// URLs render as plain text with no visual cue that they're clickable, and
/// aren't actually tappable either.
///
/// SwiftTerm has its own built-in implicit-link regex (`linkReporting =
/// .implicit`), but it's deliberately not used here: it also matches bare
/// file/relative paths (`/tmp/foo`, `src/bar`) as if they were links, which
/// in a terminal-heavy client means constant false positives. This scans
/// with its own scheme-anchored detector instead and only ever tags text
/// that actually starts with `http://`/`https://` in the source.
enum PlainLinkDetector {
    private static let detector = try? NSDataDetector(
        types: NSTextCheckingResult.CheckingType.link.rawValue)

    /// Scan every currently-visible row for bare http(s) URLs and tag them
    /// via `Terminal.tagPlainTextLink` — the same `cell.hasPayload` path a
    /// real OSC-8 hyperlink uses, so they pick up the exact same underline
    /// (`linkHighlightMode = .always`) and tap-to-open behavior for free.
    /// Idempotent and cheap to call after every feed: re-tagging an
    /// already-tagged cell (whether by a prior call here, or by a real OSC-8
    /// sequence) is harmless, and lines without an "http" substring skip the
    /// detector entirely.
    ///
    /// Rows that soft-wrapped mid-URL are joined into one logical line
    /// before scanning, via `Terminal.isRowWrapped` — otherwise a URL longer
    /// than the terminal width (common at phone widths) got truncated to
    /// its first-row fragment, or its wrapped tail (no longer starting with
    /// "http") was never tagged at all, leaving a visually-continuous
    /// underline where the opened link was actually just the first row's
    /// half. (An earlier version inferred wrapping from a row's trimmed
    /// text exactly filling the column width — a plausible-looking stand-in
    /// that broke, silently, the moment a double-width character — CJK,
    /// emoji — anywhere earlier on that row threw off a character count vs.
    /// column count comparison, without affecting the rendering itself,
    /// which is correctly column-based.)
    static func linkify(_ terminalView: TerminalView) {
        linkify(terminal: terminalView.getTerminal())
    }

    static func linkify(terminal: Terminal) {
        guard let detector else { return }
        var row = 0
        while row < terminal.rows {
            var spans: [(row: Int, length: Int)] = []
            var text = ""
            var currentRow = row
            while true {
                let rowText = terminal.viewportLineText(row: currentRow)
                spans.append((currentRow, rowText.count))
                text += rowText
                guard terminal.isRowWrapped(currentRow), currentRow + 1 < terminal.rows else { break }
                currentRow += 1
            }
            row = currentRow + 1

            guard text.contains("http") else { continue }
            let fullRange = NSRange(text.startIndex..., in: text)
            detector.enumerateMatches(in: text, range: fullRange) { match, _, _ in
                guard let match, let url = match.url,
                      let swiftRange = Range(match.range, in: text) else { return }
                // NSDataDetector's .link type also matches bare domains
                // ("example.com") and other non-URL data it's tuned for —
                // only keep matches that actually spelled out a scheme in
                // the source text, so a stray "config.io"-looking path
                // fragment can't get linkified.
                let matchedText = text[swiftRange]
                guard matchedText.hasPrefix("http://") || matchedText.hasPrefix("https://") else { return }
                // Character (grapheme-cluster) distance, not UTF-16 offset —
                // `viewportLineText` emits exactly one Character per
                // terminal column (SwiftTerm's default `translateToString`
                // behavior), so this maps directly to columns. Raw NSRange
                // offsets would drift on a line with an astral-plane
                // character (e.g. an emoji) before the match.
                let start = text.distance(from: text.startIndex, to: swiftRange.lowerBound)
                let length = text.distance(from: swiftRange.lowerBound, to: swiftRange.upperBound)
                guard length > 0 else { return }
                // A URL that runs into the right edge of a full-width final
                // row may continue on the next PHYSICAL line even though the
                // emulator saw a hard newline: programs that do their own
                // layout (Claude Code's transcript is the constant case)
                // word-wrap by printing each visual line separately, often
                // indenting the continuation — no isRowWrapped bit is ever
                // set, so the logical-line join above can't see it.
                var taggedURL = url.absoluteString
                var tails: [(row: Int, startCol: Int, length: Int)] = []
                if swiftRange.upperBound == text.endIndex,
                   let lastSpan = spans.last, lastSpan.length == terminal.cols {
                    tails = hardWrapContinuation(after: lastSpan.row, terminal: terminal)
                    if !tails.isEmpty {
                        var joined = String(matchedText)
                        for tail in tails {
                            let rowText = terminal.viewportLineText(row: tail.row)
                            let from = rowText.index(rowText.startIndex, offsetBy: tail.startCol)
                            let to = rowText.index(from, offsetBy: tail.length)
                            joined += rowText[from..<to]
                        }
                        taggedURL = joined
                    }
                }
                tagAcrossRows(startOffset: start, length: length, spans: spans,
                              terminal: terminal, url: taggedURL)
                for tail in tails {
                    terminal.tagPlainTextLink(row: tail.row, startCol: tail.startCol,
                                              endCol: tail.startCol + tail.length - 1,
                                              url: taggedURL)
                }
            }
        }
    }

    /// Characters a hard-wrapped URL tail may consist of. Deliberately
    /// narrower than RFC 3986: prose-punctuation members of the reserved set
    /// (parens, quotes, commas, semicolons, bangs) are left out so a
    /// continuation run stops where a sentence plausibly starts.
    private static func isURLTailChar(_ c: Character) -> Bool {
        if c.isASCII && (c.isLetter || c.isNumber) { return true }
        return "-._~/%?#=&:@+".contains(c)
    }

    /// Walks the physical rows after a URL that ended flush against the right
    /// edge, collecting indented URL-charset runs that look like the rest of
    /// it. Heuristic by nature — the emulator genuinely saw separate lines —
    /// so each run must be at least two characters and contain something
    /// other than a letter (a digit, slash, dash…): a real tail almost always
    /// does, while the word a new prose sentence starts with almost never
    /// does. That guard is what keeps a COMPLETE url that happens to end at
    /// the last column, followed by ordinary text, from being extended into a
    /// broken one — that case renders and taps correctly today and must stay
    /// working.
    private static func hardWrapContinuation(after row: Int, terminal: Terminal)
        -> [(row: Int, startCol: Int, length: Int)] {
        var tails: [(row: Int, startCol: Int, length: Int)] = []
        var current = row + 1
        // Enough for a ~5×-terminal-width URL; also the runaway stop.
        let maxTailRows = 4
        while current < terminal.rows, tails.count < maxTailRows {
            // A continuation the emulator itself knows about belongs to the
            // logical-line join in linkify, not to this heuristic.
            let rowText = terminal.viewportLineText(row: current)
            let indent = rowText.prefix(while: { $0 == " " }).count
            let afterIndent = rowText.dropFirst(indent)
            let run = afterIndent.prefix(while: isURLTailChar)
            guard run.count >= 2, run.contains(where: { !$0.isLetter }) else { break }
            tails.append((current, indent, run.count))
            // Keep walking only while the run itself hits the right edge —
            // a tail that stops mid-row is the URL's end.
            guard indent + run.count == terminal.cols else { break }
            current += 1
        }
        return tails
    }

    /// Splits a logical-line match back into per-physical-row column ranges
    /// (it may cross one or more soft-wrap boundaries) and tags each row's
    /// slice individually — `tagPlainTextLink` only accepts a single row.
    private static func tagAcrossRows(startOffset: Int, length: Int,
                                      spans: [(row: Int, length: Int)],
                                      terminal: Terminal, url: String) {
        var offset = startOffset
        var remaining = length
        for (row, rowLength) in spans {
            guard remaining > 0 else { break }
            guard offset < rowLength else {
                offset -= rowLength
                continue
            }
            let startCol = offset
            let take = min(rowLength - startCol, remaining)
            guard take > 0 else { break }
            terminal.tagPlainTextLink(row: row, startCol: startCol,
                                      endCol: startCol + take - 1, url: url)
            remaining -= take
            offset = 0
        }
    }
}
