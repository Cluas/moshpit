import Foundation
import Testing
@testable import Moshpit

// Every `-F` reply the controller reads used to be cut into fields on
// Characters, and a separator followed by a combining mark, a ZWJ or a
// variation selector is ONE Character together with it: a row whose name
// started like that came up a field short and was skipped. The rows below
// have the shape tmux 3.6a prints (captured on a private socket, -u, UTF-8).

// MARK: - Helpers

/// Names whose first scalar fuses with the separator in front of it — and,
/// next to them, the shapes that always parsed and must keep parsing.
private enum Name {
    static let combining = "\u{301}acute"      // COMBINING ACUTE ACCENT
    static let zwj = "\u{200D}zwj"             // ZERO WIDTH JOINER
    static let variation = "\u{FE0F}vs16"      // VARIATION SELECTOR-16
    static let spaced = "my project"
    static let cjk = "项目 一"
    static let emoji = "🧑‍💻 dev"
    static let all = [combining, zwj, variation, spaced, cjk, emoji]
}

/// Byte-for-byte equality: String's `==` is canonical equivalence.
private func same(_ actual: String?, _ expected: String) -> Bool {
    actual.map { Array($0.utf8) == Array(expected.utf8) } ?? false
}

private func bytes(_ fields: [String]) -> [[UInt8]] { fields.map { Array($0.utf8) } }

@MainActor
private func waitUntil(
    timeout: TimeInterval = 1.0,
    _ predicate: @MainActor () async -> Bool
) async -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if await predicate() { return true }
        try? await Task.sleep(for: .milliseconds(5))
    }
    return await predicate()
}

/// An attached controller on a mock transport whose boot block is answered;
/// its three discovery commands are on the wire, unanswered.
@MainActor
private func makeAttachedController(
    rendersOutput: Bool = true
) async -> (TmuxSessionController, MockTmuxTransport) {
    let transport = MockTmuxTransport()
    let controller = TmuxSessionController(sshSession: transport, rendersOutput: rendersOutput)
    await controller.attach()
    transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
    _ = await waitUntil { await transport.recordedCommands().count >= 3 }
    return (controller, transport)
}

private func block(_ num: Int, _ lines: [String]) -> String {
    lines.isEmpty
        ? "%begin \(num) \(num) 1\n%end \(num) \(num) 1\n\n"
        : "%begin \(num) \(num) 1\n\(lines.joined(separator: "\n"))\n%end \(num) \(num) 1\n\n"
}

/// A wait on something that does happen, just late on a loaded device.
private let loadedDeviceTimeout: TimeInterval = 15

// MARK: - The cut itself

@Suite("tmux format fields are cut on bytes")
struct TmuxFormatFieldsTests {

    @Test("a field that starts with a joining mark keeps the separator in front of it",
          arguments: [Name.combining, Name.zwj, Name.variation,
                      "\u{93E}mark",       // DEVANAGARI VOWEL SIGN AA, a spacing mark
                      "\u{1F3FD}tone"])    // EMOJI MODIFIER FITZPATRICK TYPE-4
    func joinerAfterSeparator(_ name: String) {
        #expect(bytes("$1 0 \(name)".tmuxFields(separator: " ", maxSplits: 2))
                == bytes(["$1", "0", name]))
        #expect(bytes("%1|working|\(name)|1758000000|\(name)|\(name)"
            .tmuxFields(separator: "|", maxSplits: 5, omittingEmptySubsequences: false))
                == bytes(["%1", "working", name, "1758000000", name, name]))
    }

    @Test("a field that ends in a prepend mark keeps the separator after it")
    func prependBeforeSeparator() {
        // U+0600 ARABIC NUMBER SIGN fuses with whatever FOLLOWS it.
        #expect(bytes("%1|working|claude\u{600}|1758000000|node|t"
            .tmuxFields(separator: "|", maxSplits: 5, omittingEmptySubsequences: false))
                == bytes(["%1", "working", "claude\u{600}", "1758000000", "node", "t"]))
    }

    @Test("CJK, emoji and spaces in the last field come through whole")
    func restOfLine() {
        for name in Name.all {
            #expect(bytes("$0 @1 2 b25d,80x24,0,0,1 1 1 \(name)".tmuxFields(separator: " ", maxSplits: 6))
                    == bytes(["$0", "@1", "2", "b25d,80x24,0,0,1", "1", "1", name]))
        }
        #expect("%5|working|claude|1|node|a | b|c"
            .tmuxFields(separator: "|", maxSplits: 5, omittingEmptySubsequences: false).last == "a | b|c")
    }

    @Test("on ASCII the fields are exactly split(separator:)'s, for every maxSplits and omitting")
    func asciiParity() {
        let lines = ["$0 1 main", "$0 1 my  project", "$0 1  lead", "$0 1 trail ", "$0 1 ", " a b ",
                     "a  b", "", " ", "|", "%5||||", "%5|a|b|c|d|e|f|g", "c|l|1700000000", "|x|", "a||b|"]
        for line in lines {
            for separator: Unicode.Scalar in [" ", "|", ",", "x"] {
                for maxSplits in [0, 1, 2, 5, 6, 8, Int.max] {
                    for omitting in [true, false] {
                        let expected = line.split(separator: Character(separator), maxSplits: maxSplits,
                                                  omittingEmptySubsequences: omitting).map(String.init)
                        #expect(line.tmuxFields(separator: separator, maxSplits: maxSplits,
                                                omittingEmptySubsequences: omitting) == expected,
                                "\(line.debugDescription) on \(separator), maxSplits \(maxSplits), omitting \(omitting)")
                    }
                }
            }
        }
    }
}

