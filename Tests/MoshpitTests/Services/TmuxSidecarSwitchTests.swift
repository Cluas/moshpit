import Foundation
import Testing
@testable import Moshpit

// The mosh sidecar's session switch: the -CC sidecar is invisible, so moving
// itself is not enough — the screen the person sees is the mosh-rendered tmux
// client, which has to come along, while every other client on the server
// (the desktop's terminal, first of all) stays exactly where it is.
//
// The rows below have the shape tmux 3.6a prints, captured on a private
// socket (`tmux -L <name> -f /dev/null`) with a pty-attached client on a
// session named `a|b`:
//
//     list-clients -F '#{client_control_mode}|#{client_pid}|#{session_id}|#{client_name}'
//         0|45533|$1|/dev/ttys026
//         1|45613|$0|client-45613
//     list-clients -F '#{client_control_mode}|#{client_session}|#{client_name}'
//         0|a|b|/dev/ttys026
//         1|main|client-45613
//     rename-session -t '=a|b' 'x|y'   →   %session-renamed $1 x|y
//
// Each fake client below renders whatever `-F` format the controller asks
// for, so the same scenario runs against any version of the command.

// MARK: - A fake tmux client list

private struct FakeClient {
    var control: Bool
    var pid: Int
    var sessionId: String
    var sessionName: String
    var name: String

    /// `format` expanded the way tmux would for this client.
    func render(_ format: String) -> String {
        var out = format
        let values = [
            "#{client_control_mode}": control ? "1" : "0",
            "#{client_pid}": String(pid),
            "#{session_id}": sessionId,
            "#{client_session}": sessionName,
            "#{session_name}": sessionName,
            "#{client_name}": name,
        ]
        for (key, value) in values { out = out.replacingOccurrences(of: key, with: value) }
        return out
    }
}

/// The `-F` argument of a `list-clients` command, as sent.
private func listClientsFormat(_ command: String) -> String? {
    guard command.hasPrefix("list-clients -F '"),
          let open = command.firstIndex(of: "'") else { return nil }
    let rest = command[command.index(after: open)...]
    guard let close = rest.firstIndex(of: "'") else { return nil }
    return String(rest[..<close])
}

private func block(_ num: Int, _ lines: [String]) -> String {
    lines.isEmpty
        ? "%begin \(num) \(num) 1\n%end \(num) \(num) 1\n\n"
        : "%begin \(num) \(num) 1\n\(lines.joined(separator: "\n"))\n%end \(num) \(num) 1\n\n"
}

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

/// A wait on something that does happen, just late on a loaded device.
private let loadedDeviceTimeout: TimeInterval = 15

@MainActor
private final class Flag {
    var isSet = false
}

/// An attached mosh sidecar (`rendersOutput: false`) on a mock tmux that
/// answers every command in FIFO order: `list-clients` from ``clients``,
/// `list-sessions` from ``sessions``, `list-windows` from ``windows``, the
/// pane discovery from ``panes``, `new-session` with ``newSessionId``,
/// anything else with an empty reply.
@MainActor
private final class SidecarRig {
    let controller: TmuxSessionController
    let transport: MockTmuxTransport
    var clients: [FakeClient]
    /// `list-sessions` rows, in tmux's own order (by name).
    var sessions: [String]
    /// `list-windows -a` rows.
    var windows: [String]
    /// `list-panes -a` discovery rows.
    var panes: [String]
    var newSessionId = "$5"
    /// `list-windows -a` rows in ``TmuxSessionController/sizeProbeFormat``.
    var sizeRows: [String] = []
    /// Answers taken ahead of every other: rows, or nil to answer as usual.
    var custom: ((String, [FakeClient]) -> [String]?)?
    private var answered = 0

    init(_ controller: TmuxSessionController, _ transport: MockTmuxTransport,
         sessions: [String], windows: [String], panes: [String], clients: [FakeClient]) {
        self.controller = controller
        self.transport = transport
        self.sessions = sessions
        self.windows = windows
        self.panes = panes
        self.clients = clients
    }

    /// A sidecar attached and through discovery. No `%session-changed` yet:
    /// it knows the sessions, and none of them as its own.
    static func make(sessions: [String], windows: [String] = [], panes: [String] = [],
                     clients: [FakeClient],
                     ledger: TmuxSessionController.PinLedger? = nil) async -> SidecarRig {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport, rendersOutput: false)
        if let ledger { controller.inheritPinLedger(ledger) }
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        let rig = SidecarRig(controller, transport, sessions: sessions, windows: windows,
                             panes: panes, clients: clients)
        _ = await waitUntil(timeout: loadedDeviceTimeout) {
            await transport.recordedCommands().count >= 3
        }
        await rig.answer { controller.snapshot.sessions.count == sessions.count }
        await rig.settle()
        return rig
    }

    private func reply(to command: String) -> [String] {
        if let custom, let rows = custom(command, clients) { return rows }
        if let format = listClientsFormat(command) { return clients.map { $0.render(format) } }
        if command.hasPrefix("switch-client -c ") {
            // tmux moves that client when it gets here in the queue — so
            // listings answered before it still show it where it was.
            let words = command.trimmingCharacters(in: .newlines).split(separator: " ").map(String.init)
            if words.count == 5, words[3] == "-t",
               let row = sessions.first(where: { $0.hasPrefix(words[4] + " ") }) {
                let name = row.split(separator: " ", maxSplits: 2).dropFirst(2).first.map(String.init) ?? ""
                clients = clients.map {
                    var client = $0
                    if client.name == words[2] { client.sessionId = words[4]; client.sessionName = name }
                    return client
                }
            }
            return []
        }
        if command.hasPrefix("list-sessions") { return sessions }
        if command.hasPrefix("list-windows -a -F '\(TmuxSessionController.sizeProbeFormat)'") { return sizeRows }
        if command.hasPrefix("list-windows") { return windows }
        if command.hasPrefix("list-panes -a -F \(TmuxSessionController.paneDiscoveryFormat)") { return panes }
        if command.hasPrefix("new-session") { return [newSessionId] }
        return []
    }

    /// Answer until nothing new has gone out for two looks in a row.
    func settle() async {
        var seen = -1
        var quiet = 0
        let deadline = Date().addingTimeInterval(loadedDeviceTimeout)
        while Date() < deadline, quiet < 2 {
            await answerPending()
            try? await Task.sleep(for: .milliseconds(40))
            let count = await transport.recordedCommands().count
            quiet = (count == seen && count == answered) ? quiet + 1 : 0
            seen = count
        }
    }

    /// Answer every command sent so far, in send order.
    func answerPending() async {
        let commands = await transport.recordedCommands()
        while answered < commands.count {
            transport.pushText(block(700 + answered, reply(to: commands[answered])))
            answered += 1
        }
    }

    /// Keep answering until `done` holds (or the deadline passes).
    @discardableResult
    func answer(until done: @MainActor () async -> Bool) async -> Bool {
        let deadline = Date().addingTimeInterval(loadedDeviceTimeout)
        while Date() < deadline {
            await answerPending()
            if await done() { return true }
            try? await Task.sleep(for: .milliseconds(10))
        }
        await answerPending()
        return await done()
    }

    /// One `adoptRenderer(pid:)` round trip — what the hub runs once the
    /// renderer mosh-server started has attached, with the pid its program
    /// recorded.
    @discardableResult
    func adopt(pid: Int) async -> Bool {
        let controller = self.controller
        let adopted = Task { await controller.adoptRenderer(pid: pid) }
        let done = Flag()
        Task { _ = await adopted.value; done.isSet = true }
        await answer { done.isSet }
        return done.isSet ? await adopted.value : false
    }

    /// `client` attaches as the phone's renderer, and the hub names it by
    /// its pid.
    func attachRenderer(_ client: FakeClient) async {
        clients.append(client)
        await adopt(pid: client.pid)
    }

    /// Switch to `sessionId` and answer until the sidecar has moved itself
    /// (the last thing the switch sends); returns every command sent.
    func switchSession(to sessionId: String) async -> [String] {
        controller.selectSession(sessionId)
        await answer {
            await self.transport.recordedCommands().contains { $0.hasPrefix("switch-client -t \(sessionId)") }
        }
        return await transport.recordedCommands()
    }

    /// What tmux tells this client when it lands on a session — its attach,
    /// and every switch after that — answered until the re-listing the
    /// controller sends for it has gone out and been answered.
    func land(on sessionId: String, named name: String) async {
        func relists() async -> Int {
            await transport.recordedCommands().filter { $0.hasPrefix("list-sessions") }.count
        }
        let before = await relists()
        transport.pushText("%session-changed \(sessionId) \(name)\n")
        await answer { await relists() > before }
        await settle()
    }

    /// Every `set-option` that touches a status line, in send order (the
    /// stamps beside them aside).
    func statusCommands() async -> [String] {
        await transport.recordedCommands().filter {
            $0.hasPrefix("set-option") && $0.contains("status") && !$0.contains("@moshpit_status_hidden")
        }
    }

    /// The `@moshpit_*` session marks set and unset (B86).
    func markCommands() async -> [String] {
        await transport.recordedCommands().filter {
            $0.hasPrefix("set-option") && ($0.contains("@moshpit_owned") || $0.contains("@moshpit_status_hidden"))
        }
    }

    /// A sidecar as the session starts one: tmux's client already running,
    /// its first `%session-changed` the attach. `marks` answers the owned
    /// sessions' listing (`$5 owned=1 hidden=…`).
    static func booting(sessions: [String], clients: [FakeClient], marks: [String],
                        landing: (id: String, name: String)) async -> SidecarRig {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport, rendersOutput: false)
        await controller.beginControlMode()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        let rig = SidecarRig(controller, transport, sessions: sessions, windows: [], panes: [],
                             clients: clients)
        rig.custom = { command, _ in command.contains(" owned=#{@moshpit_owned}") ? marks : nil }
        transport.pushText("%session-changed \(landing.id) \(landing.name)\n")
        await rig.answer { controller.snapshot.sessions.count == sessions.count }
        await rig.settle()
        return rig
    }
}

