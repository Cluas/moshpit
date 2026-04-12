#!/usr/bin/env swift
// Standalone test for TmuxControlClient protocol parsing.
// Run: swift scripts/test_tmux_cc.swift

import Foundation

// ============================================================
// Inline a minimal TmuxControlClient for testing (no Combine)
// ============================================================

class TmuxControlClientTest {
    var panes: [String: PaneInfo] = [:]
    var windows: [String: WindowInfo] = [:]
    var activeWindowId = ""
    var isAttached = false

    struct PaneInfo { let id, windowId, title, command: String; let width, height: Int; let isActive: Bool }
    struct WindowInfo { let id, name, layout: String; let index: Int; let isActive: Bool }

    var onPaneOutput: ((String, Data) -> Void)?
    var onReady: (() -> Void)?

    private var rawBuffer = Data()
    private var insideResponseBlock = false
    private var pendingResponseLines: [String] = []
    private var commandQueue: [(id: Int, completion: (String) -> Void)] = []
    private var nextCommandId = 0
    private var readyFired = false
    private var sendRaw: ((String) -> Void)?

    func attach(sessionName: String, sendRaw: @escaping (String) -> Void) {
        self.sendRaw = sendRaw
        self.isAttached = true
        self.readyFired = false
        self.insideResponseBlock = false
        self.commandQueue.removeAll()
        self.pendingResponseLines.removeAll()
        self.rawBuffer = Data()
        sendRaw("tmux -CC attach-session -t \(sessionName)\n")
    }

    func sendCommand(_ command: String, completion: @escaping (String) -> Void) {
        commandQueue.append((id: nextCommandId, completion: completion))
        nextCommandId += 1
        sendRaw?("\(command)\n")
    }

    private static let outputPrefixBytes: [UInt8] = [0x25, 0x6F, 0x75, 0x74, 0x70, 0x75, 0x74, 0x20]

    func feedData(_ data: Data) {
        rawBuffer.append(data)
        while let nlIndex = rawBuffer.firstIndex(of: 0x0A) {
            var lineData = rawBuffer[rawBuffer.startIndex..<nlIndex]
            rawBuffer = Data(rawBuffer[(nlIndex + 1)...])
            if let last = lineData.last, last == 0x0D { lineData = lineData.dropLast() }
            // Fast-path for %output lines: process at byte level to avoid
            // lossy UTF-8 String conversion destroying raw multi-byte sequences.
            let lineBytes = [UInt8](lineData)
            if lineBytes.count > 8 && lineBytes.starts(with: Self.outputPrefixBytes) {
                parseOutputBytes(lineBytes)
            } else {
                let line = String(decoding: lineData, as: UTF8.self)
                parseLine(line)
            }
        }
    }

    private func parseLine(_ rawLine: String) {
        // Strip DSC/OSC junk that precedes control mode messages on the initial line.
        var line = rawLine
        if !line.hasPrefix("%") {
            for marker in ["%begin ", "%end ", "%error ", "%output ", "%session-changed ",
                           "%layout-change ", "%window-", "%pane-mode-changed ", "%exit"] {
                if let range = line.range(of: marker) {
                    line = String(line[range.lowerBound...])
                    break
                }
            }
        }
        if line.hasPrefix("%begin ") {
            insideResponseBlock = true
            pendingResponseLines = []
            return
        }
        if line.hasPrefix("%end ") {
            insideResponseBlock = false
            let response = pendingResponseLines.joined(separator: "\n")
            if let queued = commandQueue.first {
                commandQueue.removeFirst()
                queued.completion(response)
            }
            pendingResponseLines = []
            if !readyFired { readyFired = true; onReady?() }
            return
        }
        if line.hasPrefix("%error ") {
            insideResponseBlock = false
            if let queued = commandQueue.first {
                commandQueue.removeFirst()
                queued.completion("")
            }
            pendingResponseLines = []
            if !readyFired { readyFired = true; onReady?() }
            return
        }
        if insideResponseBlock {
            pendingResponseLines.append(line)
            return
        }
        if line.hasPrefix("%output ") {
            parseOutput(line)
            return
        }
        // Other notifications ignored for test
    }