// MARK: - The controller's replies

@Suite("tmux replies keep rows whose names start with a joining mark", .serialized)
@MainActor
struct TmuxJoiningMarkRowsTests {

    @Test("discovery keeps every session, window and pane, and lands on the active ones")
    func discoveryKeepsEveryRow() async throws {
        let (controller, transport) = await makeAttachedController()
        // One session per name, each with one window and one pane of that
        // name; $0 — the combining-mark one — is attached, so the attach must
        // land on its window and pane.
        let names = Name.all.enumerated()
        transport.pushText(
            block(1, names.map { i, name in "$\(i) \(i == 0 ? 1 : 0) \(name)" })
            + block(2, names.map { i, name in "$\(i) @\(i) 0 b25d,80x24,0,0,\(i) 1 1 \(name)" })
            + block(3, names.map { i, name in "%\(i) @\(i) 0 80 24 1 0 0 \(name)" }))

        let listed = await waitUntil(timeout: loadedDeviceTimeout) {
            controller.snapshot.panes.count == Name.all.count
        }
        #expect(listed, "every pane is listed")
        #expect(controller.snapshot.sessions.count == Name.all.count)
        #expect(controller.snapshot.windows.count == Name.all.count)
        #expect(controller.snapshot.panes.count == Name.all.count)
        for (i, name) in Name.all.enumerated() {
            #expect(same(controller.snapshot.sessions["$\(i)"]?.name, name))
            #expect(same(controller.snapshot.windows["@\(i)"]?.name, name))
            #expect(same(controller.snapshot.panes["%\(i)"]?.command, name))
        }
        #expect(controller.snapshot.activeSessionId == "$0")
        #expect(controller.snapshot.activeWindowId == "@0")
        #expect(controller.snapshot.activePaneId == "%0")
    }

    @Test("the agent-stamp poll keeps a pane whose agent, command or title starts with a joining mark")
    func agentHooksKeepEveryPane() async throws {
        let (controller, transport) = await makeAttachedController()
        // Empty discovery: no panes, so no repaint chain joins the FIFO.
        transport.pushText(block(1, []) + block(2, []) + block(3, []))
        controller.pollAgentHooks()
        _ = await waitUntil { await transport.recordedCommands().count >= 4 }
        transport.pushText(block(9, [
            "%1|working|\(Name.combining)|1758000000|node|title",
            "%2|attention|claude|1758000000|\(Name.zwj)|title",
            "%3|done|claude|1758000000|node|\(Name.variation)",
            "%4|working|claude\u{600}|1758000000|node|a|b",
        ]))

        #expect(await waitUntil(timeout: loadedDeviceTimeout) { controller.agentHooksLoaded })
        #expect(controller.agentHooks.count == 4)
        #expect(controller.agentHooks["%1"]?.state == "working")
        #expect(same(controller.agentHooks["%1"]?.agent, Name.combining))
        #expect(same(controller.agentHooks["%2"]?.command, Name.zwj))
        #expect(same(controller.agentHooks["%3"]?.title, Name.variation))
        #expect(same(controller.agentHooks["%4"]?.agent, "claude\u{600}"))
        #expect(same(controller.agentHooks["%4"]?.title, "a|b"), "a `|` in the title stays in it")
    }