// MARK: - Clients

private let sidecar = FakeClient(control: true, pid: 45613, sessionId: "$1", sessionName: "a|b",
                                 name: "client-45613")
private let renderer = FakeClient(control: false, pid: 45533, sessionId: "$1", sessionName: "a|b",
                                  name: "/dev/ttys026")
private let desktop = FakeClient(control: false, pid: 41001, sessionId: "$0", sessionName: "alpha",
                                 name: "/dev/ttys009")

// MARK: - Session ids, not names

@Suite("the mosh sidecar's session switch matches sessions by id", .serialized)
@MainActor
struct TmuxSidecarSessionIdTests {

    @Test("a renderer on a session whose name holds a `|` comes along, and no garbled client is sent")
    func pipeInSessionName() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 alpha", "$1 2 a|b", "$2 0 main"],
                                        clients: [desktop, sidecar])
        await rig.attachRenderer(renderer)
        await rig.settle()
        #expect(rig.controller.snapshot.activeSessionId == "$1", "the sidecar is where the renderer is")

        let commands = await rig.switchSession(to: "$2")
        #expect(commands.contains { $0.hasPrefix("switch-client -c /dev/ttys026 -t $2") },
                "the renderer on `a|b` comes along")
        #expect(!commands.contains { $0.contains("b|/dev/") }, "no client name cut out of the session name")
        #expect(!commands.contains { $0.contains("/dev/ttys009") }, "the desktop on another session stays")
    }

    @Test("a rename from the desktop reaches the snapshot, and the switch still finds the renderer")
    func renameFromTheDesktop() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 alpha", "$1 2 a|b", "$2 0 main"],
                                        clients: [desktop, sidecar])
        await rig.attachRenderer(renderer)

        // What tmux 3.6a sends every control client on `rename-session`.
        rig.transport.pushText("%session-renamed $1 x|y\n")
        rig.clients = rig.clients.map {
            var client = $0
            if client.sessionId == "$1" { client.sessionName = "x|y" }
            return client
        }
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.sessions["$1"]?.name == "x|y"
        }, "the breadcrumb, the sheets and the hub's attach-by-name see the new name")

        let commands = await rig.switchSession(to: "$2")
        #expect(commands.contains { $0.hasPrefix("switch-client -c /dev/ttys026 -t $2") },
                "the renderer on the renamed session comes along")
        #expect(!commands.contains { $0.contains("/dev/ttys009") })
    }

    @Test("a session Moshpit created gets its status bar back by id, whatever it is called by then")
    func ownedSessionRestoredById() async throws {
        let rig = await SidecarRig.make(sessions: ["$1 2 a|b"], clients: [sidecar, renderer])
        rig.sessions.append("$5 0 mine")
        rig.controller.newSession(named: "mine")
        await rig.answer {
            await rig.statusCommands().contains { $0.contains("$5") && $0.contains("status off") }
        }
        // Renamed from the desktop: the name it was created with now names
        // nothing — or, through tmux's prefix match, someone else's session.
        rig.transport.pushText("%session-renamed $5 renamed\n")

        let restored = Flag()
        Task { @MainActor in
            await rig.controller.restoreSuppressedStatusAndFlush()
            restored.isSet = true
        }
        #expect(await rig.answer { restored.isSet })
        let commands = await rig.transport.recordedCommands()
        #expect(commands.contains { $0.hasPrefix("set-option -t '$5' -u status") },
                "the restore targets the session it styled")
        #expect(!commands.contains { $0.contains("-t 'mine'") },
                "never the name it was created with: `-t` falls back to a prefix match")
    }
}

// MARK: - Only the phone's own renderer

// Sessions without a `|` from here on: the old name-based filter read them
// correctly, so what these show is which clients it chose.
private let workSidecar = FakeClient(control: true, pid: 45613, sessionId: "$1", sessionName: "work",
                                     name: "/dev/ttys030")
private let workRenderer = FakeClient(control: false, pid: 45533, sessionId: "$1", sessionName: "work",
                                      name: "/dev/ttys026")
private let workDesktop = FakeClient(control: false, pid: 41001, sessionId: "$1", sessionName: "work",
                                     name: "/dev/ttys009")