    private func parseOutputBytes(_ bytes: [UInt8]) {
        let afterPrefix = bytes[8...]
        guard let spaceIdx = afterPrefix.firstIndex(of: 0x20) else { return }
        let paneId = String(decoding: afterPrefix[afterPrefix.startIndex..<spaceIdx], as: UTF8.self)
        let dataBytes = Array(afterPrefix[(spaceIdx + 1)...])
        let decoded = decodeOctalEscapedBytes(dataBytes)
        if !decoded.isEmpty { onPaneOutput?(paneId, decoded) }
    }

    private func parseOutput(_ line: String) {
        guard line.count > 8 else { return }
        let afterPrefix = line.dropFirst(8)
        guard let spaceIdx = afterPrefix.firstIndex(of: " ") else { return }
        let paneId = String(afterPrefix[afterPrefix.startIndex..<spaceIdx])
        let escapedData = String(afterPrefix[afterPrefix.index(after: spaceIdx)...])
        let decoded = decodeOctalEscaped(escapedData)
        if !decoded.isEmpty { onPaneOutput?(paneId, decoded) }
    }

    func parseListPanes(_ response: String) {
        // Format: pane_id:window_id:width:height:active:command:title (title last, may contain colons)
        for line in response.split(separator: "\n") {
            let parts = line.split(separator: ":", maxSplits: 6)
            guard parts.count >= 7 else { continue }
            let paneId = String(parts[0])
            panes[paneId] = PaneInfo(
                id: paneId, windowId: String(parts[1]), title: String(parts[6]),
                command: String(parts[5]),
                width: Int(parts[2]) ?? 80, height: Int(parts[3]) ?? 24,
                isActive: parts[4] == "1"
            )
        }
    }

    func parseListWindows(_ response: String) {
        // Format: window_id:name:index:active:layout (layout last, may contain colons)
        for line in response.split(separator: "\n") {
            let parts = line.split(separator: ":", maxSplits: 4)
            guard parts.count >= 5 else { continue }
            let windowId = String(parts[0])
            windows[windowId] = WindowInfo(
                id: windowId, name: String(parts[1]), layout: String(parts[4]),
                index: Int(parts[2]) ?? 0, isActive: parts[3] == "1"
            )
            if parts[3] == "1" { activeWindowId = windowId }
        }
    }

    private func decodeOctalEscapedBytes(_ input: [UInt8]) -> Data {
        var result = Data()
        result.reserveCapacity(input.count)
        var i = 0
        while i < input.count {
            let c = input[i]
            if c == 0x5C { // backslash
                let next = i + 1
                if next < input.count {
                    let nc = input[next]
                    if nc >= 0x30 && nc <= 0x33 { // '0'-'3'
                        var octalValue = Int(nc - 0x30)
                        var scanIdx = next + 1
                        var digits = 1
                        while digits < 3 && scanIdx < input.count {
                            let d = input[scanIdx]
                            if d >= 0x30 && d <= 0x37 { octalValue = octalValue * 8 + Int(d - 0x30); scanIdx += 1; digits += 1 } else { break }
                        }
                        result.append(UInt8(octalValue)); i = scanIdx; continue
                    }
                    switch nc {
                    case 0x5C: result.append(0x5C); i = next + 1; continue
                    case 0x6E: result.append(0x0A); i = next + 1; continue
                    case 0x72: result.append(0x0D); i = next + 1; continue
                    case 0x74: result.append(0x09); i = next + 1; continue
                    case 0x61: result.append(0x07); i = next + 1; continue
                    case 0x62: result.append(0x08); i = next + 1; continue
                    default: result.append(0x5C); result.append(nc); i = next + 1; continue
                    }
                } else { result.append(0x5C); i += 1 }
            } else { result.append(c); i += 1 }
        }
        return result
    }

    private func decodeOctalEscaped(_ input: String) -> Data {
        var result = Data()
        var index = input.startIndex
        while index < input.endIndex {
            let c = input[index]
            if c == "\\" {
                let next = input.index(after: index)
                if next < input.endIndex {
                    let nc = input[next]
                    if nc >= "0" && nc <= "3" {
                        var octal = String(nc)
                        var scanIdx = input.index(after: next)
                        while octal.count < 3 && scanIdx < input.endIndex {
                            let d = input[scanIdx]
                            if d >= "0" && d <= "7" { octal.append(d); scanIdx = input.index(after: scanIdx) } else { break }
                        }
                        if let byte = UInt8(octal, radix: 8) { result.append(byte); index = scanIdx; continue }
                    }
                    if nc == "\\" { result.append(UInt8(ascii: "\\")); index = input.index(after: next); continue }
                }
            }
            if let byte = String(c).data(using: .utf8) { result.append(byte) }
            index = input.index(after: index)
        }
        return result
    }
}

