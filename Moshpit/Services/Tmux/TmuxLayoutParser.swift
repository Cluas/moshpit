import Foundation

/// Parsed form of tmux's window layout string, e.g.
/// `b25d,208x62,0,0{104x62,0,0,1,103x62,105,0[103x31,105,0,2,103x30,105,32,3]}`
///
/// Grammar (after the 4-hex-digit checksum + comma):
///   node     = size ( "," pane-id | children )
///   size     = WxH "," X "," Y
///   children = "{" node ("," node)* "}"   — left-to-right split
///            | "[" node ("," node)* "]"   — top-to-bottom split
indirect enum TmuxLayoutNode: Equatable {
    case pane(id: Int, width: Int, height: Int)
    case row([TmuxLayoutNode], width: Int, height: Int)     // {} side by side
    case column([TmuxLayoutNode], width: Int, height: Int)  // [] stacked

    var width: Int {
        switch self {
        case .pane(_, let w, _), .row(_, let w, _), .column(_, let w, _): return w
        }
    }

    var height: Int {
        switch self {
        case .pane(_, _, let h), .row(_, _, let h), .column(_, _, let h): return h
        }
    }

    /// All pane ids in visual order.
    var paneIds: [Int] {
        switch self {
        case .pane(let id, _, _): return [id]
        case .row(let kids, _, _), .column(let kids, _, _):
            return kids.flatMap(\.paneIds)
        }
    }
}

enum TmuxLayoutParser {

    /// Parse a raw layout descriptor. Returns nil on malformed input.
    static func parse(_ raw: String) -> TmuxLayoutNode? {
        guard let comma = raw.firstIndex(of: ",") else { return nil }
        var scanner = Substring(raw[raw.index(after: comma)...])
        return parseNode(&scanner)
    }

    private static func parseNode(_ s: inout Substring) -> TmuxLayoutNode? {
        guard let width = parseInt(&s), consume(&s, "x"),
              let height = parseInt(&s), consume(&s, ","),
              parseInt(&s) != nil, consume(&s, ","),   // x offset (unused)
              parseInt(&s) != nil                       // y offset (unused)
        else { return nil }

        if consume(&s, "{") {
            guard let kids = parseChildren(&s, until: "}") else { return nil }
            return .row(kids, width: width, height: height)
        }
        if consume(&s, "[") {
            guard let kids = parseChildren(&s, until: "]") else { return nil }
            return .column(kids, width: width, height: height)
        }
        if consume(&s, ",") {
            guard let paneId = parseInt(&s) else { return nil }
            return .pane(id: paneId, width: width, height: height)
        }
        return nil
    }

    private static func parseChildren(_ s: inout Substring, until close: Character) -> [TmuxLayoutNode]? {
        var kids: [TmuxLayoutNode] = []
        while true {
            guard let kid = parseNode(&s) else { return nil }
            kids.append(kid)
            if consume(&s, String(close)) { return kids }
            guard consume(&s, ",") else { return nil }
        }
    }

    private static func parseInt(_ s: inout Substring) -> Int? {
        let digits = s.prefix(while: \.isNumber)
        guard !digits.isEmpty else { return nil }
        s = s.dropFirst(digits.count)
        return Int(digits)
    }

    @discardableResult
    private static func consume(_ s: inout Substring, _ token: String) -> Bool {
        guard s.hasPrefix(token) else { return false }
        s = s.dropFirst(token.count)
        return true
    }
}