private let alphaDesktop = FakeClient(control: false, pid: 41002, sessionId: "$0", sessionName: "alpha",
                                      name: "/dev/ttys010")

@Suite("the mosh sidecar's session switch moves the phone's own renderer and nothing else", .serialized)
@MainActor
struct TmuxSidecarRendererTests {

    @Test("a desktop on the renderer's own session stays where it is")
    func desktopOnTheSameSession() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 3 work", "$2 0 main"],
                                        clients: [workDesktop, workSidecar])
        await rig.attachRenderer(workRenderer)
        await rig.settle()
        #expect(rig.controller.snapshot.activeSessionId == "$1")

        let commands = await rig.switchSession(to: "$2")
        #expect(commands.filter { $0.hasPrefix("switch-client -c ") }
                == ["switch-client -c /dev/ttys026 -t $2\n"],
                "the renderer comes along; the desktop on the same session does not")
    }

    @Test("with no session known yet, the renderer comes along and no desktop does")
    func noSessionKnown() async throws {
        // Nothing attached anywhere as far as the listing goes: the sidecar
        // has no session to call its own.
        let rig = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 0 work", "$2 0 main"],
                                        clients: [alphaDesktop, workDesktop, workSidecar])
        #expect(rig.controller.snapshot.activeSessionId == nil)
        await rig.attachRenderer(workRenderer)

        let commands = await rig.switchSession(to: "$2")
        #expect(commands.filter { $0.hasPrefix("switch-client -c ") }
                == ["switch-client -c /dev/ttys026 -t $2\n"])
        #expect(!commands.contains { $0.contains("/dev/ttys009") || $0.contains("/dev/ttys010") },
                "no desktop moves, on this session or any other")
    }

    @Test("a renderer never identified: no client moves, the sidecar still switches itself")
    func rendererUnknown() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 alpha", "$1 3 work", "$2 0 main"],
                                        clients: [alphaDesktop, workDesktop, workSidecar, workRenderer])
        let commands = await rig.switchSession(to: "$2")
        #expect(!commands.contains { $0.hasPrefix("switch-client -c ") },
                "two terminals on the session and nothing to tell which is the phone's: move neither")
        #expect(commands.contains { $0.hasPrefix("switch-client -t $2") })
    }

    @Test("a client that got the renderer's tty after it exited is not the renderer")
    func ttyTakenOver() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 2 work", "$2 0 main"],
                                        clients: [workSidecar])
        await rig.attachRenderer(workRenderer)
        // The renderer is gone; a desktop terminal opened the same tty.
        var successor = workRenderer
        successor.pid = 50001
        rig.clients = [workSidecar, successor]

        let commands = await rig.switchSession(to: "$2")
        #expect(!commands.contains { $0.hasPrefix("switch-client -c ") })
    }

    @Test("two clients attaching at once, neither adopted by its pid: neither is named")
    func twoAtOnce() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 3 work", "$2 0 main"],
                                        clients: [workSidecar])
        rig.clients += [workRenderer, workDesktop]
        await rig.settle()

        let commands = await rig.switchSession(to: "$2")
        #expect(!commands.contains { $0.hasPrefix("switch-client -c ") })
    }

    /// B84: the renderer used to be the one client that appeared between a
    /// count before and after its attach. Its attach failed, a desktop
    /// attached in the same moment — and the desktop was taken for it: the
    /// next session switch moved the desktop to the phone's session.
    @Test("a desktop attaching while the renderer's attach fails is never the renderer")
    func desktopAttachingBesideAFailedRenderer() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 2 work", "$2 0 main"],
                                        clients: [workSidecar])
        rig.clients.append(workDesktop)   // the renderer's attach failed; a desktop attached
        #expect(await rig.adopt(pid: workRenderer.pid) == false, "no client has the renderer's pid")
        #expect(rig.controller.rendererClient == nil,
                "the desktop that appeared between the counts was taken for the renderer")

        let commands = await rig.switchSession(to: "$2")
        #expect(!commands.contains { $0.hasPrefix("switch-client -c ") },
                "the switch moved the desktop: \(commands.filter { $0.hasPrefix("switch-client") })")
    }

    /// The hub rebuilds the sidecar when iOS killed its SSH and mosh kept
    /// the renderer alive: the new sidecar never sees an attach.
    @Test("a sidecar rebuilt on the same mosh connection still knows the renderer")
    func rebuiltSidecar() async throws {
        let first = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 3 work", "$2 0 main"],
                                          clients: [workDesktop, workSidecar])
        await first.attachRenderer(workRenderer)
        let ledger = first.controller.takePinLedger()

        var rebuilt = workSidecar
        rebuilt.pid = 45999
        rebuilt.name = "/dev/ttys031"
        let second = await SidecarRig.make(sessions: ["$0 0 alpha", "$1 3 work", "$2 0 main"],
                                           clients: [workDesktop, rebuilt, workRenderer], ledger: ledger)
        let commands = await second.switchSession(to: "$2")
        #expect(commands.filter { $0.hasPrefix("switch-client -c ") }
                == ["switch-client -c /dev/ttys026 -t $2\n"])
        let retired = await first.switchSession(to: "$0")
        #expect(!retired.contains { $0.hasPrefix("switch-client -c ") },
                "the retired sidecar handed the renderer over and forgot it")
    }
}

// MARK: - Which session the sidecar is on

// The phone on `aaa`, the desktop on `zzz`. tmux lists sessions by name, and
// `#{session_attached}` counts every client — this sidecar's own included —
// so "the last attached session" is `zzz`, the desktop's: the sidecar used
// to take that one at every re-listing, and the next rotation pinned the
// desktop's window to the phone grid.

private let aaaSidecar = FakeClient(control: true, pid: 45613, sessionId: "$0", sessionName: "aaa",
                                    name: "/dev/ttys030")
private let aaaRenderer = FakeClient(control: false, pid: 45533, sessionId: "$0", sessionName: "aaa",
                                     name: "/dev/ttys026")
private let zzzDesktop = FakeClient(control: false, pid: 41001, sessionId: "$1", sessionName: "zzz",
                                    name: "/dev/ttys009")
private let aaaZzzWindows = ["$0 @0 0 b25d,80x24,0,0,0 1 1 zsh", "$1 @1 0 b25e,200x50,0,0,1 1 1 vim"]

@Suite("the mosh sidecar follows the phone's renderer, and nobody else", .serialized)
@MainActor
struct TmuxSidecarRendererFollowTests {

    /// A re-listing: what every `%session-changed`, refresh and session
    /// create, rename or kill sends.
    private func relist(_ rig: SidecarRig) async {
        rig.controller.refresh()
        await rig.answer {
            await rig.transport.recordedCommands().contains("display-message -p done\n")
        }
        await rig.settle()
    }

    private func resizes(_ rig: SidecarRig) async -> [String] {
        await rig.transport.recordedCommands().filter {
            $0.hasPrefix("resize-window") || $0.hasPrefix("refresh-client -C")
        }
    }