// ============================================================
// Test Cases
// ============================================================

var passed = 0
var failed = 0

func assert(_ condition: Bool, _ msg: String, file: String = #file, line: Int = #line) {
    if condition { passed += 1; print("  ✓ \(msg)") }
    else { failed += 1; print("  ✗ FAIL: \(msg) (line \(line))") }
}

// --- Test 1: Basic handshake ---
print("Test 1: Initial handshake")
do {
    let client = TmuxControlClientTest()
    var sentCommands: [String] = []
    var readyFired = false
    client.onReady = { readyFired = true }
    client.attach(sessionName: "test") { sentCommands.append($0) }

    assert(sentCommands.last == "tmux -CC attach-session -t test\n", "attach command sent")

    // Simulate tmux response: DSC + %begin/%end
    let response = "\u{1B}P1000p\r\n%begin 1234 5678 0\r\n%end 1234 5678 0\r\n"
    client.feedData(response.data(using: .utf8)!)

    assert(readyFired, "onReady fired after initial handshake")
}

// --- Test 2: list-panes parsing ---
print("\nTest 2: list-panes response parsing")
do {
    let client = TmuxControlClientTest()
    // Format: pane_id:window_id:width:height:active:command:title
    let response = "%0:@0:80:24:1:zsh:pane title\n%1:@0:120:40:0:vim:pane2\n%2:@1:80:24:0:htop:pane3"
    client.parseListPanes(response)

    assert(client.panes.count == 3, "3 panes parsed (got \(client.panes.count))")
    assert(client.panes["%0"]?.isActive == true, "pane %0 is active")
    assert(client.panes["%0"]?.command == "zsh", "pane %0 command is zsh")
    assert(client.panes["%1"]?.width == 120, "pane %1 width is 120")
    assert(client.panes["%1"]?.windowId == "@0", "pane %1 belongs to @0")
    assert(client.panes["%2"]?.command == "htop", "pane %2 command is htop")
}

// --- Test 3: list-windows parsing ---
print("\nTest 3: list-windows response parsing")
do {
    let client = TmuxControlClientTest()
    // Format: window_id:name:index:active:layout
    let response = "@0:server:1:1:abcd,80x24\n@1:dev:2:0:efgh,120x40"
    client.parseListWindows(response)

    assert(client.windows.count == 2, "2 windows parsed (got \(client.windows.count))")
    assert(client.windows["@0"]?.name == "server", "window @0 name is server")
    assert(client.windows["@0"]?.isActive == true, "window @0 is active")
    assert(client.windows["@1"]?.name == "dev", "window @1 name is dev")
    assert(client.activeWindowId == "@0", "active window is @0")
}

// --- Test 4: %output decoding ---
print("\nTest 4: %output octal decoding")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    // Hello\r\n with octal escapes
    let line = "%output %5 Hello\\015\\012"
    client.feedData((line + "\r\n").data(using: .utf8)!)

    assert(outputs.count == 1, "1 output received (got \(outputs.count))")
    if let first = outputs.first {
        assert(first.0 == "%5", "output for pane %5")
        let text = String(data: first.1, encoding: .utf8) ?? ""
        assert(text == "Hello\r\n", "decoded text is Hello\\r\\n (got \(text.debugDescription))")
    }
}

// --- Test 5: ESC sequence decoding ---
print("\nTest 5: ESC sequence in %output")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    // \033[0m = ESC [ 0 m
    let line = "%output %0 \\033[0mhello"
    client.feedData((line + "\n").data(using: .utf8)!)

    assert(outputs.count == 1, "1 output received")
    if let first = outputs.first {
        let bytes = [UInt8](first.1)
        assert(bytes[0] == 0x1B, "first byte is ESC (0x1B)")
        assert(String(data: first.1, encoding: .utf8)?.hasSuffix("hello") == true, "ends with hello")
    }
}