    /// The mosh sidecar moves the renderer along by matching `list-clients`
    /// rows against the session it leaves. Character-split, the renderer's
    /// row was skipped — and the session itself was missing from discovery,
    /// so the sidecar fell back to the desktop's session and moved the
    /// DESKTOP's client instead. (The switch now moves the one client it
    /// knows is the renderer — see TmuxSidecarSwitchTests.)
    @Test("a sidecar session switch moves the renderer off a joiner-named session, and only it")
    func sidecarSwitchMovesTheRenderer() async throws {
        let (controller, transport) = await makeAttachedController(rendersOutput: false)
        transport.pushText(block(1, ["$2 1 work", "$1 0 main", "$0 1 \(Name.combining)"])
                           + block(2, []) + block(3, []))
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            controller.snapshot.sessions["$0"]?.name == Name.combining
        }, "the joiner-named session is listed")

        // Answer every command in FIFO order — a client listing with the
        // rows below (`TmuxClientRow.format`), the rest empty.
        var clients = ["1|4242|$0|client-4242", "0|3009|$2|/dev/ttys009"]
        var answered = 3
        func answer(until done: () async -> Bool) async {
            let deadline = Date().addingTimeInterval(loadedDeviceTimeout)
            while Date() < deadline {
                let commands = await transport.recordedCommands()
                while answered < commands.count {
                    let num = 700 + answered
                    transport.pushText(commands[answered].hasPrefix("list-clients")
                        ? block(num, clients) : block(num, []))
                    answered += 1
                }
                if await done() { return }
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        // The renderer attaches, and the hub names it by the pid its
        // program recorded.
        clients.insert("0|3003|$0|/dev/ttys003", at: 1)
        let adopted = Task { await controller.adoptRenderer(pid: 3003) }
        await answer { controller.rendererClient != nil }
        #expect(await adopted.value)
        #expect(controller.snapshot.activeSessionId == "$0", "the sidecar follows the renderer's session")

        controller.selectSession("$1")
        await answer { await transport.recordedCommands().contains { $0.hasPrefix("switch-client -t $1") } }

        let commands = await transport.recordedCommands()
        #expect(commands.contains { $0.hasPrefix("switch-client -t $1") }, "the sidecar itself switches")
        #expect(commands.contains { $0.hasPrefix("switch-client -c /dev/ttys003 -t $1") },
                "the renderer on the joiner-named session comes along")
        #expect(!commands.contains { $0.contains("/dev/ttys009") },
                "the desktop on another session is left alone")
    }

    @Test("panePath keeps a cwd whose first component starts with a joining mark")
    func panePathJoiner() async throws {
        let (controller, transport) = await makeAttachedController()
        // Empty discovery: nothing else joins the FIFO.
        transport.pushText(block(1, []) + block(2, []) + block(3, []))
        let replies = ["/\(Name.combining)/proj", "/\(Name.zwj)", "/\(Name.variation) dir",
                       "/home/u/project", "relative/path"]
        for (i, reply) in replies.enumerated() {
            let asked = Task { await controller.panePath("%0") }
            _ = await waitUntil(timeout: loadedDeviceTimeout) {
                await transport.recordedCommands().filter { $0.contains("#{pane_current_path}") }.count == i + 1
            }
            transport.pushText(block(10 + i, [reply]))
            let path = await asked.value
            if reply.hasPrefix("relative") {
                #expect(path == nil, "not absolute")
            } else {
                #expect(same(path, reply), "\(reply.debugDescription)")
            }
        }
    }

    /// tmux 3.6a prints `#{pane_current_path}` verbatim: a pane in
    /// `proj ` answers `…/proj \r\n` (private socket), and the space is the
    /// directory's own. Trimmed, the agent's relative paths and the files key
    /// went to `proj` — another directory, or none.
    @Test("panePath keeps the spaces and tabs that are part of a directory's name")
    func panePathKeepsSpaces() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText(block(1, []) + block(2, []) + block(3, []))
        let replies = ["/home/u/proj ", "/home/u/proj  ", "/home/u/ lead", "/tmp/ ", "/home/u/tab\t",
                       "/home/u/plain"]
        for (i, reply) in replies.enumerated() {
            let asked = Task { await controller.panePath("%0") }
            _ = await waitUntil(timeout: loadedDeviceTimeout) {
                await transport.recordedCommands().filter { $0.contains("#{pane_current_path}") }.count == i + 1
            }
            // Every other reply framed the way a pty hands it over, CRLF.
            let framed = block(10 + i, [reply])
            transport.pushText(i.isMultiple(of: 2) ? framed.replacingOccurrences(of: "\n", with: "\r\n") : framed)
            let path = await asked.value
            #expect(same(path, reply), "\(reply.debugDescription)")
        }
    }

    @Test("a size lease whose label starts with a joining mark still parses")
    func sizeLeaseLabel() {
        let lease = TmuxSessionController.SizeLease(
            parsing: "/dev/ttys001|\(Name.combining)|1758000000", clients: [])
        #expect(lease?.clientName == "/dev/ttys001")
        #expect(same(lease?.label, Name.combining))
    }

    @Test("the layout gate takes ASCII digits only")
    func layoutGateIsASCII() {
        let isPlausible = TmuxSessionController.isPlausibleLayout
        #expect(isPlausible("21be,200x50,0,0"))
        #expect(!isPlausible("21be,２００x50,0,0"), "fullwidth digits")
        #expect(!isPlausible("21be,٢٠٠x50,0,0"), "Arabic-Indic digits")
        #expect(!isPlausible("２１ｂｅ,200x50,0,0"), "a fullwidth checksum")
        #expect(!isPlausible("21be,2\u{301}00x50,0,0"), "a digit with a combining mark")
        #expect(TmuxSessionController.windowWidth(fromLayout: "21be,200x50,0,0") == 200)
    }
}