    @Test("the phone on aaa, the desktop on zzz: a re-listing keeps the sidecar on aaa, and a rotation sizes aaa")
    func desktopOnALaterSession() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$1 1 zzz"], windows: aaaZzzWindows,
                                        clients: [aaaSidecar, zzzDesktop])
        await rig.land(on: "$0", named: "aaa")
        await rig.attachRenderer(aaaRenderer)
        await relist(rig)
        #expect(rig.controller.snapshot.activeSessionId == "$0", "the breadcrumb names the phone's session")
        #expect(rig.controller.snapshot.activeWindowId == "@0")

        rig.controller.syncMoshWindow(cols: 40, rows: 20)
        await rig.answer { await resizes(rig).count >= 1 }
        await rig.settle()
        #expect(await resizes(rig) == ["resize-window -t @0 -x 40 -y 20\n"],
                "the phone's own window, never the desktop's 200x50 one")
    }

    @Test("the renderer switching session by itself takes the sidecar along, and only the sidecar")
    func rendererMovesOnItsOwn() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$2 1 mmm", "$1 1 zzz"],
                                        windows: aaaZzzWindows + ["$2 @2 0 b25f,80x24,0,0,2 1 1 htop"],
                                        clients: [aaaSidecar, zzzDesktop])
        await rig.land(on: "$0", named: "aaa")
        await rig.attachRenderer(aaaRenderer)
        await rig.settle()
        // `prefix s` in the mosh TUI: the renderer is on mmm now.
        rig.clients = rig.clients.map {
            var client = $0
            if client.pid == aaaRenderer.pid { client.sessionId = "$2"; client.sessionName = "mmm" }
            return client
        }
        rig.controller.refresh()
        #expect(await rig.answer { rig.controller.snapshot.activeSessionId == "$2" })
        await rig.settle()
        let commands = await rig.transport.recordedCommands()
        #expect(commands.contains("switch-client -t $2\n"), "the sidecar goes where the renderer went")
        #expect(!commands.contains { $0.hasPrefix("switch-client -c ") }, "no other client is moved")

        rig.controller.syncMoshWindow(cols: 40, rows: 20)
        await rig.answer { await resizes(rig).count >= 1 }
        await rig.settle()
        #expect(await resizes(rig) == ["resize-window -t @2 -x 40 -y 20\n"])
    }

    @Test("a renderer never identified: a re-listing follows nothing, and a rotation sizes nothing")
    func rendererUnknown() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$1 1 zzz"], windows: aaaZzzWindows,
                                        clients: [aaaSidecar, zzzDesktop, aaaRenderer])
        await rig.land(on: "$0", named: "aaa")
        await relist(rig)
        #expect(rig.controller.snapshot.activeSessionId == "$0", "the session it attached to, not a guess")
        rig.controller.syncMoshWindow(cols: 40, rows: 20)
        rig.controller.repinActiveWindow()
        await rig.settle()
        #expect(await resizes(rig).isEmpty, "no window sized, not even this client")
    }

    @Test("the renderer detached: no window is sized any more")
    func rendererDetached() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$1 1 zzz"], windows: aaaZzzWindows,
                                        clients: [aaaSidecar, zzzDesktop])
        await rig.land(on: "$0", named: "aaa")
        await rig.attachRenderer(aaaRenderer)
        await rig.settle()
        rig.clients.removeAll { $0.pid == aaaRenderer.pid }
        // tmux tells every control client; the rename behind it marks when
        // the detach has been read.
        rig.transport.pushText("%client-detached /dev/ttys026\n%session-renamed $0 aaa2\n")
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.sessions["$0"]?.name == "aaa2"
        })
        rig.controller.syncMoshWindow(cols: 40, rows: 20)
        await rig.settle()
        #expect(await resizes(rig).isEmpty)
    }

    /// iOS killed the sidecar's SSH; mosh kept the renderer. The rebuilt
    /// sidecar's attach lands on the most recent session — the desktop's.
    @Test("a rebuilt sidecar finds the renderer before it sizes anything")
    func rebuiltSidecarFindsTheRenderer() async throws {
        let first = await SidecarRig.make(sessions: ["$0 1 aaa", "$1 1 zzz"], windows: aaaZzzWindows,
                                          clients: [aaaSidecar, zzzDesktop])
        await first.land(on: "$0", named: "aaa")
        await first.attachRenderer(aaaRenderer)
        await first.settle()
        let ledger = first.controller.takePinLedger()

        var rebuilt = aaaSidecar
        rebuilt.pid = 45999
        rebuilt.name = "/dev/ttys031"
        rebuilt.sessionId = "$1"
        rebuilt.sessionName = "zzz"
        let second = await SidecarRig.make(sessions: ["$0 1 aaa", "$1 2 zzz"], windows: aaaZzzWindows,
                                           clients: [rebuilt, zzzDesktop, aaaRenderer], ledger: ledger)
        await second.land(on: "$1", named: "zzz")
        #expect(await second.answer {
            second.controller.snapshot.activeSessionId == "$0" && second.controller.snapshot.activeWindowId == "@0"
        })
        await second.settle()
        #expect(await second.transport.recordedCommands().contains("switch-client -t $0\n"))
        second.controller.syncMoshWindow(cols: 40, rows: 20)
        await second.answer { await resizes(second).count >= 1 }
        await second.settle()
        #expect(await resizes(second) == ["resize-window -t @0 -x 40 -y 20\n"])
    }

    /// A re-listing the switch itself set off lists the renderer where it
    /// still is; following that would drag the sidecar back.
    @Test("a switch is not undone by a listing sent before its moves")
    func switchNotUndone() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$2 0 mmm", "$1 1 zzz"],
                                        windows: aaaZzzWindows + ["$2 @2 0 b25f,80x24,0,0,2 1 1 htop"],
                                        clients: [aaaSidecar, zzzDesktop])
        await rig.land(on: "$0", named: "aaa")
        await rig.attachRenderer(aaaRenderer)
        await rig.settle()
        let before = await rig.transport.recordedCommands().count
        // A re-listing on its way when the person switches: its reply asks
        // where the renderer is, and that question reaches tmux ahead of the
        // moves the switch sends.
        rig.controller.refresh()
        rig.controller.selectSession("$2")
        // The switch's own listing and the re-listing's question about the
        // renderer: both out, both answered.
        await rig.answer {
            await rig.transport.recordedCommands().dropFirst(before)
                .filter { $0.hasPrefix("list-clients") }.count >= 2
        }
        await rig.settle()
        // Queued behind every reply pushed so far: once it is read, so are they.
        rig.transport.pushText("%session-renamed $0 aaa-read\n")
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.sessions["$0"]?.name == "aaa-read"
        })
        let commands = Array(await rig.transport.recordedCommands().dropFirst(before))
        #expect(rig.controller.snapshot.activeSessionId == "$2")
        #expect(commands.filter { $0.hasPrefix("switch-client") }
                == ["switch-client -c /dev/ttys026 -t $2\n", "switch-client -t $2\n"],
                "one switch, nothing sent back towards aaa")
    }
}

// MARK: - The rest of the server

// What tmux 3.6a tells a -CC client when sessions, windows and other
// clients change outside its own session (private socket, recorded for this
// round): `new-session` elsewhere sends `%unlinked-window-add @5`,
// `%sessions-changed`, `%unlinked-window-renamed @5 tmux`; `kill-session`
// elsewhere `%unlinked-window-renamed`, `%sessions-changed`,
// `%unlinked-window-close @5`; killing the window on screen
// `%session-window-changed $0 @0` and then `%unlinked-window-close @1`;
// another client attaching or switching `%client-session-changed
// /dev/ttys024 $1 other`. All of it used to be dropped as unknown.