// --- Test 6: Command response with %begin/%end ---
print("\nTest 6: Command response via %begin/%end")
do {
    let client = TmuxControlClientTest()
    var sentCommands: [String] = []
    client.attach(sessionName: "s") { sentCommands.append($0) }

    // Initial handshake
    client.feedData("%begin 1 2 0\r\n%end 1 2 0\r\n".data(using: .utf8)!)

    // Send a command
    var response = ""
    client.sendCommand("list-panes -a -F '#{pane_id}'") { response = $0 }

    // Simulate response
    client.feedData("%begin 1 3 1\r\n%0\r\n%1\r\n%2\r\n%end 1 3 1\r\n".data(using: .utf8)!)

    assert(response == "%0\n%1\n%2", "got 3-line response (got \(response.debugDescription))")
}

// --- Test 7: Chunked data ---
print("\nTest 7: Data arriving in small chunks")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    let fullLine = "%output %10 hello world\\015\\012\r\n"
    let data = fullLine.data(using: .utf8)!
    // Feed byte by byte
    for i in 0..<data.count {
        client.feedData(Data([data[i]]))
    }

    assert(outputs.count == 1, "1 output from chunked feed (got \(outputs.count))")
}

// --- Test 8: Backslash escaping ---
print("\nTest 8: Backslash literal (\\134)")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    let line = "%output %0 path\\134file\r\n"
    client.feedData(line.data(using: .utf8)!)

    if let first = outputs.first {
        let text = String(data: first.1, encoding: .utf8) ?? ""
        assert(text == "path\\file", "backslash decoded (got \(text.debugDescription))")
    }
}

// --- Test 9: Mixed notifications and output ---
print("\nTest 9: Mixed protocol messages")
do {
    let client = TmuxControlClientTest()
    var outputCount = 0
    client.onPaneOutput = { _, _ in outputCount += 1 }
    client.attach(sessionName: "s") { _ in }

    let stream = """
    %begin 1 1 0\r
    %end 1 1 0\r
    %session-changed $0 main\r
    %output %0 hello\\015\\012\r
    %output %1 world\\015\\012\r
    %layout-change @0 abcd\r
    %output %0 test\\015\\012\r
    %window-renamed @0 newname\r

    """
    client.feedData(stream.data(using: .utf8)!)

    assert(outputCount == 3, "3 outputs received (got \(outputCount))")
}

// --- Test 10: DSC-embedded %begin on same line ---
print("\nTest 10: DSC/OSC prefix stripped to find %begin")
do {
    let client = TmuxControlClientTest()
    var readyFired = false
    client.onReady = { readyFired = true }
    client.attach(sessionName: "s") { _ in }

    // Simulate the real initial line where %begin is embedded after DSC/OSC junk
    let line = "]2;tmux -CC attach-session -t 0]1;tmux\u{1B}P1000p%begin 1775636889 3247338 0\r\n%end 1775636889 3247338 0\r\n"
    client.feedData(line.data(using: .utf8)!)

    assert(readyFired, "onReady fired with DSC-embedded %begin")
}

// --- Test 11: Colons in pane title ---
print("\nTest 11: Colons in pane title (title is last field)")
do {
    let client = TmuxControlClientTest()
    // Format: pane_id:window_id:width:height:active:command:title
    // Title "my:pane:title" contains colons
    let response = "%0:@0:80:24:1:zsh:my:pane:title"
    client.parseListPanes(response)

    assert(client.panes.count == 1, "1 pane parsed (got \(client.panes.count))")
    assert(client.panes["%0"]?.title == "my:pane:title", "title with colons preserved (got \(client.panes["%0"]?.title ?? "nil"))")
    assert(client.panes["%0"]?.command == "zsh", "command is zsh")
    assert(client.panes["%0"]?.width == 80, "width is 80")
}

// --- Test 12: Colons in window layout ---
print("\nTest 12: Colons in window layout (layout is last field)")
do {
    let client = TmuxControlClientTest()
    // Format: window_id:name:index:active:layout
    // Layout "7b3a,298x67,0,0{149x67,0,0,116,148x67,150,0,117}" contains colons-like chars
    let response = "@0:server:0:1:7b3a,298x67,0,0{149x67,0,0,116,148x67,150,0,117}"
    client.parseListWindows(response)

    assert(client.windows.count == 1, "1 window parsed (got \(client.windows.count))")
    assert(client.windows["@0"]?.layout == "7b3a,298x67,0,0{149x67,0,0,116,148x67,150,0,117}", "layout preserved")
    assert(client.windows["@0"]?.isActive == true, "window is active")
}

