import Foundation
import SwiftTerm

/// Makes the paths an agent reports tappable — "对比图：qchar/out/GripLab.png",
/// "saved to ~/Desktop/charts/" — so its output opens on the phone instead of
/// waiting for a desk. The companion of `PlainLinkDetector`: same visible-row
/// walk, same soft-wrap join, same `tagPlainTextLink` cell payload, so a path
/// link underlines and taps exactly like a URL does.
///
/// Only paths the HOST confirmed get tagged (`RemotePathLinks`), so the text
/// itself never has to be trusted: a candidate the probe hasn't answered yet
/// is left plain and picked up by the pass that runs when the answer lands.
enum PathLinkDetector {
    @MainActor
    static func linkify(terminal: Terminal, resolver: RemotePathLinks) {
        var row = 0
        while row < terminal.rows {
            var spans: [(row: Int, length: Int)] = []
            var text = ""
            var current = row
            while true {
                let rowText = terminal.viewportLineText(row: current)
                spans.append((current, rowText.count))
                text += rowText
                guard terminal.isRowWrapped(current), current + 1 < terminal.rows else { break }
                current += 1
            }
            row = current + 1

            guard text.contains("/") || text.contains(".") else { continue }
            for candidate in TerminalPathScanner.scan(text) {
                guard let first = PlainLinkDetector.cell(atOffset: candidate.offset, spans: spans) else { continue }
                // Whatever already links here wins — an OSC-8 link, a URL, or
                // this pass's own tag from an earlier run (which also keeps
                // a confirmed path from being looked up again every pass).
                if terminal.link(at: .screen(Position(col: first.col, row: first.row)),
                                 mode: .explicitOnly) != nil { continue }
                guard let path = resolver.resolvedPath(for: candidate) else { continue }
                PlainLinkDetector.tagAcrossRows(startOffset: candidate.offset, length: candidate.length,
                                                spans: spans, terminal: terminal,
                                                url: RemotePathLink.url(for: path))
            }
        }
    }
}