@Suite("the mosh sidecar hears about sessions, windows and clients that change elsewhere", .serialized)
@MainActor
struct TmuxSidecarServerNotificationTests {

    private func sent(_ rig: SidecarRig, after before: Int, _ prefix: String) async -> Int {
        await rig.transport.recordedCommands().dropFirst(before).filter { $0.hasPrefix(prefix) }.count
    }

    @Test("a session created elsewhere shows up, listed once for the whole burst")
    func sessionCreatedElsewhere() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa"], windows: [aaaZzzWindows[0]],
                                        clients: [aaaSidecar])
        await rig.land(on: "$0", named: "aaa")
        let before = await rig.transport.recordedCommands().count
        rig.sessions.append("$2 0 third")
        rig.windows.append("$2 @5 0 b25f,80x24,0,0,5 1 1 tmux")
        rig.transport.pushText("%unlinked-window-add @5\n%sessions-changed\n%unlinked-window-renamed @5 tmux\n")
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            let sessions = await sent(rig, after: before, "list-sessions")
            let windows = await sent(rig, after: before, "list-windows")
            return sessions > 0 && windows > 0
        }, "only a manual refresh used to show it")
        #expect(await sent(rig, after: before, "list-sessions") == 1)
        #expect(await sent(rig, after: before, "list-windows") == 1)
        #expect(await rig.answer {
            rig.controller.snapshot.sessions["$2"] != nil && rig.controller.snapshot.windows["@5"] != nil
        })
        await rig.settle()
        #expect(rig.controller.snapshot.sessions["$2"]?.name == "third")
        #expect(rig.controller.snapshot.windows["@5"]?.sessionId == "$2")
        #expect(rig.controller.snapshot.activeSessionId == "$0", "nobody follows it")
    }

    @Test("a session killed elsewhere goes, and its window at once")
    func sessionKilledElsewhere() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$2 0 third"],
                                        windows: [aaaZzzWindows[0], "$2 @5 0 b25f,80x24,0,0,5 1 1 bash"],
                                        clients: [aaaSidecar])
        await rig.land(on: "$0", named: "aaa")
        #expect(rig.controller.snapshot.windows["@5"] != nil)
        rig.sessions.removeAll { $0.hasPrefix("$2 ") }
        rig.windows.removeAll { $0.hasPrefix("$2 ") }
        rig.transport.pushText("%unlinked-window-renamed @5 bash\n%sessions-changed\n%unlinked-window-close @5\n")
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.windows["@5"] == nil
        }, "the close itself prunes it, no listing needed")
        #expect(await rig.answer { rig.controller.snapshot.sessions["$2"] == nil })
        #expect(rig.controller.snapshot.sessions["$0"] != nil)
    }

    @Test("a window renamed in another session is renamed here")
    func windowRenamedElsewhere() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$2 0 other"],
                                        windows: [aaaZzzWindows[0], "$2 @4 0 b25f,80x24,0,0,4 1 1 o2"],
                                        clients: [aaaSidecar])
        await rig.land(on: "$0", named: "aaa")
        rig.transport.pushText("%unlinked-window-renamed @4 o2renamed\n")
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.windows["@4"]?.name == "o2renamed"
        })
    }

    /// tmux moves the session's current window — what the renderer shows —
    /// to the one used last, and says so BEFORE the close, while the dying
    /// window is still the sidecar's own: that first notice is turned away.
    @Test("the window on screen killed: the sidecar lands where tmux did, not on a guess")
    func windowOnScreenKilled() async throws {
        let rig = await SidecarRig.make(
            sessions: ["$0 1 aaa"],
            windows: ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 0 1 htop",
                      "$0 @1 2 b25f,80x24,0,0,1 1 1 vim"],
            clients: [aaaSidecar])
        await rig.land(on: "$0", named: "aaa")
        #expect(rig.controller.snapshot.activeWindowId == "@1")
        rig.windows = ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 1 1 htop"]
        rig.transport.pushText("%session-window-changed $0 @2\n%unlinked-window-close @1\n")
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.windows["@1"] == nil
        })
        #expect(await rig.answer { rig.controller.snapshot.activeWindowId != nil })
        #expect(rig.controller.snapshot.activeWindowId == "@2", "tmux's current window, not the first by index")
    }

    @Test("the renderer switching session by itself: the notification takes the sidecar along")
    func rendererSwitchNotified() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$2 0 mmm", "$1 1 zzz"],
                                        windows: aaaZzzWindows + ["$2 @2 0 b25f,80x24,0,0,2 1 1 htop"],
                                        clients: [aaaSidecar, zzzDesktop])
        await rig.land(on: "$0", named: "aaa")
        await rig.attachRenderer(aaaRenderer)
        await rig.settle()
        // `prefix s` in the mosh TUI.
        rig.clients = rig.clients.map {
            var client = $0
            if client.pid == aaaRenderer.pid { client.sessionId = "$2"; client.sessionName = "mmm" }
            return client
        }
        rig.sessions = ["$0 1 aaa", "$2 1 mmm", "$1 1 zzz"]
        rig.transport.pushText("%client-session-changed /dev/ttys026 $2 mmm\n")
        #expect(await rig.answer { rig.controller.snapshot.activeSessionId == "$2" })
        #expect(await rig.transport.recordedCommands().contains("switch-client -t $2\n"))
    }

    @Test("another client switching: its session's attached flag is re-listed, nobody follows it")
    func desktopSwitchNotified() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa", "$2 0 mmm", "$1 1 zzz"],
                                        windows: aaaZzzWindows + ["$2 @2 0 b25f,80x24,0,0,2 1 1 htop"],
                                        clients: [aaaSidecar, zzzDesktop])
        await rig.land(on: "$0", named: "aaa")
        await rig.attachRenderer(aaaRenderer)
        await rig.settle()
        rig.sessions = ["$0 1 aaa", "$2 1 mmm", "$1 0 zzz"]
        rig.transport.pushText("%client-session-changed /dev/ttys009 $2 mmm\n")
        #expect(await rig.answer { rig.controller.snapshot.sessions["$2"]?.isAttached == true })
        await rig.settle()
        #expect(rig.controller.snapshot.sessions["$1"]?.isAttached == false)
        #expect(rig.controller.snapshot.activeSessionId == "$0")
    }

    @Test("a burst of notifications costs one listing of each kind, and one more after it")
    func burstIsCoalesced() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa"], windows: [aaaZzzWindows[0]],
                                        clients: [aaaSidecar])
        await rig.land(on: "$0", named: "aaa")
        let before = await rig.transport.recordedCommands().count
        rig.transport.pushText("%sessions-changed\n%client-session-changed /dev/ttys009 $0 aaa\n"
                               + "%sessions-changed\n%unlinked-window-add @7\n%unlinked-window-add @8\n"
                               + "%unlinked-window-add @9\n%window-renamed @0 marker\n")
        // The rename comes last: once it is in, every notice before it has
        // been read, and nothing has been answered yet.
        #expect(await waitUntil(timeout: loadedDeviceTimeout) {
            rig.controller.snapshot.windows["@0"]?.name == "marker"
        })
        #expect(await sent(rig, after: before, "list-sessions") == 1)
        #expect(await sent(rig, after: before, "list-windows") == 1)
        await rig.answer {
            let sessions = await sent(rig, after: before, "list-sessions")
            let windows = await sent(rig, after: before, "list-windows")
            return sessions >= 2 && windows >= 2
        }
        await rig.settle()
        #expect(await sent(rig, after: before, "list-sessions") == 2)
        #expect(await sent(rig, after: before, "list-windows") == 2)
    }
}

