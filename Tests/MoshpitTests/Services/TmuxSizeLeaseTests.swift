import Foundation
import Testing
import SwiftTerm
@testable import Moshpit

// Two Moshpit devices on one tmux window: the size lease in
// `@moshpit_size_owner` decides whose grid the window follows. These tests
// drive one controller ("moshpit-a", an iPhone) against a mock tmux whose
// replies describe the other device ("moshpit-b", an iPad).

// MARK: - Helpers

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

private func block(_ num: Int, _ body: String) -> String {
    body.isEmpty
        ? "%begin \(num) \(num) 0\n%end \(num) \(num) 0\n\n"
        : "%begin \(num) \(num) 0\n\(body)\n%end \(num) \(num) 0\n\n"
}

/// The lease line the other device would have written: `client|label|stamp`.
private func lease(_ client: String, label: String = "iPad", idleFor: TimeInterval) -> String {
    "\(client)|\(label)|\(Int(Date().timeIntervalSince1970 - idleFor))"
}

/// What tmux answers each command the controller sends, by prefix. Every
/// command gets a reply slot (control mode is FIFO), so unknown commands get
/// an empty block.
private struct TmuxReplies {
    /// `#{client_name}` of every attached client. We are always "moshpit-a".
    var clients: [String] = ["moshpit-a"]
    /// The window's `@moshpit_size_owner`; nil is unset (tmux prints nothing).
    var lease: String?
    var capture: String = "READY"

    func reply(to command: String, slot: Int) -> String {
        if command.hasPrefix("list-clients -F '#{client_name}'") {
            return block(slot, clients.joined(separator: "\n"))
        }
        if command.hasPrefix("show-options -wqv") { return block(slot, lease ?? "") }
        if command.hasPrefix("capture-pane") { return block(slot, capture) }
        if command.hasPrefix("display-message") { return block(slot, "0 0") }
        return block(slot, "")
    }
}

/// Answer, in FIFO order, every command recorded past `answered` until the
/// controller goes quiet. Returns the new count.
@MainActor
private func answerAll(_ transport: MockTmuxTransport, from answered: Int,
                       with replies: TmuxReplies) async -> Int {
    var answered = answered
    for _ in 0..<20 {
        let commands = await transport.recordedCommands()
        while answered < commands.count {
            transport.pushText(replies.reply(to: commands[answered], slot: 700 + answered))
            answered += 1
        }
        try? await Task.sleep(for: .milliseconds(20))
        if await transport.recordedCommands().count == answered { break }
    }
    return answered
}

private extension Sequence where Element == String {
    func has(prefix: String) -> Bool { contains { $0.hasPrefix(prefix) } }
    func first(prefix: String) -> String? { first { $0.hasPrefix(prefix) } }
}

private let claimPrefix = "set-option -w -t @0 @moshpit_size_owner"
private let releasePrefix = "set-option -wu -t @0 @moshpit_size_owner"
private let pinPrefix = "resize-window -t @0 -x 70 -y 35"
private let probePrefix = "show-options -wqv -t @0 @moshpit_size_owner"

/// An attached iPhone-sized controller on a one-window session whose window
/// tmux reports at a desktop's 200 columns. The attach pin has gone through
/// the lease gate with `replies` describing the other device; `answered` is
/// how many FIFO slots were answered so far.
private struct Rig {
    let controller: TmuxSessionController
    let transport: MockTmuxTransport
    let answered: Int
}

@MainActor
private func makeRig(replies: TmuxReplies) async -> Rig {
    let transport = MockTmuxTransport()
    let controller = TmuxSessionController(sshSession: transport)
    controller.setInitialClientSize(cols: 70, rows: 35)
    controller.ourClientName = "moshpit-a"
    controller.sizeOwnerLabel = "iPhone"
    // Keep the quiet re-check out of tests that do not ask for it.
    controller.foreignQuietRepinDelay = 30
    await controller.attach()
    transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
    _ = await waitUntil { await transport.recordedCommands().count >= 3 }
    transport.pushText("""
    %begin 1 1 0
    $0 1 main
    %end 1 1 0
    %begin 2 2 0
    $0 @0 0 200x50,0,0,0 1 1 main
    %end 2 2 0
    %begin 3 3 0
    %0 @0 0 200 50 1 0 0 bash
    %end 3 3 0

    """)
    _ = await waitUntil { controller.snapshot.activePaneId == "%0" }
    let answered = await answerAll(transport, from: 3, with: replies)
    return Rig(controller: controller, transport: transport, answered: answered)
}