// MARK: - Empty last fields

/// A window's name and a pane's command are the one field of their row that
/// can be empty, and tmux then ends the row on its separator. Rows captured
/// from tmux 3.6a on a private socket: `rename-window ''`, and a pane running
/// a binary named `--` (tmux reads its command as "").
@Suite("tmux replies keep rows whose last field is empty", .serialized)
@MainActor
struct TmuxEmptyLastFieldTests {

    @Test("a window renamed to '' keeps its row, its panes and their terminals; so does a pane with no command")
    func emptyNameAndCommand() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText(
            block(1, ["$0 1 main"])
            + block(2, ["$0 @0 0 b25d,80x24,0,0,0 0 2 zsh",
                        "$0 @11 4 5960,80x24,0,0,11 1 1 "])
            + block(3, ["%0 @0 0 80 11 1 0 0 zsh",
                        "%3 @0 1 80 11 0 0 0 ",
                        "%11 @11 0 80 24 1 0 0 zsh"]))

        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            controller.snapshot.panes.count == 3
        }, "every pane is listed")
        let window = try #require(controller.snapshot.windows["@11"], "the window renamed to ''")
        #expect(window.name == "")
        #expect(window.index == 4)
        #expect(window.isActive)
        #expect(controller.snapshot.activeWindowId == "@11", "and as tmux's active window, it is where the attach lands")
        #expect(controller.snapshot.activePaneId == "%11")
        #expect(controller.coordinator(for: "%11") != nil, "its pane got a terminal")
        let commandless = try #require(controller.snapshot.panes["%3"], "the pane whose command reads ''")
        #expect(commandless.command == "")
        #expect(commandless.windowId == "@0")
        #expect(controller.coordinator(for: "%3") != nil)
    }

    @Test("well-formed rows parse exactly as before: spaces inside, before and after a name")
    func wellFormedRows() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText(
            block(1, ["$0 1 main"])
            + block(2, ["$0 @0 0 b25d,80x24,0,0,0 1 1 two words",
                        "$0 @1 1 b25e,80x24,0,0,1 0 1  lead",
                        "$0 @2 2 b25f,80x24,0,0,2 0 1 trail "])
            + block(3, ["%0 @0 0 80 24 1 1 0 node index.js",
                        "%1 @1 0 80 24 1 0 1 vim"]))

        #expect(await waitUntil(timeout: loadedDeviceTimeout) { controller.snapshot.panes.count == 2 })
        #expect(controller.snapshot.windows["@0"]?.name == "two words")
        #expect(controller.snapshot.windows["@1"]?.name == " lead")
        #expect(controller.snapshot.windows["@2"]?.name == "trail ")
        #expect(controller.snapshot.panes["%0"]?.command == "node index.js")
        #expect(controller.snapshot.activeWindowId == "@0")
        #expect(controller.snapshot.activePaneId == "%0")
    }
}