// MARK: - The status line

// `status` is a SESSION option: off on the user's session, it is off for the
// desktop's terminal on that session too. The sidecar used to set it off on
// every `%session-changed` — whatever session it landed on, the user's own
// included — and only a clean disconnect set it back.

@Suite("over mosh the status line goes only on a session Moshpit created", .serialized)
@MainActor
struct TmuxSidecarStatusLineTests {

    @Test("landing on the user's own session, and switching between theirs, sets no status option")
    func usersSessionsUntouched() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 alpha", "$1 2 work"],
                                        clients: [alphaDesktop, workSidecar])
        await rig.land(on: "$1", named: "work")
        _ = await rig.switchSession(to: "$0")
        await rig.land(on: "$0", named: "alpha")
        #expect(await rig.statusCommands().isEmpty,
                "the desktop on these sessions keeps its status bar, and nothing is left behind to restore")
        let restored = Flag()
        Task { @MainActor in
            await rig.controller.restoreSuppressedStatusAndFlush()
            restored.isSet = true
        }
        #expect(await rig.answer { restored.isSet })
        #expect(await rig.statusCommands().isEmpty, "the disconnect owes nothing either")
    }

    @Test("a session Moshpit created goes without its status line while the phone is on it, and gets it back")
    func ownedSessionWhileOnIt() async throws {
        let rig = await SidecarRig.make(sessions: ["$1 2 work"], clients: [workSidecar])
        await rig.land(on: "$1", named: "work")
        rig.sessions.append("$5 0 mine")
        rig.controller.newSession(named: "mine")
        await rig.answer {
            await rig.transport.recordedCommands().contains { $0.hasPrefix("switch-client -t $5") }
        }
        await rig.land(on: "$5", named: "mine")
        #expect(await rig.statusCommands() == ["set-option -t '$5' status off\n"])

        _ = await rig.switchSession(to: "$1")
        await rig.land(on: "$1", named: "work")
        #expect(await rig.statusCommands() == ["set-option -t '$5' status off\n",
                                               "set-option -t '$5' -u status\n"],
                "leaving it gives it back; the user's session is never touched")
    }

    /// iOS kills the sidecar's SSH in the background while mosh keeps the
    /// renderer: the hub rebuilds the sidecar, and the session whose status
    /// line the first one turned off must not be forgotten with it.
    @Test("a rebuilt sidecar still owes the status line its predecessor turned off")
    func rebuiltSidecarGivesItBack() async throws {
        let first = await SidecarRig.make(sessions: ["$1 2 work"], clients: [workSidecar])
        await first.land(on: "$1", named: "work")
        first.sessions.append("$5 0 mine")
        first.controller.newSession(named: "mine")
        await first.answer { await first.statusCommands().contains { $0.contains("status off") } }
        let ledger = first.controller.takePinLedger()

        // The rebuilt sidecar comes back on the user's session.
        let second = await SidecarRig.make(sessions: ["$1 2 work", "$5 0 mine"],
                                           clients: [workSidecar], ledger: ledger)
        await second.land(on: "$1", named: "work")
        #expect(await second.statusCommands() == ["set-option -t '$5' -u status\n"],
                "the session Moshpit created gets its status line back; the user's is left alone")
    }

    @Test("a rebuilt sidecar back on that session keeps it off, and the disconnect gives it back")
    func rebuiltSidecarOnTheSameSession() async throws {
        let first = await SidecarRig.make(sessions: ["$1 2 work"], clients: [workSidecar])
        await first.land(on: "$1", named: "work")
        first.sessions.append("$5 0 mine")
        first.controller.newSession(named: "mine")
        await first.answer { await first.statusCommands().contains { $0.contains("status off") } }
        let ledger = first.controller.takePinLedger()

        let second = await SidecarRig.make(sessions: ["$1 1 work", "$5 1 mine"],
                                           clients: [workSidecar], ledger: ledger)
        await second.land(on: "$5", named: "mine")
        #expect(await second.statusCommands().isEmpty, "still off, still owed: nothing to send yet")
        let restored = Flag()
        Task { @MainActor in
            await second.controller.restoreSuppressedStatusAndFlush()
            restored.isSet = true
        }
        #expect(await second.answer { restored.isSet })
        #expect(await second.statusCommands() == ["set-option -t '$5' -u status\n"])
    }
}

// MARK: - Sessions Moshpit made, known from the server (B86)

/// B86: the sessions Moshpit created were known only in memory. An app killed
/// while the phone sat on one left its status line off, and the next launch's
/// sidecar never knew the session for Moshpit's: off for good, the desktop's
/// status bar with it. Now the session carries `@moshpit_owned`, and the
/// hidden status line a stamp naming the client that hid it.
@Suite("sessions Moshpit made are known from the server (B86)", .serialized)
@MainActor
struct TmuxSidecarOwnedSessionTests {