private func screenText(_ terminal: Terminal) -> String {
    terminal.getText(start: Position(col: 0, row: 0), end: Position(col: 69, row: 34))
}

// MARK: - Suite

@Suite("tmux window size lease", .serialized)
@MainActor
struct TmuxSizeLeaseTests {

    // ─────────────────────────────────────────────────────────────
    // Attach: who gets the window
    // ─────────────────────────────────────────────────────────────

    @Test("a free window is claimed (passively) and pinned on attach")
    func attachClaimsFreeWindow() async throws {
        let rig = await makeRig(replies: TmuxReplies())
        let transport = rig.transport
        let commands = await transport.recordedCommands()
        #expect(commands.has(prefix: probePrefix), "attach asks who holds the window before pinning")
        #expect(commands.has(prefix: "\(claimPrefix) \"moshpit-a|iPhone|0\""))
        #expect(commands.has(prefix: pinPrefix))
        let claim = try #require(commands.firstIndex { $0.hasPrefix(claimPrefix) })
        let pin = try #require(commands.firstIndex { $0.hasPrefix(pinPrefix) })
        #expect(claim < pin, "the claim is written before the pin")
    }

    @Test("a window another live device just used is left at its size: no pin, no claim")
    func attachDefersToLiveHolder() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        let commands = await transport.recordedCommands()
        #expect(!commands.has(prefix: pinPrefix), "the iPad is typing into this window")
        #expect(!commands.has(prefix: claimPrefix))
        #expect(controller.sizeFollow == nil, "nothing to veil until the window reads foreign")
    }

    @Test("a holder idle past foregroundStealAfter loses the window to an attaching device")
    func attachTakesIdleHolder() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 60))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        let commands = await transport.recordedCommands()
        #expect(commands.has(prefix: "\(claimPrefix) \"moshpit-a|iPhone|0\""),
                "a passive claim: we did not interact, we merely came first")
        #expect(commands.has(prefix: pinPrefix))
        #expect(controller.sizeFollow == nil)
    }

    @Test("a lease left by a client that is no longer attached is nobody's")
    func attachTakesGhostLease() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a"],
                                  lease: lease("moshpit-b", idleFor: 1))
        let rig = await makeRig(replies: replies)
        let transport = rig.transport
        let commands = await transport.recordedCommands()
        #expect(commands.has(prefix: "\(claimPrefix) \"moshpit-a|iPhone|0\""))
        #expect(commands.has(prefix: pinPrefix))
    }

    // ─────────────────────────────────────────────────────────────
    // Following
    // ─────────────────────────────────────────────────────────────

    @Test("drift on a window a live device holds is followed, not reclaimed; its output stays gated")
    func driftFollowsInsteadOfReclaiming() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        let answered = rig.answered
        let terminal = controller.terminalView(for: "%0").getTerminal()
        #expect(await waitUntil { screenText(terminal).contains("READY") })
        let before = await transport.recordedCommands().count

        // The iPad's activity re-lays the window out at its grid.
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil {
            controller.sizeFollow == TmuxSessionController.SizeFollow(windowId: "@0", ownerLabel: "iPad")
        }, "the veil names the holder")
        _ = await answerAll(transport, from: answered, with: replies)
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(!after.has(prefix: pinPrefix), "following means leaving the size alone")
        #expect(!after.has(prefix: claimPrefix))

        // Output laid out for 200 columns would shred on our 70: the frame
        // the person last saw stays put under the veil.
        transport.pushText("%output %0 IPAD-SIZED\n")
        try? await Task.sleep(for: .milliseconds(150))
        #expect(!screenText(terminal).contains("IPAD-SIZED"))
        #expect(screenText(terminal).contains("READY"))
    }

    @Test("a live holder keeps the window however long it idles: the quiet re-check never steals")
    func recheckNeverStealsFromLiveHolder() async throws {
        var replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil { controller.sizeFollow != nil })
        var answered = await answerAll(transport, from: rig.answered, with: replies)

        // The iPad stops typing but stays on the window — idle far past what
        // an attaching or foregrounding device may take.
        replies.lease = lease("moshpit-b", idleFor: 60)
        let before = await transport.recordedCommands().count
        controller.foreignQuietRepinDelay = 0.1
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        // Several re-check rounds go by, each answered "still the iPad's".
        let deadline = Date().addingTimeInterval(0.6)
        while Date() < deadline {
            answered = await answerAll(transport, from: answered, with: replies)
            try? await Task.sleep(for: .milliseconds(30))
        }
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(after.filter { $0.hasPrefix(probePrefix) }.count >= 2, "the re-check keeps asking")
        #expect(!after.has(prefix: pinPrefix), "a drift or re-check never steals from a live holder")
        #expect(!after.has(prefix: claimPrefix))
        #expect(controller.sizeFollow?.ownerLabel == "iPad", "still following")
    }

    @Test("while following, the re-check that finds the window free takes it and drops the veil")
    func recheckTakesFreedWindow() async throws {
        var replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil { controller.sizeFollow != nil })
        var answered = await answerAll(transport, from: rig.answered, with: replies)

        // The iPad backgrounds: its lease is gone by the next re-check.
        replies.lease = nil
        let before = await transport.recordedCommands().count
        controller.foreignQuietRepinDelay = 0.1
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().dropFirst(before).has(prefix: probePrefix)
        }, "the quiet re-check probes the lease")
        answered = await answerAll(transport, from: answered, with: replies)
        #expect(await waitUntil { controller.sizeFollow == nil })
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(after.has(prefix: "\(claimPrefix) \"moshpit-a|iPhone|0\""))
        #expect(after.has(prefix: pinPrefix))
    }

    @Test("a re-check also takes a window whose holder vanished (crash, reconnect)")
    func recheckTakesWindowOfVanishedHolder() async throws {
        var replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil { controller.sizeFollow != nil })
        var answered = await answerAll(transport, from: rig.answered, with: replies)

        replies.clients = ["moshpit-a"]   // the lease line is still there
        let before = await transport.recordedCommands().count
        controller.foreignQuietRepinDelay = 0.1
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().dropFirst(before).has(prefix: probePrefix)
        })
        answered = await answerAll(transport, from: answered, with: replies)
        #expect(await waitUntil { controller.sizeFollow == nil })
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(after.has(prefix: claimPrefix))
        #expect(after.has(prefix: pinPrefix))
    }

    // ─────────────────────────────────────────────────────────────
    // Taking over
    // ─────────────────────────────────────────────────────────────

    @Test("the follower's tap takes the window: a stamped claim, then the pin, and output flows again")
    func tapTakesOver() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        let terminal = controller.terminalView(for: "%0").getTerminal()
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil { controller.sizeFollow != nil })
        let before = await transport.recordedCommands().count

        controller.takeSizeOwnership()
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(before).has(prefix: pinPrefix)
        })
        #expect(controller.sizeFollow == nil)
        let after = await transport.recordedCommands().dropFirst(before)
        let claim = after.first(prefix: claimPrefix)
        #expect(claim != nil)
        #expect(claim?.contains("moshpit-a|iPhone|") == true)
        #expect(claim?.contains("|0\"") == false, "an interaction stamps the claim with now, not a passive 0")
        let claimAt = try #require(after.firstIndex { $0.hasPrefix(claimPrefix) })
        let pinAt = try #require(after.firstIndex { $0.hasPrefix(pinPrefix) })
        #expect(claimAt < pinAt)

        transport.pushText("%output %0 MINE-AGAIN\n")
        #expect(await waitUntil { screenText(terminal).contains("MINE-AGAIN") },
                "the gate opens with the veil")
    }

    @Test("a keystroke into a followed window takes it over too")
    func keystrokeTakesOver() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil { controller.sizeFollow != nil })
        let before = await transport.recordedCommands().count

        controller.sendInput(Data("x".utf8), paneId: "%0")
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(before).has(prefix: "send-keys -t %0")
        })
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(after.has(prefix: claimPrefix))
        #expect(after.has(prefix: pinPrefix))
        #expect(controller.sizeFollow == nil)
        let pinAt = try #require(after.firstIndex { $0.hasPrefix(pinPrefix) })
        let keysAt = try #require(after.firstIndex { $0.hasPrefix("send-keys") })
        #expect(pinAt < keysAt, "the window is ours before the keystroke lands in it")
    }

    // ─────────────────────────────────────────────────────────────
    // The stamp
    // ─────────────────────────────────────────────────────────────

    @Test("keystrokes refresh our stamp, at most once per leaseStampRefreshInterval")
    func keystrokesRefreshStampThrottled() async throws {
        let rig = await makeRig(replies: TmuxReplies())
        let controller = rig.controller
        let transport = rig.transport
        controller.leaseStampRefreshInterval = 0.3
        func claims() async -> [String] {
            await transport.recordedCommands().filter { $0.hasPrefix(claimPrefix) }
        }
        #expect(await claims().count == 1, "attach wrote the passive claim")

        controller.sendInput(Data("a".utf8), paneId: "%0")
        controller.sendInput(Data("b".utf8), paneId: "%0")
        #expect(await waitUntil { await claims().count == 2 })
        try? await Task.sleep(for: .milliseconds(100))
        #expect(await claims().count == 2, "the second keystroke inside the interval writes nothing")
        #expect(await claims().last?.contains("|0\"") == false, "the refresh carries a real stamp")

        try? await Task.sleep(for: .milliseconds(300))
        controller.sendInput(Data("c".utf8), paneId: "%0")
        #expect(await waitUntil { await claims().count == 3 })
    }

    // ─────────────────────────────────────────────────────────────
    // Background / foreground
    // ─────────────────────────────────────────────────────────────

    @Test("backgrounding hands our lease back with the pin")
    func backgroundReleasesLease() async throws {
        let rig = await makeRig(replies: TmuxReplies())
        let controller = rig.controller
        let transport = rig.transport
        controller.releaseWindowPins()
        #expect(await waitUntil { await transport.recordedCommands().has(prefix: releasePrefix) })
        let commands = await transport.recordedCommands()
        let unpin = commands.firstIndex { $0.hasPrefix("set-option -u -w -t @0 window-size") }
        let release = commands.firstIndex { $0.hasPrefix(releasePrefix) }
        #expect(unpin != nil && release != nil)
    }

    @Test("coming to the front asks first, and defers to a device that took the window meanwhile")
    func foregroundDefersToNewHolder() async throws {
        var replies = TmuxReplies()
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        controller.releaseWindowPins()
        _ = await waitUntil { await transport.recordedCommands().has(prefix: releasePrefix) }
        var answered = await answerAll(transport, from: rig.answered, with: replies)

        // While we were away the iPad opened the window and is typing.
        replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                              lease: lease("moshpit-b", idleFor: 1))
        let before = await transport.recordedCommands().count
        controller.foregroundProbeTimeout = 30
        controller.repinActiveWindow()
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(before).has(prefix: probePrefix)
        }, "foreground probes instead of pinning blind")
        answered = await answerAll(transport, from: answered, with: replies)
        try? await Task.sleep(for: .milliseconds(150))
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(after.has(prefix: "refresh-client -f !ignore-size"), "we rejoin the sizing math either way")
        #expect(!after.has(prefix: pinPrefix), "the window is the iPad's now")
        #expect(!after.has(prefix: claimPrefix))

        // Its next drift veils us as a follower.
        transport.pushText("%layout-change @0 c71d,200x50,0,0,0 c71d,200x50,0,0,0 *\n")
        #expect(await waitUntil { controller.sizeFollow?.ownerLabel == "iPad" })
    }

    @Test("coming to the front takes a window nobody claimed while we were away")
    func foregroundTakesFreeWindow() async throws {
        let replies = TmuxReplies()
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        let transport = rig.transport
        controller.releaseWindowPins()
        _ = await waitUntil { await transport.recordedCommands().has(prefix: releasePrefix) }
        var answered = await answerAll(transport, from: rig.answered, with: replies)

        let before = await transport.recordedCommands().count
        controller.foregroundProbeTimeout = 30
        controller.repinActiveWindow()
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(before).has(prefix: probePrefix)
        })
        answered = await answerAll(transport, from: answered, with: replies)
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(before).has(prefix: pinPrefix)
        })
        let after = await transport.recordedCommands().dropFirst(before)
        #expect(after.has(prefix: "\(claimPrefix) \"moshpit-a|iPhone|0\""))
    }

    @Test("if tmux never answers the foreground probe, the window is pinned anyway")
    func foregroundProbeTimeoutPins() async throws {
        let rig = await makeRig(replies: TmuxReplies())
        let controller = rig.controller
        let transport = rig.transport
        controller.releaseWindowPins()
        _ = await waitUntil { await transport.recordedCommands().has(prefix: releasePrefix) }
        let before = await transport.recordedCommands().count
        controller.foregroundProbeTimeout = 0.1
        controller.repinActiveWindow()
        #expect(await waitUntil(timeout: 1.5) {
            await transport.recordedCommands().dropFirst(before).has(prefix: pinPrefix)
        }, "a silent tmux must not leave the screen frozen behind the gate")
    }

    // ─────────────────────────────────────────────────────────────
    // Teardown
    // ─────────────────────────────────────────────────────────────

    @Test("sizeLeaseReleaseCommands unsets only leases that are ours, once")
    func releaseCommandsForTeardown() async throws {
        let rig = await makeRig(replies: TmuxReplies())
        let controller = rig.controller
        #expect(controller.sizeLeaseReleaseCommands() == ["set-option -wu -t @0 @moshpit_size_owner"])
        #expect(controller.sizeLeaseReleaseCommands().isEmpty, "consumed")
    }

    @Test("sizeLeaseReleaseCommands is empty when we were only following")
    func noReleaseCommandsWhenFollowing() async throws {
        let replies = TmuxReplies(clients: ["moshpit-a", "moshpit-b"],
                                  lease: lease("moshpit-b", idleFor: 2))
        let rig = await makeRig(replies: replies)
        let controller = rig.controller
        #expect(controller.sizeLeaseReleaseCommands().isEmpty)
    }
}

