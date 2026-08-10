import Foundation
import Testing
@testable import Moshpit

/// Exercises `TmuxLayoutParser`, the recursive-descent parser for tmux's
/// `layout-string` window descriptor. The emphasis is on the two things a
/// parser fed live server output must get right: correct trees for real
/// layouts, and *graceful* nil-returns (never a crash) for every shape of
/// truncated, malformed, or pathologically deep input.
@Suite("TmuxLayoutParser")
struct TmuxLayoutParserTests {

    // MARK: - Well-formed layouts

    @Test("single-pane layout parses to a leaf carrying id + geometry")
    func singlePane() {
        // <4-hex checksum>,<WxH>,<x>,<y>,<pane-id>
        let node = TmuxLayoutParser.parse("b25d,208x62,0,0,1")
        #expect(node == .pane(id: 1, width: 208, height: 62))
        #expect(node?.paneIds == [1])
        #expect(node?.width == 208)
        #expect(node?.height == 62)
    }

    @Test("brace group parses as a left-to-right row split")
    func rowSplit() {
        let node = TmuxLayoutParser.parse("b25d,208x62,0,0{104x62,0,0,1,103x62,105,0,2}")
        #expect(node == .row([
            .pane(id: 1, width: 104, height: 62),
            .pane(id: 2, width: 103, height: 62),
        ], width: 208, height: 62))
        // paneIds preserves visual left-to-right order.
        #expect(node?.paneIds == [1, 2])
    }

    @Test("bracket group parses as a top-to-bottom column split")
    func columnSplit() {
        let node = TmuxLayoutParser.parse("abcd,208x62,0,0[208x31,0,0,1,208x30,0,32,2]")
        #expect(node == .column([
            .pane(id: 1, width: 208, height: 31),
            .pane(id: 2, width: 208, height: 30),
        ], width: 208, height: 62))
        #expect(node?.paneIds == [1, 2])
    }

    @Test("nested layout (row containing a column) parses to the full tree")
    func nestedRowWithColumn() {
        // The exact example from the parser's doc comment: one pane beside a
        // vertical stack of two.
        let raw = "b25d,208x62,0,0{104x62,0,0,1,103x62,105,0[103x31,105,0,2,103x30,105,32,3]}"
        let node = TmuxLayoutParser.parse(raw)
        #expect(node == .row([
            .pane(id: 1, width: 104, height: 62),
            .column([
                .pane(id: 2, width: 103, height: 31),
                .pane(id: 3, width: 103, height: 30),
            ], width: 103, height: 62),
        ], width: 208, height: 62))
        // Flattened ids walk the tree depth-first in visual order.
        #expect(node?.paneIds == [1, 2, 3])
    }

    @Test("three-way row split keeps every child")
    func threeWayRow() {
        let node = TmuxLayoutParser.parse("0000,300x50,0,0{100x50,0,0,1,100x50,100,0,2,100x50,200,0,3}")
        #expect(node?.paneIds == [1, 2, 3])
    }

    // MARK: - Malformed / truncated input (must degrade to nil, never crash)

    @Test("empty string returns nil")
    func emptyString() {
        #expect(TmuxLayoutParser.parse("") == nil)
    }

    @Test("no checksum comma at all returns nil")
    func noComma() {
        #expect(TmuxLayoutParser.parse("208x62") == nil)
    }

    @Test("checksum present but body missing returns nil")
    func checksumOnly() {
        #expect(TmuxLayoutParser.parse("b25d,") == nil)
    }

    @Test("missing the 'x' size separator returns nil")
    func missingXSeparator() {
        #expect(TmuxLayoutParser.parse("b25d,208-62,0,0,1") == nil)
    }

    @Test("non-numeric dimension returns nil")
    func nonNumericDimension() {
        #expect(TmuxLayoutParser.parse("b25d,WxH,0,0,1") == nil)
    }

    @Test("geometry present but neither pane-id nor children returns nil")
    func geometryWithoutTerminator() {
        // Valid size/offsets, then the string simply ends — no ",id" and no group.
        #expect(TmuxLayoutParser.parse("b25d,208x62,0,0") == nil)
    }

    @Test("unclosed brace group returns nil rather than looping")
    func unclosedBrace() {
        #expect(TmuxLayoutParser.parse("b25d,208x62,0,0{104x62,0,0,1") == nil)
    }

    @Test("unclosed bracket group returns nil")
    func unclosedBracket() {
        #expect(TmuxLayoutParser.parse("b25d,208x62,0,0[208x31,0,0,1") == nil)
    }

    @Test("group with a trailing comma but no following node returns nil")
    func danglingSeparator() {
        #expect(TmuxLayoutParser.parse("b25d,208x62,0,0{104x62,0,0,1,}") == nil)
    }

    @Test("mismatched close bracket (brace opened, bracket to close) returns nil")
    func mismatchedDelimiters() {
        #expect(TmuxLayoutParser.parse("b25d,208x62,0,0{104x62,0,0,1]") == nil)
    }

    @Test("truncated mid-number returns nil")
    func truncatedMidNumber() {
        #expect(TmuxLayoutParser.parse("b25d,208x") == nil)
    }

    @Test("pure garbage after the checksum returns nil")
    func garbageBody() {
        #expect(TmuxLayoutParser.parse("b25d,!!!not-a-layout!!!") == nil)
    }

    // MARK: - Pathological depth (recursion must not crash)

    /// Build a legal, fully-balanced layout nested `depth` levels deep — each
    /// level is a single-child brace group wrapping the next, bottoming out in
    /// one pane. Confirms the recursive descent handles deep-but-valid input.
    private func deeplyNested(depth: Int) -> String {
        var body = "10x10,0,0,1"                 // innermost leaf pane
        for _ in 0..<depth {
            body = "100x100,0,0{\(body)}"
        }
        return "b25d,\(body)"
    }

    @Test("a moderately deep but well-formed layout parses to its single leaf")
    func deepWellFormedParses() {
        let node = TmuxLayoutParser.parse(deeplyNested(depth: 200))
        // It parses successfully and the single leaf pane id survives the descent.
        #expect(node?.paneIds == [1])
    }

    @Test("a deeply nested but truncated layout degrades to nil without crashing")
    func deepTruncatedIsGraceful() {
        // Same deep structure, but strip every closing brace: the parser must
        // unwind the whole recursion and return nil, not trap.
        let open = deeplyNested(depth: 300).replacingOccurrences(of: "}", with: "")
        #expect(TmuxLayoutParser.parse(open) == nil)
    }

    // MARK: - Node accessors

    @Test("width/height accessors read through every node case")
    func accessorsCoverAllCases() {
        let pane = TmuxLayoutNode.pane(id: 1, width: 10, height: 20)
        let row = TmuxLayoutNode.row([pane], width: 30, height: 40)
        let column = TmuxLayoutNode.column([pane], width: 50, height: 60)
        #expect(pane.width == 10 && pane.height == 20)
        #expect(row.width == 30 && row.height == 40)
        #expect(column.width == 50 && column.height == 60)
    }
}