    @Test("a session Moshpit makes is marked on the server, and its hidden status line stamped")
    func markedAndStamped() async throws {
        let rig = await SidecarRig.make(sessions: ["$1 2 work"], clients: [workSidecar])
        await rig.land(on: "$1", named: "work")
        rig.sessions.append("$5 0 mine")
        rig.controller.newSession(named: "mine")
        await rig.answer {
            await rig.transport.recordedCommands().contains { $0.hasPrefix("switch-client -t $5") }
        }
        await rig.land(on: "$5", named: "mine")
        #expect(await rig.markCommands() == ["set-option -t '$5' @moshpit_owned 1\n",
                                             "set-option -t '$5' -F @moshpit_status_hidden '#{client_name}'\n"])
        _ = await rig.switchSession(to: "$1")
        await rig.land(on: "$1", named: "work")
        #expect(await rig.markCommands().last == "set-option -t '$5' -u @moshpit_status_hidden\n")
    }

    @Test("after a killed app, the status line a dead sidecar left off comes back, and the session is Moshpit's")
    func killedAppGivesItBack() async throws {
        let rig = await SidecarRig.booting(sessions: ["$1 1 work", "$5 0 mine", "$6 0 theirs"],
                                           clients: [FakeClient(control: true, pid: 45613, sessionId: "$1",
                                                                sessionName: "work", name: "client-45613")],
                                           marks: ["$1 owned= hidden=", "$5 owned=1 hidden=client-99999"],
                                           landing: ("$1", "work"))
        #expect(await rig.statusCommands() == ["set-option -t '$5' -u status\n"])
        #expect(rig.controller.ownedSessionIds == ["$5"])
        _ = await rig.switchSession(to: "$5")
        await rig.land(on: "$5", named: "mine")
        #expect(await rig.statusCommands().last == "set-option -t '$5' status off\n")
    }

    @Test("after a killed app, back on that very session: it stays off, ours now")
    func killedAppOnThatSession() async throws {
        let rig = await SidecarRig.booting(sessions: ["$1 1 work", "$5 0 mine", "$6 0 theirs"],
                                           clients: [FakeClient(control: true, pid: 45613, sessionId: "$5",
                                                                sessionName: "mine", name: "client-45613")],
                                           marks: ["$1 owned= hidden=", "$5 owned=1 hidden=client-99999"],
                                           landing: ("$5", "mine"))
        #expect(!(await rig.statusCommands()).contains { $0.contains("-u status") })
        _ = await rig.switchSession(to: "$1")
        await rig.land(on: "$1", named: "work")
        #expect(await rig.statusCommands().last == "set-option -t '$5' -u status\n")
    }

    @Test("a live client's stamp and the user's own sessions are left alone")
    func liveStampAndUsersSessions() async throws {
        let rig = await SidecarRig.booting(sessions: ["$1 1 work", "$5 0 mine", "$6 0 theirs"],
                                           clients: [FakeClient(control: true, pid: 45613, sessionId: "$1",
                                                                sessionName: "work", name: "client-45613"),
                                                     FakeClient(control: true, pid: 45999, sessionId: "$5",
                                                                sessionName: "mine", name: "client-45999")],
                                           marks: ["$1 owned= hidden=", "$5 owned=1 hidden=client-45999",
                                                   "$6 owned= hidden="],
                                           landing: ("$1", "work"))
        _ = await rig.switchSession(to: "$6")
        await rig.land(on: "$6", named: "theirs")
        #expect(await rig.statusCommands().isEmpty)
        #expect(await rig.markCommands().isEmpty)
    }
}

// MARK: - The rows

@Suite("the sidecar's list-clients rows")
struct TmuxClientRowTests {

    @Test("tmux 3.6a's rows: a terminal and a -CC client")
    func realRows() throws {
        let terminal = try #require(TmuxClientRow("0|45533|$1|/dev/ttys026"))
        #expect(!terminal.isControl)
        #expect(terminal.sessionId == "$1")
        #expect(terminal.identity == TmuxClientIdentity(pid: 45533, name: "/dev/ttys026"))
        let control = try #require(TmuxClientRow("1|45613|$0|client-45613"))
        #expect(control.isControl)
        #expect(control.identity == TmuxClientIdentity(pid: 45613, name: "client-45613"))
        #expect(TmuxClientRow("0|7|$3|/dev/pts/12")?.identity?.name == "/dev/pts/12")
    }

    @Test("a row that is not plain enough to act on still counts, but is nobody",
          arguments: ["0|a|b|/dev/ttys026",      // the old name-in-the-middle shape
                      "0||$1|/dev/ttys026",      // no pid
                      "0|-5|$1|/dev/ttys026",    // not a pid
                      "0|１２|$1|/dev/ttys026",  // fullwidth digits
                      "0|12|$1|/dev/tty s",      // would split the command
                      "0|12|$1|/dev/tty;x"])
    func unplainRows(_ line: String) throws {
        let row = try #require(TmuxClientRow(line))
        #expect(!row.isControl)
        #expect(row.identity == nil)
    }

    @Test("not a row at all", arguments: ["", "0", "0|1|$1", "2|1|$1|x", "0|1|$1|", "zsh: no such file"])
    func notRows(_ line: String) {
        #expect(TmuxClientRow(line) == nil)
    }
}

// MARK: - The control client

@Suite("tmux control client: %session-renamed")
struct TmuxSessionRenamedParserTests {

    @Test("%session-renamed hands over the id and the whole new name", arguments: [
        ("%session-renamed $1 x|y\n", "$1", "x|y"),
        ("%session-renamed $12 my project\n", "$12", "my project"),
        ("%session-renamed $3 \u{301}acute\n", "$3", "\u{301}acute"),
    ])
    func sessionRenamed(_ line: String, _ id: String, _ name: String) async throws {
        final class Box: @unchecked Sendable {
            var renamed: [(String, String)] = []
            var errors = 0
        }
        let box = Box()
        let client = TmuxControlClient()
        await client.setCallbacks(onSessionRenamed: { box.renamed.append(($0, $1)) },
                                  onProtocolError: { _ in box.errors += 1 })
        await client.feed(Data("%begin 900 500 0\n%end 900 500 0\n".utf8))
        await client.feed(Data(line.utf8))
        #expect(box.renamed.count == 1)
        #expect(box.renamed.first?.0 == id)
        #expect(box.renamed.first.map { Array($0.1.utf8) } == Array(name.utf8))
        #expect(box.errors == 0)
    }
}

/// B270: mosh + tmux with the phone the only client. The renderer — tmux's
/// client, mosh-server's program — sizes the session from the moment it
/// attaches, before the sidecar exists, so the sidecar's records were of
/// layouts already squashed; and on the disconnect the windows went back to
/// automatic sizing with the renderer still attached (and the sidecar itself
/// a sizing client): straight back to the phone's grid.
@Suite("the mosh sidecar hands the windows back as they were (B270)", .serialized)
@MainActor
struct TmuxSidecarHandBackTests {
    private static let split = "bf8c,200x60,0,0[200x30,0,0,0,200x29,0,31{100x29,0,31,1,99x29,101,31,2}]"
    private static let squashed = "307e,60x20,0,0[60x10,0,0,0,60x9,0,11{30x9,0,11,1,29x9,31,11,2}]"