// --- Test 13: Fully octal-escaped emoji (4-byte UTF-8) ---
print("\nTest 13: Fully octal-escaped emoji (wrench U+1F527)")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    // 🔧 = U+1F527, UTF-8: F0 9F 94 A7, Octal: \360\237\224\247
    let line = "%output %0 \\360\\237\\224\\247\r\n"
    client.feedData(line.data(using: .utf8)!)

    assert(outputs.count == 1, "1 output received (got \(outputs.count))")
    if let first = outputs.first {
        let text = String(data: first.1, encoding: .utf8) ?? ""
        assert(text == "\u{1F527}", "decoded emoji is wrench (got \(text.debugDescription))")
    }
}

// --- Test 14: Partially octal-escaped emoji (some bytes escaped, some raw) ---
print("\nTest 14: Partially octal-escaped emoji (raw 0xF0 + escaped 0x9F + raw 0x94 0xA7)")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    // Simulate tmux escaping only 0x9F (C1 control) as \237, leaving F0, 94, A7 as raw bytes.
    // Build raw bytes: "%output %0 " + 0xF0 + "\237" + 0x94 + 0xA7 + "\r\n"
    var rawLine = Data()
    rawLine.append(contentsOf: [UInt8]("%output %0 ".utf8))
    rawLine.append(0xF0)
    rawLine.append(contentsOf: [UInt8]("\\237".utf8))  // octal escape for 0x9F
    rawLine.append(0x94)
    rawLine.append(0xA7)
    rawLine.append(contentsOf: [0x0D, 0x0A])  // \r\n
    client.feedData(rawLine)

    assert(outputs.count == 1, "1 output received (got \(outputs.count))")
    if let first = outputs.first {
        let bytes = [UInt8](first.1)
        assert(bytes == [0xF0, 0x9F, 0x94, 0xA7], "decoded bytes are F0 9F 94 A7 (got \(bytes.map { String(format: "%02X", $0) }.joined(separator: " ")))")
        let text = String(data: first.1, encoding: .utf8) ?? ""
        assert(text == "\u{1F527}", "decoded emoji is wrench (got \(text.debugDescription))")
    }
}

// --- Test 15: Chinese characters passed through as raw UTF-8 ---
print("\nTest 15: Chinese characters (raw UTF-8, not escaped)")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    // "你好" in UTF-8 — bytes are all >= 0x80, so tmux may or may not escape them
    // Test the case where they are NOT escaped (raw UTF-8 bytes in the line)
    var rawLine = Data()
    rawLine.append(contentsOf: [UInt8]("%output %0 ".utf8))
    rawLine.append(contentsOf: [UInt8]("你好".utf8))
    rawLine.append(contentsOf: [0x0D, 0x0A])
    client.feedData(rawLine)

    assert(outputs.count == 1, "1 output received (got \(outputs.count))")
    if let first = outputs.first {
        let text = String(data: first.1, encoding: .utf8) ?? ""
        assert(text == "你好", "decoded Chinese text (got \(text.debugDescription))")
    }
}

// --- Test 16: Box-drawing characters ---
print("\nTest 16: Box-drawing characters (U+2500 horizontal line)")
do {
    let client = TmuxControlClientTest()
    var outputs: [(String, Data)] = []
    client.onPaneOutput = { paneId, data in outputs.append((paneId, data)) }

    // ─ = U+2500, UTF-8: E2 94 80
    // tmux may escape all bytes or none; test fully escaped case
    let line = "%output %0 \\342\\224\\200\\342\\224\\200\\342\\224\\200\r\n"
    client.feedData(line.data(using: .utf8)!)

    assert(outputs.count == 1, "1 output received (got \(outputs.count))")
    if let first = outputs.first {
        let text = String(data: first.1, encoding: .utf8) ?? ""
        assert(text == "───", "decoded 3 horizontal box-drawing chars (got \(text.debugDescription))")
    }
}

// --- Summary ---
print("\n========================================")
print("Results: \(passed) passed, \(failed) failed")
if failed > 0 { exit(1) }
print("All tests passed!")