// MARK: - Lease line parsing

@Suite("tmux size lease line")
struct TmuxSizeLeaseParsingTests {
    typealias Lease = TmuxSessionController.SizeLease

    @Test("unset (empty) and a harness's generic `0 0` are not leases")
    func rejectsNonLeases() {
        #expect(Lease(parsing: "", clients: ["a"]) == nil)
        #expect(Lease(parsing: "0 0", clients: ["a"]) == nil)
        #expect(Lease(parsing: "a|iPad", clients: ["a"]) == nil)
        #expect(Lease(parsing: "|iPad|12", clients: ["a"]) == nil)
        #expect(Lease(parsing: "a||12", clients: ["a"]) == nil)
        #expect(Lease(parsing: "a|iPad|soon", clients: ["a"]) == nil)
    }

    @Test("liveness is whether the holder is among the attached clients")
    func liveness() {
        #expect(Lease(parsing: "b|iPad|12", clients: ["a", "b"])?.live == true)
        #expect(Lease(parsing: "b|iPad|12", clients: ["a"])?.live == false)
    }

    @Test("an empty client list is a failed listing, not an empty server: err toward live")
    func emptyClientsReadsLive() {
        #expect(Lease(parsing: "b|iPad|12", clients: [])?.live == true)
    }

    @Test("fields and encoding round-trip with an integer stamp")
    func roundTrip() {
        let parsed = Lease(parsing: "moshpit-b|iPad|1757900000", clients: ["moshpit-b"])
        #expect(parsed?.clientName == "moshpit-b")
        #expect(parsed?.label == "iPad")
        #expect(parsed?.stamp == 1_757_900_000)
        #expect(parsed?.encoded == "moshpit-b|iPad|1757900000")
        #expect(Lease(clientName: "a", label: "iPhone", stamp: 12.9, live: true).encoded == "a|iPhone|12")
    }
}