    @Test("the layouts the renderer squashed before the sidecar attached are its records")
    func recordsWhatTheRendererSquashed() async throws {
        let rig = await SidecarRig.make(sessions: ["$0 1 aaa"],
                                        windows: ["$0 @0 0 \(Self.squashed) 1 3 zsh"],
                                        clients: [aaaSidecar])
        rig.controller.noteLayoutsBeforeRenderer("@0 3 0 %0 \(Self.split)\n@1 1 0 %3 6c3d,200x60,0,0,3\n")
        rig.sizeRows = ["@0 3 0 %0 \(Self.squashed)", "@1 1 0 %3 6c3d,200x60,0,0,3"]
        await rig.land(on: "$0", named: "aaa")
        await rig.settle()
        let probes = await rig.transport.recordedCommands()
            .filter { $0.hasPrefix("list-windows -a -F '\(TmuxSessionController.sizeProbeFormat)'") }
        #expect(probes.count == 1, "\(probes)")
        let restore = rig.controller.pristineLayoutRestoreCommands()
        #expect(restore == ["resize-window -t @0 -x 200 -y 60",
                            "select-layout -t @0 '\(Self.split)'",
                            "set-option -u -w -t @0 window-size"], "\(restore)")
    }

    private static let wideTwo = "c1a2,200x60,0,0{100x60,0,0,3,99x60,101,0[99x30,101,0,4,99x29,101,31,5]}"
    private static let squashedTwo = "51b0,60x20,0,0{30x20,0,0,3,29x20,31,0[29x10,31,0,4,29x9,31,11,5]}"

    /// Two sessions of a 3-pane window each, the phone's renderer on `$1`
    /// and known by its pid, its window pinned: a window is at the phone's
    /// grid while the renderer sits on its session.
    private func twoSplitSessions() async -> SidecarRig {
        let sidecarClient = FakeClient(control: true, pid: 45613, sessionId: "$1", sessionName: "work",
                                       name: "client-45613")
        let rendererClient = FakeClient(control: false, pid: 45533, sessionId: "$1", sessionName: "work",
                                        name: "/dev/ttys026")
        let rig = await SidecarRig.make(sessions: ["$1 1 work", "$2 0 play"],
                                        windows: ["$1 @1 0 \(Self.squashed) 1 3 one",
                                                  "$2 @2 0 \(Self.wideTwo) 1 3 two"],
                                        clients: [sidecarClient, rendererClient])
        let split = Self.split, squashed = Self.squashed
        let wideTwo = Self.wideTwo, squashedTwo = Self.squashedTwo
        rig.custom = { command, clients in
            let on = { (session: String) in clients.contains { !$0.control && $0.sessionId == session } }
            let one = on("$1") ? squashed : split
            let two = on("$2") ? squashedTwo : wideTwo
            if command.hasPrefix("list-windows -t '$2' -F '\(TmuxSessionController.sizeProbeFormat)'") {
                return ["@2 3 0 %3 \(two)"]
            }
            if command.hasPrefix("display-message -p -t '@1' '#{window_layout}'") { return [one] }
            return nil
        }
        rig.controller.noteLayoutsBeforeRenderer("@1 3 0 %0 \(split)\n@2 3 0 %3 \(wideTwo)\n")
        rig.sizeRows = ["@1 3 0 %0 \(squashed)", "@2 3 0 %3 \(wideTwo)"]
        await rig.land(on: "$1", named: "work")
        let adopted = Task { await rig.controller.adoptRenderer(pid: 45533) }
        await rig.answer { rig.controller.rendererClient != nil }
        _ = await adopted.value
        await rig.settle()
        rig.controller.syncMoshWindow(cols: 60, rows: 20)
        await rig.settle()
        return rig
    }

    @Test("round 34: a session switch records what the renderer's move squashes, read on both sides of it")
    func switchRecordsWhatTheMoveSquashes() async throws {
        let rig = await twoSplitSessions()
        #expect(rig.controller.rendererSessionId == "$1")
        let base = await rig.transport.recordedCommands().count
        rig.controller.selectSession("$2")
        await rig.answer {
            await rig.transport.recordedCommands()
                .filter { $0.hasPrefix("list-windows -t '$2'") }.count >= 2
        }
        await rig.settle()
        let sent = Array(await rig.transport.recordedCommands().dropFirst(base))
        let probes = sent.indices.filter { sent[$0].hasPrefix("list-windows -t '$2'") }
        let moved = try #require(sent.firstIndex { $0.hasPrefix("switch-client -c /dev/ttys026 -t $2") })
        #expect(probes.count == 2 && probes[0] < moved && moved < probes[1], "\(sent)")
        let restore = rig.controller.pristineLayoutRestoreCommands()
        #expect(restore.contains("select-layout -t @2 '\(Self.wideTwo)'"), "\(restore)")
    }

    @Test("round 34: the session the renderer left is handed back: unpinned, its records replayed after the move")
    func switchHandsTheSessionLeftBack() async throws {
        let rig = await twoSplitSessions()
        // The transport records each command as written: with its line end.
        func commands() async -> [String] {
            await rig.transport.recordedCommands().map { $0.trimmingCharacters(in: .newlines) }
        }
        #expect(await commands().contains("resize-window -t @1 -x 60 -y 20"))
        let base = await commands().count
        rig.controller.selectSession("$2")
        await rig.answer {
            await commands().contains("select-layout -t '@1' '\(Self.split)'")
        }
        await rig.settle()
        let sent = Array(await commands().dropFirst(base))
        let moved = try #require(sent.firstIndex { $0.hasPrefix("switch-client -c /dev/ttys026 -t $2") })
        let unpin = try #require(sent.firstIndex(of: "set-option -u -w -t @1 window-size"), "\(sent)")
        let replay = try #require(sent.firstIndex(of: "select-layout -t '@1' '\(Self.split)'"), "\(sent)")
        #expect(moved < unpin && unpin < replay, "\(sent)")
        #expect(!rig.controller.resizedWindows.contains("@1"))
        let restore = rig.controller.pristineLayoutRestoreCommands()
        #expect(!restore.contains { $0.contains("@1") }, "\(restore)")
    }

    @Test("the renderer is detached by its pid and name before the hand-back, never another client")
    func detachCommand() {
        let renderer = TmuxClientIdentity(pid: 4481, name: "/dev/ttys023")
        #expect(TmuxSessionController.rendererDetachCommand(tmux: "tmux", renderer: renderer)
                == "tmux list-clients -F '#{client_pid} #{client_name}' 2>/dev/null | grep -qxF '4481 /dev/ttys023' "
                + "&& tmux detach-client -t '/dev/ttys023'; true")
        let terminal = TmuxSessionController(sshSession: MockTmuxTransport())
        #expect(terminal.rendererDetachCommand(tmux: "tmux") == nil, "a terminal's controller has no renderer")
    }
}


// MARK: - The window over mosh is tmux's current window (B80)

@Suite("over mosh the sidecar's window is tmux's current window", .serialized)
@MainActor
struct TmuxSidecarCurrentWindowTests {

    /// B80: mosh shows tmux's own TUI through a regular client of the
    /// session, and a session's current window is shared by all its
    /// clients — so after `prefix n` in the mosh TUI, or a desktop's
    /// `select-window`, the phone's screen is on the new window. The sidecar
    /// kept the SSH rule (keep the phone's own choice) and its breadcrumb,
    /// the next pin and the next zoom stayed on the old one.
    @Test("prefix n in the mosh TUI, or a desktop's select-window, moves the breadcrumb with the screen")
    func followsTmuxCurrentWindow() async throws {
        let rig = await SidecarRig.make(
            sessions: ["$0 1 aaa"],
            windows: ["$0 @0 0 b25d,80x24,0,0,0 1 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 0 1 htop"],
            clients: [FakeClient(control: true, pid: 45613, sessionId: "$0", sessionName: "aaa",
                                 name: "client-45613")])
        rig.transport.pushText("%session-changed $0 aaa\n")
        await rig.settle()
        #expect(rig.controller.snapshot.activeWindowId == "@0")

        // What tmux 3.6a sends every control client of the session.
        rig.windows = ["$0 @0 0 b25d,80x24,0,0,0 0 1 zsh", "$0 @2 1 b25e,80x24,0,0,2 1 1 htop"]
        rig.transport.pushText("%session-window-changed $0 @2\n")
        await rig.settle()
        #expect(rig.controller.snapshot.activeWindowId == "@2", "the breadcrumb is on the window mosh shows")

        // A listing says the same, and keeps it there.
        rig.controller.refresh()
        await rig.settle()
        #expect(rig.controller.snapshot.activeWindowId == "@2")
    }
}
