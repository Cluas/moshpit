import Foundation
import Testing
import SwiftTerm
@testable import Moshpit

// MARK: - Helpers

private func bytes(_ s: String) -> Data { Data(s.utf8) }

/// Wait for an `@MainActor`-isolated async predicate to become true (poll
/// every 5 ms up to `timeout`). Async so callers can `await` actor-isolated
/// values inside the predicate body (e.g. `await transport.recordedCommands()`).
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

/// Build a controller wired to a fresh `MockTmuxTransport`, attach it, and
/// return both so each test can drive the parser side of the protocol while
/// asserting on the snapshot.
@MainActor
private func makeAttachedController() async -> (TmuxSessionController, MockTmuxTransport) {
    let transport = MockTmuxTransport()
    let controller = TmuxSessionController(sshSession: transport)
    await controller.attach()
    // Real tmux answers the boot line (`-CC attach`) with the connection's
    // first reply block; TmuxControlClient swallows each control session's
    // boot block (see awaitingBootBlock). The mock sends it so the tests
    // exercise the same stream shape real tmux produces.
    transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
    return (controller, transport)
}

// MARK: - Suite

@Suite("TmuxSessionController state machine", .serialized)
@MainActor
struct TmuxSessionControllerTests {

    // ─────────────────────────────────────────────────────────────
    // attach() / initial discovery
    // ─────────────────────────────────────────────────────────────

    @Test("attach() flips isAttached=true and dispatches list-sessions / list-windows / list-panes in order")
    func attachSendsDiscoveryCommands() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        await controller.attach()

        #expect(controller.snapshot.isAttached == true)

        // Wait for the three discovery writes to land — `send(rawCommand:)`
        // dispatches them via Task.detached so they may arrive asynchronously.
        let captured = await waitUntil { await transport.recordedCommands().count >= 3 }
        #expect(captured)

        let commands = await transport.recordedCommands()
        #expect(commands[0].hasPrefix("list-sessions"))
        #expect(commands[1].hasPrefix("list-windows"))
        #expect(commands[2].hasPrefix("list-panes"))
    }

    @Test("detach() cancels pumping, resets parser, clears isAttached")
    func detachShutsEverythingDown() async throws {
        let (controller, transport) = await makeAttachedController()
        // Drain the discovery commands so we observe the pre-detach baseline.
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }

        await controller.detach()
        transport.finish()

        #expect(controller.snapshot.isAttached == false)
    }

    // ─────────────────────────────────────────────────────────────
    // Parsing list-* responses
    // ─────────────────────────────────────────────────────────────

    @Test("list-sessions response populates snapshot.sessions and picks the attached session")
    func listSessionsResponse() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 1 }

        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        $1 0 alt
        %end 1 1 0

        """)

        let ok = await waitUntil { controller.snapshot.sessions.count == 2 }
        #expect(ok, "expected two sessions")
        #expect(controller.snapshot.sessions["$0"]?.isAttached == true)
        #expect(controller.snapshot.sessions["$1"]?.isAttached == false)
        #expect(controller.snapshot.activeSessionId == "$0")
    }

    @Test("list-windows response populates windows + activeWindowId from the active flag")
    func listWindowsResponse() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 2 }

        // First reply consumes the list-sessions callback (empty).
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        $0 @1 1 81x24,0,0,1 0 1 logs
        %end 2 2 0

        """)

        let ok = await waitUntil { controller.snapshot.windows.count == 2 }
        #expect(ok)
        #expect(controller.snapshot.windows["@0"]?.name == "main")
        #expect(controller.snapshot.windows["@0"]?.paneCount == 1)
        #expect(controller.snapshot.activeWindowId == "@0")
    }

    @Test("list-panes response builds PaneInfo entries and picks the active pane")
    func listPanesResponse() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }

        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 2 main
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %1 @0 1 80 24 0 vim
        %end 3 3 0

        """)

        let ok = await waitUntil { controller.snapshot.panes.count == 2 }
        #expect(ok)
        #expect(controller.snapshot.panes["%0"]?.command == "bash")
        #expect(controller.snapshot.panes["%0"]?.isActive == true)
        #expect(controller.snapshot.activePaneId == "%0")
    }

    // ─────────────────────────────────────────────────────────────
    // Window-size pin: release on Home, re-pin on return
    // ─────────────────────────────────────────────────────────────

    @Test("releaseWindowPins leaves sizing math (ignore-size) and unsets pinned windows; repin reverses both")
    func releaseAndRepinWindowPins() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }

        // Drain the 3 discovery callbacks (list-sessions / -windows / -panes) so
        // activeWindowId resolves to @0 and the FIFO response queue is empty.
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %end 3 3 0

        """)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })

        // A client resize pins @0 to our phone grid (70×35).
        controller.resizeClient(rows: 35, cols: 70)
        #expect(await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("resize-window -t @0 -x 70 -y 35") }
        }, "resizeClient should pin the active window to the client size")

        // Backgrounding: leave the sizing math + return the window to automatic.
        // Must NOT use `resize-window -A` (that re-pins at "largest session",
        // still manual — the desktop stays stuck).
        controller.releaseWindowPins()
        #expect(await waitUntil {
            let cmds = await transport.recordedCommands()
            return cmds.contains { $0.hasPrefix("refresh-client -f ignore-size") }
                && cmds.contains { $0.hasPrefix("set-option -u -w -t @0 window-size") }
        }, "release must set ignore-size and unset the per-window size override")
        #expect(!(await transport.recordedCommands().contains { $0.contains(" -A") }),
                "release must not use resize-window -A (it re-pins, still manual)")

        // Foregrounding: rejoin sizing, re-assert our size, re-pin the grid.
        controller.repinActiveWindow()
        #expect(await waitUntil {
            let cmds = await transport.recordedCommands()
            return cmds.contains { $0.hasPrefix("refresh-client -f !ignore-size") }
                && cmds.filter { $0.hasPrefix("resize-window -t @0 -x 70 -y 35") }.count >= 2
        }, "repin must clear ignore-size and restore the phone-grid pin")
    }

    @Test("releaseWindowPins without pinned windows still leaves sizing math, but unsets nothing")
    func releaseWithoutPinsOnlyIgnoresSize() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }

        controller.releaseWindowPins()   // no pins — but our size must stop dragging `latest`

        #expect(await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("refresh-client -f ignore-size") }
        }, "even with no pins, the client must leave the sizing math (the 2s poll re-wins latest)")
        try? await Task.sleep(for: .milliseconds(40))
        #expect(!(await transport.recordedCommands().contains { $0.hasPrefix("set-option -u -w") }),
                "nothing was pinned, so no per-window override should be unset")
    }

    // ─────────────────────────────────────────────────────────────
    // Resize debounce + resync-from-tmux (the IME/keyboard garble fix)
    // ─────────────────────────────────────────────────────────────

    /// Answer every control command written so far — and everything those
    /// replies go on to enqueue — with a harmless block, until the chatter
    /// stops. tmux replies to EVERY command, so blanket-answering is exactly
    /// what a real server does and it keeps the response FIFO in sync.
    ///
    /// Needed because the attach-time repaint is a CHAIN, not a batch: the
    /// backfill's `alternate_on` probe enqueues its scrollback capture only
    /// when the probe's reply lands, and THAT reply is what releases the
    /// resync parked behind the dump (see `backfillsInFlight` — the resync
    /// waits so the authoritative frame isn't scrolled off by 2 000 lines of
    /// history). No fixed number of pushes can drain that.
    ///
    /// `answered` is how many replies the caller has already pushed itself;
    /// the return value lets a later call resume from there.
    @discardableResult
    private func settleControlChatter(_ transport: MockTmuxTransport,
                                      answered alreadyAnswered: Int) async -> Int {
        var answered = alreadyAnswered
        for _ in 0..<20 {
            // Only ever answer commands that have actually been written —
            // a reply with no slot waiting for it desyncs the FIFO.
            let sent = await transport.recordedCommands().count
            while answered < sent {
                transport.pushText("%begin 900 900 0\n0 0\n%end 900 900 0\n\n")
                answered += 1
            }
            try? await Task.sleep(for: .milliseconds(20))
            if await transport.recordedCommands().count == answered { break }
        }
        return answered
    }

    /// Answer every command recorded past `answered`, in FIFO order, using
    /// `reply(command)` to build each response block. Returns the new count.
    private func answerPending(_ transport: MockTmuxTransport, answered: Int,
                               _ reply: (String, Int) -> String) async -> Int {
        var answered = answered
        let cmds = await transport.recordedCommands()
        while answered < cmds.count {
            transport.pushText(reply(cmds[answered], answered))
            answered += 1
        }
        return answered
    }

    private func block(_ num: Int, _ body: String) -> String {
        body.isEmpty
            ? "%begin \(num) \(num) 0\n%end \(num) \(num) 0\n\n"
            : "%begin \(num) \(num) 0\n\(body)\n%end \(num) \(num) 0\n\n"
    }

    /// Discovery replies that resolve one session/window/pane (@0/%0 active).
    private func pushOneWindowDiscovery(_ transport: MockTmuxTransport) {
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %end 3 3 0

        """)
    }

    @Test("resizeClient debounces a resize storm to one refresh-client at the final size")
    func resizeStormCoalesces() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })

        // Keyboard/IME animation: three sizes in quick succession.
        controller.resizeClient(rows: 40, cols: 90)
        controller.resizeClient(rows: 37, cols: 80)
        controller.resizeClient(rows: 35, cols: 70)

        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().contains { $0.hasPrefix("refresh-client -C 70x35") }
        }, "the final size must be committed after the debounce")
        let cmds = await transport.recordedCommands()
        #expect(!cmds.contains { $0.hasPrefix("refresh-client -C 90x40") },
                "intermediate sizes must be coalesced away")
        #expect(!cmds.contains { $0.hasPrefix("refresh-client -C 80x37") },
                "intermediate sizes must be coalesced away")
    }

    @Test("output arriving while the window pin is handed back is not painted, but its bell still is")
    func outputWhilePinReleasedIsDropped() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        await settleControlChatter(transport, answered: 3)

        let terminal = controller.terminalView(for: "%0").getTerminal()
        transport.pushText("%output %0 abc\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 3 },
                "output must paint while the pin is ours")

        // Off screen / backgrounded: tmux has handed the window to whatever else
        // is attached, so these bytes are laid out for someone else's width.
        controller.releaseWindowPins()

        @MainActor final class Bells { var count = 0 }
        let bells = Bells()
        controller.onPaneBell = { _ in bells.count += 1 }

        transport.pushText("%output %0 defghij\n")
        try? await Task.sleep(for: .milliseconds(150))
        #expect(terminal.getCursorLocation().x == 3,
                "a released pin means these bytes must not reach the grid")

        // The bell has to survive: it's the agent-needs-you signal, and the
        // parser that normally raises it is exactly what's being skipped.
        transport.pushText("%output %0 \\007\n")   // tmux escapes BEL octally
        #expect(await waitUntil { bells.count == 1 }, "a bell must still get through")
        #expect(terminal.getCursorLocation().x == 3)
    }

    @Test("coming back to the foreground re-pins AND repaints, not just re-pins")
    func repinRepaintsTheActivePane() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })
        // Answer the fresh attach's backfill probe + dump so nothing is left in
        // flight to park the repaint behind (see `backfillsInFlight`). tmux
        // answers every command in send order, so the FIFO here is: the probe,
        // the window pin (no callback, but it still takes a reply slot), then
        // the dump the probe's reply enqueued.
        transport.pushText("""
        %begin 4 4 0
        0 0
        %end 4 4 0
        %begin 5 5 0
        %end 5 5 0
        %begin 6 6 0
        hello
        %end 6 6 0

        """)
        _ = await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("capture-pane -p -e -S") }
        }
        let before = await transport.recordedCommands().count

        // `%output` kept flowing while backgrounded, laid out for whatever width
        // the window took once we handed it back — so returning has to repaint,
        // not just re-pin, or the stale mis-wrapped frame is what's on screen.
        controller.repinActiveWindow()

        #expect(await waitUntil(timeout: 2.0) {
            let cmds = await transport.recordedCommands()
            guard cmds.count > before else { return false }
            let after = cmds[before...]
            return after.contains { $0.hasPrefix("resize-window -t @0") }
                && after.contains { $0.hasPrefix("capture-pane -p -e -t %0") }
        }, "the re-pin must be followed by a repaint of the active pane")
    }

    @Test("a fresh attach pins the active window to the client grid without waiting for a size report")
    func freshAttachPinsTheWindow() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        // Real tmux answers the boot line with the connection's first reply
        // block; the client swallows it (see makeAttachedController).
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)

        // No resizeClient() here on purpose: `window-size latest` leaves the
        // window at whatever an already-attached desktop client made it, and
        // nothing else claims it on a fresh attach — the pane's program would
        // render to that width while this narrower client hard-wraps every line.
        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().contains {
                $0.hasPrefix("resize-window -t @0 -x 69 -y 60")
            }
        }, "discovery must pin the active window to the phone grid on its own")
    }

    @Test("the terminal view's FIRST size report commits even when it matches the seeded estimate")
    func firstSizeReportCommitsEvenWhenItMatchesTheEstimate() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        // Pre-attach seed: a GUESS at the phone grid, not a view's report.
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        // Real tmux answers the boot line with the connection's first reply
        // block; the client swallows it (see makeAttachedController).
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })

        // The view finishes layout and reports EXACTLY the guessed grid. This
        // has to still commit: the commit is what re-paints the pane from
        // tmux's model, and tmux never repaints on its own for a size it
        // already has. Treating it as "nothing changed" left a pane painted
        // before layout (i.e. at the wrong grid) stranded until the user
        // resized the app themselves by raising the keyboard.
        controller.resizeClient(rows: 60, cols: 69)

        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().contains { $0.hasPrefix("refresh-client -C 69x60") }
        }, "the first report from a real view must commit, estimate or not")
    }

    @Test("a repeat of an already-confirmed size is still deduped")
    func confirmedSizeStillDedupes() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        // Real tmux answers the boot line with the connection's first reply
        // block; the client swallows it (see makeAttachedController).
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })

        controller.resizeClient(rows: 60, cols: 69)   // confirms
        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().contains { $0.hasPrefix("refresh-client -C 69x60") }
        })
        controller.resizeClient(rows: 60, cols: 69)   // a no-op report
        controller.resizeClient(rows: 60, cols: 69)
        try? await Task.sleep(for: .milliseconds(400))

        let commits = await transport.recordedCommands()
            .filter { $0.hasPrefix("refresh-client -C 69x60") }
        #expect(commits.count == 1,
                "only the first report is a confirmation; the rest are still redundant")
    }

    @Test("repeated foreign-width layout-change drift backs off after a few reclaims (tug-of-war guard)")
    func layoutDriftReclaimBacksOffDuringTugOfWar() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })

        // Pin the phone grid first, as attach normally does — this sets
        // `lastClientSize` so a differently-sized layout-change reads as drift.
        controller.resizeClient(rows: 35, cols: 70)
        #expect(await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().contains { $0.hasPrefix("resize-window -t @0 -x 70 -y 35") }
        })
        let baseline = await transport.recordedCommands().count

        // A live peer client keeps re-widening the SAME window (e.g. tmux
        // `window-size latest` following its own activity) — four drift
        // notifications in a row, faster than any real reclaim round trip.
        transport.pushText("""
        %layout-change @0 200x24,0,0,2
        %layout-change @0 200x24,0,0,2
        %layout-change @0 200x24,0,0,2
        %layout-change @0 200x24,0,0,2

        """)
        try? await Task.sleep(for: .milliseconds(150))

        let reclaims = await transport.recordedCommands()
            .dropFirst(baseline)
            .filter { $0.hasPrefix("resize-window -t @0 -x 70 -y 35") }
        #expect(reclaims.count == 2,
                "only the first two drift events reclaim; the third trips the backoff and the fourth is suppressed by it")
    }

    @Test("a settled resize resyncs the active pane from tmux: capture the frame, then the cursor")
    func settledResizeResyncsFromTmux() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        // The fresh attach fires its own backfill + resync for %0 — settle it
        // so the indices below belong to THIS test's resize.
        await settleControlChatter(transport, answered: 3)
        let baseline = await transport.recordedCommands().count

        controller.resizeClient(rows: 35, cols: 70)

        // Frame BEFORE cursor: %output arriving between the two replies is then
        // applied on top of the fresh frame AND already reflected in the cursor
        // reply — the final CUP is exact (the cursor-first order restored a
        // pre-redraw cursor: the "cursor drifts after an IME switch" bug).
        #expect(await waitUntil(timeout: 2.0) {
            let cmds = await transport.recordedCommands().dropFirst(baseline)
            guard let capture = cmds.firstIndex(where: { $0.hasPrefix("capture-pane -p -e -t %0") }),
                  let cursor = cmds.firstIndex(where: {
                      $0.hasPrefix("display-message -p -t %0 '#{cursor_x}") })
            else { return false }
            return capture < cursor
        }, "resync must capture the frame, then query the cursor, in stream order")
    }

    @Test("resyncActivePane(reveal: false) corrects the buffer without lifting an active cover; the default reveals")
    func resyncRevealParameterGatesCoverLift() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })

        // A fresh attach ALSO auto-veils + resyncs the active pane (see
        // ensureTerminalsForAllPanes(isFreshAttach:)), chained behind the
        // terminal's own backfill: probe → scrollback capture → the parked
        // resync's frame → its cursor query. Settle the whole chain so the
        // FIFO is clean before this test's own veilForSwitch/resyncActivePane
        // calls — the resync's frame reply also lifts the automatic cover via
        // its default `reveal: true`, same as after any completed fresh
        // attach, so the pane is uncovered by the time we get there.
        await settleControlChatter(transport, answered: 3)
        #expect(await waitUntil { controller.activeCoordinator?.isCoverPresented == false },
                "the auto-resync's own frame reply must reveal the fresh-attach veil before this test begins")

        // veilForSwitch needs no real window/rendering (unlike
        // freezeForKeyboardTransition's UIKit snapshot) — a test-friendly
        // way to put a cover up and observe when it comes down.
        controller.activeCoordinator?.veilForSwitch()
        #expect(controller.activeCoordinator?.isCoverPresented == true)

        let before = await transport.recordedCommands().count
        controller.resyncActivePane(reveal: false)
        _ = await waitUntil { await transport.recordedCommands().count > before }
        transport.pushText("""
        %begin 100 100 0
        resynced without reveal
        %end 100 100 0
        %begin 101 101 0
        0 0
        %end 101 101 0

        """)
        try? await Task.sleep(for: .milliseconds(200))
        #expect(controller.activeCoordinator?.isCoverPresented == true, """
                reveal: false must correct the buffer without lifting the cover — this is the fix for the \
                keyboard show/hide race where an immediate, possibly mid-redraw capture used to lift the \
                cover and flash a duplicated/blank frame before the settled capture corrected it
                """)

        controller.resyncActivePane()   // default reveal: true
        transport.pushText("""
        %begin 102 102 0
        resynced and revealed
        %end 102 102 0
        %begin 103 103 0
        0 0
        %end 103 103 0

        """)
        #expect(await waitUntil { controller.activeCoordinator?.isCoverPresented == false },
                "the default reveal must lift the cover once the frame lands")
    }

    @Test("extendCoverTimeout no-ops without a cover, and reschedules the reveal deadline with one")
    func extendCoverTimeoutBehavior() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        let coordinator = controller.activeCoordinator

        // A fresh attach auto-veils + resyncs the active pane (see
        // ensureTerminalsForAllPanes(isFreshAttach:)) — settle that chain
        // (backfill's probe, backfill's own capture, then the resync parked
        // behind the dump) so the auto-cover is lifted by the resync's own
        // frame reply before this test's assertions, which are about
        // extendCoverTimeout's behavior in isolation, not the fresh-attach
        // veil.
        await settleControlChatter(transport, answered: 3)
        #expect(await waitUntil { coordinator?.isCoverPresented == false },
                "the fresh-attach auto-veil must be lifted by its own resync frame reply before this test begins")

        coordinator?.extendCoverTimeout(by: 0.05)
        #expect(coordinator?.isCoverPresented == false,
                "must not itself start a freeze that wasn't already requested")

        coordinator?.veilForSwitch()   // installs a cover with the default 0.8s safety timeout
        #expect(coordinator?.isCoverPresented == true)
        coordinator?.extendCoverTimeout(by: 0.05)   // reschedule far sooner than the 0.8s default

        try? await Task.sleep(for: .milliseconds(20))
        #expect(coordinator?.isCoverPresented == true, "the extended deadline shouldn't have fired yet")

        #expect(await waitUntil(timeout: 0.5) { coordinator?.isCoverPresented == false },
                "the extended deadline should fire and reveal, well before the original 0.8s default would have")
    }

    @Test("selectWindow resyncs the target pane from tmux after activating it")
    func selectWindowResyncsTarget() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        $0 @1 1 81x24,0,0,1 0 1 work
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %1 @1 0 80 24 1 vim
        %end 3 3 0

        """)
        #expect(await waitUntil { controller.snapshot.windows.count == 2 })
        // Both panes get an attach-time backfill; settle them (and the fresh
        // attach's own resync of %0) so %1's resync below isn't parked behind
        // an unanswered dump.
        await settleControlChatter(transport, answered: 3)

        controller.selectWindow("@1")

        #expect(await waitUntil {
            let cmds = await transport.recordedCommands()
            guard let select = cmds.firstIndex(where: { $0.hasPrefix("select-window -t @1") }),
                  let capture = cmds.firstIndex(where: { $0.hasPrefix("capture-pane -p -e -t %1") })
            else { return false }
            return select < capture
        }, "the switched-to pane must be repainted from tmux's model after activation")
    }

    // ─────────────────────────────────────────────────────────────
    // Weak-network protocol hardening (2026-08-19 garble evidence)
    // ─────────────────────────────────────────────────────────────

    @Test("a late boot banner must not shift command↔response pairing")
    func lateBootBannerKeepsPairingAligned() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        await controller.attach()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }

        // The boot line's own reply block arrives AFTER discovery already
        // enqueued — routine over a weak network. It must pop the reserved
        // boot slot, not list-sessions' callback: when it popped the wrong
        // slot, every later response landed one command back — capture
        // frames delivered to cursor callbacks, and the agent-hook poll's
        // "%0||||" payload painted into a visible pane as its "frame".
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %end 3 3 0

        """)

        #expect(await waitUntil { controller.snapshot.sessions["$0"] != nil },
                "list-sessions' reply must reach list-sessions' callback")
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
    }

    @Test("%pause resumes with tmux's real verb, and %continue repaints the paused pane")
    func pauseUsesContinueVerbAndResyncs() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        await settleControlChatter(transport, answered: 3)

        transport.pushText("%pause %0\n")
        #expect(await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("refresh-client -A %0:continue") }
        }, """
        the resume verb must be in tmux's vocabulary (on/off/continue/pause) — the old `:+` \
        earned an %error and the pane stayed paused, output stopped for good
        """)

        let before = await transport.recordedCommands()
            .filter { $0.hasPrefix("capture-pane -p -e -t %0") }.count
        transport.pushText("%continue %0\n")
        #expect(await waitUntil {
            await transport.recordedCommands()
                .filter { $0.hasPrefix("capture-pane -p -e -t %0") }.count > before
        }, "tmux dropped output while paused — %continue must trigger a repaint from its model")
    }

    @Test("output for a foreign-width window is gated (bell-only) until our width returns, then resynced")
    func foreignWidthOutputGated() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        await settleControlChatter(transport, answered: 3)

        let terminal = controller.terminalView(for: "%0").getTerminal()
        transport.pushText("%output %0 abc\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 3 })

        // A desktop client just won `window-size latest`: the window flips to
        // its grid. Everything the pane now emits is laid out 499 wide —
        // painting it into this 69-column terminal is the shredded-fragments
        // garble (phone byte capture, 2026-08-19).
        transport.pushText("%layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\n")
        transport.pushText("%output %0 defghij\n")
        try? await Task.sleep(for: .milliseconds(150))
        #expect(terminal.getCursorLocation().x == 3,
                "foreign-width output must not reach the grid")

        transport.pushText("%output %0 \\007\n")
        @MainActor final class Bells { var count = 0 }
        let bells = Bells()
        controller.onPaneBell = { _ in bells.count += 1 }
        transport.pushText("%output %0 \\007\n")
        #expect(await waitUntil { bells.count >= 1 },
                "the agent-needs-you bell must survive the gate")

        // Our reclaim (or the desktop detaching) brings the width back:
        // the gate lifts and the gap is repainted from tmux's model.
        let before = await transport.recordedCommands()
            .filter { $0.hasPrefix("capture-pane -p -e -t %0") }.count
        transport.pushText("%layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\n")
        #expect(await waitUntil {
            await transport.recordedCommands()
                .filter { $0.hasPrefix("capture-pane -p -e -t %0") }.count > before
        }, "returning to our width must trigger the gap repaint")
    }

    @Test("a resync frame captured at a foreign size is rejected, not painted")
    func foreignSizeFrameRejected() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        await settleControlChatter(transport, answered: 3)

        let terminal = controller.terminalView(for: "%0").getTerminal()
        transport.pushText("%output %0 sentinel\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 8 })

        // Resync races a desktop-size flap: the capture comes back 70 rows
        // tall — authoritative for a grid this terminal doesn't have.
        let before = await transport.recordedCommands().count
        controller.resyncActivePane()
        _ = await waitUntil { await transport.recordedCommands().count > before }
        var reply = "%begin 200 200 0\n"
        for i in 1...70 { reply += "wide-frame-row-\(i)\n" }
        reply += "%end 200 200 0\n%begin 201 201 0\n0 0\n%end 201 201 0\n\n"
        transport.pushText(reply)
        try? await Task.sleep(for: .milliseconds(200))
        // The paired cursor reply still applies (a bare CUP — harmless); the
        // CONTENT is what must survive untouched.
        let row0 = terminal.getText(start: Position(col: 0, row: 0),
                                    end: Position(col: 20, row: 0))
        #expect(row0.contains("sentinel"), """
                the oversized frame must be dropped — feeding a 70-row/499-column capture into a \
                60-row/69-column grid wraps every line sevenfold (the 2026-08-19 garble); got: \(row0)
                """)
        #expect(await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("resize-window -t @0 -x 69") }
        }, "the reject must re-pin the window so a good capture can follow")
    }

    @Test("after a foreign-width cycle, output stays gated until the repair frame lands — a split sequence's tail must never paint")
    func gateHoldsUntilRepairFrame() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        // Mint the terminal FIRST so its backfill probe + dump are among the
        // chatter the settle loop answers — the FIFO is then quiet.
        let terminal = controller.terminalView(for: "%0").getTerminal()
        var answered = await settleControlChatter(transport, answered: 3)

        transport.pushText("%output %0 abc\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 3 })

        // Desktop wins the size; the pane app repaints 499 wide, its bytes
        // split MID escape sequence across %output events (real chunking —
        // phone byte capture, 2026-08-19). The head is gated with the rest.
        transport.pushText("%layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\n")
        transport.pushText("%output %0 WIDE-REPAINT\\033[38;\n")
        // Our re-pin brings the width back — but the app's in-flight bytes
        // straggle in AFTER the flip, starting with the TAIL of the split
        // sequence. The gate must hold until the repair frame, or that tail
        // paints as literal text.
        transport.pushText("%layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\n")
        transport.pushText("%output %0 5;210mTAIL-GARBAGE\n")
        try? await Task.sleep(for: .milliseconds(150))
        #expect(terminal.getCursorLocation().x == 3,
                "output between the width's return and the repair frame must stay gated")

        // The agent-needs-you bell still rings through the held gate.
        @MainActor final class Bells { var count = 0 }
        let bells = Bells()
        controller.onPaneBell = { _ in bells.count += 1 }
        transport.pushText("%output %0 \\007\n")
        #expect(await waitUntil { bells.count >= 1 },
                "the bell must survive the held gate")

        // Answer the FIFO: the reclaim's resize-window, then the settling
        // pass's capture (the repair frame) and cursor probe.
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(answered)
                .contains { $0.hasPrefix("capture-pane") }
        }, "the return to our width must dispatch a repair capture")
        answered = await answerPending(transport, answered: answered) { cmd, n in
            if cmd.hasPrefix("capture-pane") { return block(300 + n, "REPAIRED-ROW") }
            if cmd.hasPrefix("display-message") { return block(300 + n, "0 0") }
            return block(300 + n, "")
        }
        #expect(await waitUntil {
            let row0 = terminal.getText(start: Position(col: 0, row: 0),
                                        end: Position(col: 12, row: 0))
            return row0.contains("REPAIRED-ROW")
        }, "the repair frame must paint")

        // Frame landed → the gate reopens: live output flows again.
        transport.pushText("%output %0 live-again\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 10 },
                "output after the repair frame must paint")
        let head = terminal.getText(start: Position(col: 0, row: 0),
                                    end: Position(col: 68, row: 2))
        #expect(!head.contains("5;210m"),
                "the split-sequence tail painted as literal text: \(head)")
        #expect(!head.contains("TAIL-GARBAGE"),
                "straggling foreign bytes painted before the repair frame: \(head)")
    }

    @Test("a rejected repair frame is recaptured until one lands at our size")
    func rejectedFrameIsRetried() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        let terminal = controller.terminalView(for: "%0").getTerminal()
        var answered = await settleControlChatter(transport, answered: 3)

        transport.pushText("%output %0 sentinel\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 8 })

        // One full war cycle drives the repair machinery.
        transport.pushText("%layout-change @0 c71d,499x62,0,0,0 c71d,499x62,0,0,0 *\n")
        transport.pushText("%layout-change @0 c71d,69x60,0,0,0 c71d,69x60,0,0,0 *\n")

        // Every capture comes back mid-flap (70 rows tall) TWICE — the exact
        // storm that starved the old reject-without-retry: its only shots
        // were the two settling passes. The third capture gets a good frame;
        // pre-fix, no third capture is ever sent and the repair never lands.
        var oversized = 0
        let deadline = Date().addingTimeInterval(4.0)
        var repaired = false
        while Date() < deadline, !repaired {
            answered = await answerPending(transport, answered: answered) { cmd, n in
                if cmd.hasPrefix("capture-pane") {
                    if oversized < 2 {
                        oversized += 1
                        let rows = (1...70).map { "flap-row-\($0)" }.joined(separator: "\n")
                        return block(400 + n, rows)
                    }
                    return block(400 + n, "REPAIRED-AFTER-RETRY")
                }
                if cmd.hasPrefix("display-message") { return block(400 + n, "0 0") }
                return block(400 + n, "")
            }
            let row0 = terminal.getText(start: Position(col: 0, row: 0),
                                        end: Position(col: 24, row: 0))
            repaired = row0.contains("REPAIRED-AFTER-RETRY")
            try? await Task.sleep(for: .milliseconds(40))
        }
        #expect(repaired, """
                two rejected captures must not starve the repair — the reject \
                path has to keep recapturing until a frame lands at our size
                """)
    }

    @Test("a resync frame with foreign-WIDTH lines is rejected even when its row count fits")
    func foreignWidthLinesRejected() async throws {
        let transport = MockTmuxTransport()
        let controller = TmuxSessionController(sshSession: transport)
        controller.setInitialClientSize(cols: 69, rows: 60)
        await controller.attach()
        transport.pushText("%begin 100 0 0\n%end 100 0 0\n\n")
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        let terminal = controller.terminalView(for: "%0").getTerminal()
        _ = await settleControlChatter(transport, answered: 3)

        transport.pushText("%output %0 sentinel\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 8 })

        // A desktop-width frame whose bottom rows were blank: the trim leaves
        // 30 rows (fits!) of 200-column lines — the rows-check's blind spot.
        let before = await transport.recordedCommands().count
        controller.resyncActivePane()
        _ = await waitUntil { await transport.recordedCommands().count > before }
        let wide = String(repeating: "W", count: 200)
        var reply = "%begin 200 200 0\n"
        for _ in 1...30 { reply += wide + "\n" }
        reply += "%end 200 200 0\n%begin 201 201 0\n0 0\n%end 201 201 0\n\n"
        transport.pushText(reply)
        try? await Task.sleep(for: .milliseconds(200))
        let row0 = terminal.getText(start: Position(col: 0, row: 0),
                                    end: Position(col: 20, row: 0))
        #expect(row0.contains("sentinel"), """
                a 200-column line can only come from a foreign-size capture — \
                feeding it into a 69-column grid wraps every line threefold; got: \(row0)
                """)
    }

    @Test("selectWindow runs a two-pass resync: an immediate silent capture plus a settled one")
    func selectWindowRunsSettlingResync() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        $0 @1 1 81x24,0,0,1 0 1 work
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %1 @1 0 80 24 1 vim
        %end 3 3 0

        """)
        #expect(await waitUntil { controller.snapshot.windows.count == 2 })
        await settleControlChatter(transport, answered: 3)

        controller.selectWindow("@1")

        #expect(await waitUntil {
            await transport.recordedCommands()
                .filter { $0.hasPrefix("capture-pane -p -e -t %1") }.count >= 1
        }, "the immediate pass must capture right away")
        #expect(await waitUntil(timeout: 2.5) {
            await transport.recordedCommands()
                .filter { $0.hasPrefix("capture-pane -p -e -t %1") }.count >= 2
        }, """
        the settled pass (~700ms) must follow — the switch's resize-window triggers the target \
        app's SIGWINCH repaint, the immediate capture races it, and without a second capture the \
        mid-redraw frame stood for seconds over a weak network
        """)
    }

    @Test("backfill cancels a stale copy-mode left over from a dead connection")
    func backfillCancelsStaleCopyMode() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })

        // Minting %0 triggers backfill, whose first query now also carries
        // #{pane_in_mode}. Answer "0 1" (primary screen, IN copy-mode — the
        // stale leftover) on every pending FIFO slot; fire-and-forget slots
        // ignore it, the backfill callback reads it.
        #expect(await waitUntil {
            await transport.recordedCommands().contains {
                $0.hasPrefix("display-message -p -t %0 '#{alternate_on} #{pane_in_mode}'") }
        }, "backfill must query alternate_on + pane_in_mode together")
        for _ in 0..<8 {
            transport.pushText("%begin 9 9 0\n0 1\n%end 9 9 0\n\n")
        }

        #expect(await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("send-keys -t %0 -X cancel") }
        }, "a pane still in copy-mode on first sight after attach must be cancelled — the 'reconnect scrolls to the top' bug")
    }

    @Test("a resync requested during a pane's backfill is sent AFTER the scrollback dump, not before it")
    func resyncWaitsForTheBackfillDump() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })

        // The fresh attach asks for BOTH a backfill and a repaint of %0. The
        // backfill's probe goes out at once; the repaint must not.
        #expect(await waitUntil {
            await transport.recordedCommands().contains {
                $0.hasPrefix("display-message -p -t %0 '#{alternate_on}")
            }
        })
        let racedAhead = await transport.recordedCommands()
            .contains { $0.hasPrefix("capture-pane -p -e -t %0") }
        #expect(racedAhead == false, """
                the resync must not go out while the backfill is still in flight: the dump's own \
                capture is only enqueued from the probe's reply, so it would land LAST and scroll \
                the authoritative frame away — leaving the pane showing the tail of history (the \
                last thing typed before the drop) after every reconnect
                """)

        await settleControlChatter(transport, answered: 3)

        let cmds = await transport.recordedCommands()
        let dump = try #require(cmds.firstIndex { $0.hasPrefix("capture-pane -p -e -S") },
                                "backfill must have captured the scrollback")
        let frame = try #require(cmds.firstIndex { $0.hasPrefix("capture-pane -p -e -t %0") },
                                 "the parked resync must have been released by the dump's reply")
        #expect(dump < frame, "the authoritative frame has to be fed last, on top of the history")
    }

    @Test("newWindow lands ON the window it just created")
    func newWindowSelectsTheNewWindow() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activeWindowId == "@0" })
        await settleControlChatter(transport, answered: 3)
        let baseline = await transport.recordedCommands().count

        controller.newWindow(named: "logs")

        #expect(await waitUntil {
            await transport.recordedCommands().contains {
                $0.hasPrefix("new-window -P -F '#{window_id}' -n 'logs'")
            }
        }, "new-window must print the new window's id so we can land on it")

        // Reply in send order: the printed id, then the re-list that gives
        // selectWindow a snapshot to work with.
        transport.pushText("""
        %begin 20 20 0
        @1
        %end 20 20 0
        %begin 21 21 0
        $0 @0 0 81x24,0,0,0 0 1 main
        $0 @1 1 81x24,0,0,1 1 1 logs
        %end 21 21 0
        %begin 22 22 0
        %0 @0 0 80 24 1 bash
        %1 @1 0 80 24 1 bash
        %end 22 22 0

        """)

        #expect(await waitUntil { controller.snapshot.activeWindowId == "@1" }, """
                creating a window must move the view to it — discovery alone can't, because \
                parseListWindows deliberately refuses to follow tmux's current window while ours \
                is alive (that's what stops another client's switch from yanking our view)
                """)
        #expect(controller.snapshot.activePaneId == "%1",
                "and the pane shown must be the new window's, not the old window's")
        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(baseline)
                .contains { $0.hasPrefix("select-window -t @1") }
        }, "tmux must be told too, so the mosh-rendered client follows")
    }

    @Test("selectPane keeps the window zoomed while switching (select-pane -Z)")
    func selectPaneKeepsZoom() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        // One window, two panes — the single-pane presentation zooms one of
        // them; cycling must swap the zoomed pane in ONE layout change, not
        // unzoom → select → re-zoom (split flash on every swipe).
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 2 main
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %1 @0 1 80 24 0 vim
        %end 3 3 0

        """)
        #expect(await waitUntil { controller.snapshot.panes.count == 2 })

        controller.selectPane("%1")

        #expect(await waitUntil {
            await transport.recordedCommands().contains { $0.hasPrefix("select-pane -Z -t %1") }
        }, "pane switches must use select-pane -Z so the zoom never drops")
    }

    /// Drive `activePaneWantsMouse` true by answering the flag probe with "1".
    /// `settleControlChatter`'s generic "0 0" reply would leave it false.
    private func armMouseApp(_ controller: TmuxSessionController,
                             _ transport: MockTmuxTransport,
                             answered: Int) async -> Int {
        controller.refreshActivePaneMouse()
        var count = answered
        _ = await waitUntil { await transport.recordedCommands().count > count }
        transport.pushText("%begin 900 900 0\n1\n%end 900 900 0\n\n")
        count += 1
        _ = await waitUntil { controller.activePaneWantsMouse }
        return count
    }

    @Test("a swipe while the pane is streaming parks the LOCAL scrollback — never copy-mode, never the wheel")
    func scrollWhileStreamingParksLocally() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        let answered = await settleControlChatter(transport, answered: 3)
        _ = await armMouseApp(controller, transport, answered: answered)

        // Hold the pane "streaming" for the whole test rather than racing the
        // real 0.5s window, which a loaded full-suite run loses.
        controller.streamingWindow = 60

        // The agent is mid-answer; give the pane real scrollback so a local
        // park has history to show.
        let terminal = controller.terminalView(for: "%0").getTerminal()
        for chunk in 0..<6 {
            var blob = ""
            for i in 0..<20 { blob += "%output %0 stream-line-\(chunk * 20 + i)\\015\\012\n" }
            transport.pushText(blob)
        }
        transport.pushText("%output %0 thinking\n")
        #expect(await waitUntil { terminal.getCursorLocation().x == 8 },
                "the output must be processed before the swipe")

        let before = await transport.recordedCommands().count
        controller.scroll(lines: 3)
        try? await Task.sleep(for: .milliseconds(300))

        // Copy-mode is drawn only for regular tty clients — a -CC client
        // receives ZERO %output while it scrolls (tmux 3.6a, tmux-cc-lab
        // `copymode` scenario, 2026-08-19). Driving it from here scrolled an
        // invisible screen and hijacked any desktop client on the window.
        let sent = await transport.recordedCommands().dropFirst(before)
        #expect(!sent.contains { $0.hasPrefix("copy-mode") },
                "copy-mode must never be driven from the -CC renderer")
        #expect(!sent.contains { $0.contains("-X scroll-up") },
                "copy-mode paging must never be driven from the -CC renderer")
        #expect(!sent.contains { $0.contains("send-keys -t %0 -H") },
                "no wheel may be forwarded while the app is repainting")

        // The park holds live output away from the viewport…
        let xBefore = terminal.getCursorLocation().x
        transport.pushText("%output %0 while-scrolled\n")
        try? await Task.sleep(for: .milliseconds(150))
        #expect(terminal.getCursorLocation().x == xBefore,
                "output while parked must be held, not yank the viewport")

        // …and typing snaps back to live: the hold replays in order.
        controller.sendInput(Data("k".utf8), paneId: "%0")
        #expect(await waitUntil { terminal.getCursorLocation().x == xBefore + 14 },
                "the held output must replay when the user types")
    }

    @Test("a swipe on an idle mouse app still forwards the wheel")
    func scrollWhenIdleForwardsWheel() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })
        let answered = await settleControlChatter(transport, answered: 3)
        _ = await armMouseApp(controller, transport, answered: answered)

        // No %output has ever arrived for this pane, so it is not streaming.
        let before = await transport.recordedCommands().count
        controller.scroll(lines: 3)

        #expect(await waitUntil {
            await transport.recordedCommands().dropFirst(before)
                .contains { $0.contains("send-keys -t %0 -H") }
        }, "an idle mouse app keeps scrolling itself — copy-mode would break its own paging")
    }

    /// Interleaved %output and command replies must be handled in byte-stream
    /// order. The old wiring hopped each parser callback to the main actor in
    /// its own Task — Swift doesn't guarantee FIFO between independent tasks,
    /// so under load a capture-pane reply could be handled before %output that
    /// preceded it, smearing stale bytes over a resync frame. The single
    /// event-stream pipeline makes this deterministic.
    @Test("%output and command replies are handled in byte-stream order")
    func eventOrderingIsPreserved() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        pushOneWindowDiscovery(transport)
        #expect(await waitUntil { controller.snapshot.activePaneId == "%0" })

        @MainActor final class Recorder { var seq: [String] = [] }
        let recorder = Recorder()
        controller.onPaneActivity = { _ in recorder.seq.append("out") }
        controller.onAgentHooksUpdated = { recorder.seq.append("hooks") }

        // Empty the FIFO before queueing the polls, so the poll replies pair
        // 1:1 with the interleaved pushes below. A fresh attach leaves a whole
        // repaint CHAIN outstanding (backfill probe → its scrollback capture →
        // the resync parked behind that dump → its cursor query), each link
        // only enqueued when the previous reply lands, so this has to be
        // answer-until-quiet rather than a fixed push.
        await settleControlChatter(transport, answered: 3)

        // Queue 20 commands whose replies fire onAgentHooksUpdated…
        for _ in 0..<20 { controller.pollAgentHooks() }
        // …then interleave: %output, reply, %output, reply, … in ONE stream.
        for _ in 0..<20 {
            transport.pushText("%output %0 x\n%begin 9 9 0\n%end 9 9 0\n")
        }

        #expect(await waitUntil(timeout: 2.0) { recorder.seq.count == 40 })
        let expected = (0..<20).flatMap { _ in ["out", "hooks"] }
        #expect(recorder.seq == expected,
                "events must interleave exactly as pushed — any swap means the pipeline reordered")
    }

    @Test("selectWindow resizes the target to our width BEFORE select-window (no wide-render ？ on switch)")
    func selectWindowPreSizesBeforeActivating() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }

        // Two windows in the active session; @0 active, @1 idle.
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        $0 @1 1 81x24,0,0,1 0 1 work
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %1 @1 0 80 24 1 vim
        %end 3 3 0

        """)
        #expect(await waitUntil { controller.snapshot.windows.count == 2 })

        controller.resizeClient(rows: 35, cols: 70)   // establishes our width
        controller.selectWindow("@1")

        #expect(await waitUntil {
            let cmds = await transport.recordedCommands()
            guard let resize = cmds.firstIndex(where: { $0.hasPrefix("resize-window -t @1 -x 70") }),
                  let select = cmds.firstIndex(where: { $0.hasPrefix("select-window -t @1") })
            else { return false }
            return resize < select
        }, "the target window must be sized to our width before select-window streams its output")
    }

    // ─────────────────────────────────────────────────────────────
    // Terminal pool invariants (the Phase 3 architectural promise)
    // ─────────────────────────────────────────────────────────────

    @Test("terminalView(for:) returns the SAME instance across calls — the Phase 3 invariant")
    func terminalViewIsIdempotent() async throws {
        let (controller, _) = await makeAttachedController()
        let tv1 = controller.terminalView(for: "%0")
        let tv2 = controller.terminalView(for: "%0")
        let tv3 = controller.terminalView(for: "%0")
        #expect(tv1 === tv2)
        #expect(tv2 === tv3)
    }

    @Test("Different paneIds get different TerminalView instances")
    func differentPanesGetDifferentTerminals() async throws {
        let (controller, _) = await makeAttachedController()
        let a = controller.terminalView(for: "%0")
        let b = controller.terminalView(for: "%1")
        #expect(a !== b)
    }

    // ─────────────────────────────────────────────────────────────
    // %output routing
    // ─────────────────────────────────────────────────────────────

    @Test("%output for a pane that already exists feeds the existing coordinator (no terminal duplicated)")
    func outputUsesExistingTerminal() async throws {
        let (controller, transport) = await makeAttachedController()
        let preMint = controller.terminalView(for: "%0")

        transport.pushText("%output %0 hello\n")

        // Race: parser callback hops to MainActor. Give it a beat.
        _ = await waitUntil(timeout: 0.5) {
            controller.terminalView(for: "%0") === preMint
        }
        #expect(controller.terminalView(for: "%0") === preMint)
    }

    @Test("%output for a pane we have not yet seen mints a TerminalView on demand (no byte drop)")
    func outputBeforeListPanesMintsTerminal() async throws {
        let (controller, transport) = await makeAttachedController()

        transport.pushText("%output %42 surprise\n")

        let appeared = await waitUntil(timeout: 0.5) {
            controller.snapshot.panes["%42"] != nil
        }
        #expect(appeared, "%output before list-panes should still register a placeholder pane")
    }

    // ─────────────────────────────────────────────────────────────
    // Window lifecycle
    // ─────────────────────────────────────────────────────────────

    @Test("%window-add re-fires list-windows + list-panes to refresh the world")
    func windowAddTriggersRefetch() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        let baseline = await transport.recordedCommands().count

        transport.pushText("%window-add @9\n")

        // First confirm the parser callback actually fired on MainActor —
        // handleWindowAdd seeds a placeholder WindowInfo before sending the
        // refetch commands. If this assertion holds and the next one fails,
        // the issue is downstream in send/Task.detached, not in the parser.
        let snapshotted = await waitUntil(timeout: 2.0) {
            controller.snapshot.windows["@9"] != nil
        }
        #expect(snapshotted, "controller should record the new window in snapshot")

        let refetched = await waitUntil(timeout: 2.0) {
            await transport.recordedCommands().count >= baseline + 2
        }
        #expect(refetched, "expected two refetch commands (list-windows + list-panes)")
        let cmds = await transport.recordedCommands()
        let tail = Array(cmds.suffix(2))
        #expect(tail.contains(where: { $0.hasPrefix("list-windows") }))
        #expect(tail.contains(where: { $0.hasPrefix("list-panes") }))
    }

    @Test("%window-close cascades: window removed, panes removed, terminals freed, active window switched")
    func windowCloseCascadesCleanup() async throws {
        let (controller, transport) = await makeAttachedController()

        // Seed two windows + pane each via list-* responses.
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        $0 @1 1 81x24,0,0,1 0 1 logs
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %1 @1 0 80 24 0 tail
        %end 3 3 0

        """)

        _ = await waitUntil { controller.snapshot.panes.count == 2 }
        #expect(controller.snapshot.activeWindowId == "@0")

        transport.pushText("%window-close @0\n")

        let cascaded = await waitUntil(timeout: 0.5) {
            controller.snapshot.windows["@0"] == nil
                && controller.snapshot.panes["%0"] == nil
        }
        #expect(cascaded)
        #expect(controller.snapshot.activeWindowId == "@1")
    }

    @Test("%window-renamed mutates the existing window's name without resetting other fields")
    func windowRenamedMutatesInPlace() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 bash
        %end 3 3 0

        """)
        _ = await waitUntil { controller.snapshot.windows["@0"]?.name == "main" }

        transport.pushText("%window-renamed @0 edge\n")

        let renamed = await waitUntil { controller.snapshot.windows["@0"]?.name == "edge" }
        #expect(renamed)
        #expect(controller.snapshot.windows["@0"]?.index == 0,
                "rename should not reset the index")
    }

    @Test("%layout-change for an unknown window creates the window with that layout")
    func layoutChangeCreatesUnknownWindow() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText("%layout-change @99 81x24,0,0,77\n")

        let appeared = await waitUntil {
            controller.snapshot.windows["@99"]?.layout == "81x24,0,0,77"
        }
        #expect(appeared)
    }

    @Test("%session-changed updates activeSessionId and creates a session entry if needed")
    func sessionChangedCreatesEntry() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText("%session-changed $42 newone\n")

        let updated = await waitUntil {
            controller.snapshot.activeSessionId == "$42"
                && controller.snapshot.sessions["$42"]?.name == "newone"
        }
        #expect(updated)
    }

    @Test("%window-pane-changed updates snapshot.activePaneId for a window of OUR session")
    func windowPaneChangedUpdatesActivePane() async throws {
        let (controller, transport) = await makeAttachedController()
        // Seed: our session $0 owns window @0 (active) with pane %7.
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        %end 2 2 0
        %begin 3 3 0
        %7 @0 0 80 24 1 zsh
        %end 3 3 0

        """)
        _ = await waitUntil { controller.snapshot.windows["@0"] != nil }

        transport.pushText("%window-pane-changed @0 %7\n")
        let updated = await waitUntil { controller.snapshot.activePaneId == "%7" }
        #expect(updated)
    }

    @Test("%window-pane-changed for a FOREIGN window is ignored")
    func windowPaneChangedForeignWindowIgnored() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText("%window-pane-changed @77 %99\n")

        let updated = await waitUntil { controller.snapshot.activePaneId == "%99" }
        #expect(!updated, "pane focus in a window we don't know must not steal activePaneId")
    }

    @Test("%session-window-changed updates snapshot.activeWindowId for OUR session")
    func sessionWindowChangedUpdatesActiveWindow() async throws {
        let (controller, transport) = await makeAttachedController()
        // %session-changed establishes $0 as our active session first.
        transport.pushText("%session-changed $0 main\n")
        _ = await waitUntil { controller.snapshot.activeSessionId == "$0" }

        transport.pushText("%session-window-changed $0 @9\n")
        let updated = await waitUntil { controller.snapshot.activeWindowId == "@9" }
        #expect(updated)
    }

    @Test("%session-window-changed for ANOTHER session is ignored")
    func sessionWindowChangedForeignSessionIgnored() async throws {
        let (controller, transport) = await makeAttachedController()
        transport.pushText("%session-changed $0 main\n")
        _ = await waitUntil { controller.snapshot.activeSessionId == "$0" }

        // A different session ($5) switching windows must not move OUR active window.
        transport.pushText("%session-window-changed $5 @42\n")
        let moved = await waitUntil { controller.snapshot.activeWindowId == "@42" }
        #expect(!moved, "window change of a foreign session must be ignored")
    }

    // ─────────────────────────────────────────────────────────────
    // %pause auto-resume
    // ─────────────────────────────────────────────────────────────

    @Test("%pause %paneId triggers an outbound refresh-client -A request to resume the stream")
    func pauseAutoResumes() async throws {
        // Keep the controller alive — its parser callbacks capture `self`
        // weakly, so dropping the reference with `_` lets the callback fire
        // into a nil self and silently swallow the auto-resume.
        let (controller, transport) = await makeAttachedController()
        _ = controller  // keep alive across awaits
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        let baseline = await transport.recordedCommands().count

        transport.pushText("%pause %3\n")

        let resumed = await waitUntil(timeout: 2.0) {
            let cmds = await transport.recordedCommands()
            return cmds.count > baseline
                && cmds.contains(where: { $0.contains("refresh-client -A %3") })
        }
        #expect(resumed)
    }

    // ─────────────────────────────────────────────────────────────
    // Outbound input
    // ─────────────────────────────────────────────────────────────

    @Test("sendInput hex-encodes bytes via send-keys -H, preserving control bytes round-trip-safe")
    func sendInputHexEncodes() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        let baseline = await transport.recordedCommands().count

        // "a\nb" → 0x61 0x0A 0x62
        controller.sendInput(Data([0x61, 0x0A, 0x62]), paneId: "%0")

        let landed = await waitUntil(timeout: 0.5) {
            await transport.recordedCommands().count > baseline
        }
        #expect(landed)
        let cmds = await transport.recordedCommands()
        let sent = cmds.last ?? ""
        #expect(sent.hasPrefix("send-keys -t %0 -H "))
        #expect(sent.contains("61 0a 62"))
    }

    @Test("sendInput with empty data is a no-op (no write)")
    func sendInputEmptyIsNoOp() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        let baseline = await transport.recordedCommands().count

        controller.sendInput(Data(), paneId: "%0")

        try? await Task.sleep(for: .milliseconds(80))
        let after = await transport.recordedCommands().count
        #expect(after == baseline, "empty input must not produce a send-keys")
    }

    // ─────────────────────────────────────────────────────────────
    // selectWindow optimistic update
    // ─────────────────────────────────────────────────────────────

    @Test("selectWindow updates snapshot.activeWindowId immediately AND sends select-window to tmux")
    func selectWindowOptimisticAndCommand() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        let baseline = await transport.recordedCommands().count

        controller.selectWindow("@77")
        #expect(controller.snapshot.activeWindowId == "@77",
                "snapshot must update synchronously for the UI")

        let landed = await waitUntil(timeout: 0.5) {
            await transport.recordedCommands().count > baseline
        }
        #expect(landed)
        // selectWindow now also issues resize-window (fit to client) +
        // immersive-zoom queries, so assert the batch CONTAINS select-window
        // rather than checking only the last command.
        let cmds = await transport.recordedCommands()
        #expect(cmds.contains { $0.contains("select-window -t @77") })
    }

    @Test("selectWindow with the current activeWindowId still issues the command (idempotent UI)")
    func selectWindowSameTargetStillCommands() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        controller.selectWindow("@0")
        let baseline = await transport.recordedCommands().count
        controller.selectWindow("@0")
        let landed = await waitUntil(timeout: 0.5) {
            await transport.recordedCommands().count > baseline
        }
        #expect(landed)
    }

    // ─────────────────────────────────────────────────────────────
    // resizeClient
    // ─────────────────────────────────────────────────────────────

    @Test("resizeClient sends refresh-client -C <cols>x<rows>")
    func resizeClientSendsRefresh() async throws {
        let (controller, transport) = await makeAttachedController()
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        let baseline = await transport.recordedCommands().count

        controller.resizeClient(rows: 30, cols: 100)

        let landed = await waitUntil(timeout: 0.5) {
            await transport.recordedCommands().count > baseline
        }
        #expect(landed)
        // resizeClient also fits the active window, so check the batch
        // contains the refresh-client rather than only the last command.
        let cmds = await transport.recordedCommands()
        #expect(cmds.contains { $0.contains("refresh-client -C 100x30") })
    }

    // ─────────────────────────────────────────────────────────────
    // %exit / %client-detached
    // ─────────────────────────────────────────────────────────────

    @Test("%exit flips isAttached=false")
    func exitFlipsAttachedFalse() async throws {
        let (controller, transport) = await makeAttachedController()
        #expect(controller.snapshot.isAttached == true)

        transport.pushText("%exit detached\n")

        let detached = await waitUntil { controller.snapshot.isAttached == false }
        #expect(detached)
    }

    @Test("%client-detached is ignored — it's a server-wide broadcast about some client, not necessarily ours")
    func clientDetachedDoesNotFlipAttached() async throws {
        let (controller, transport) = await makeAttachedController()
        #expect(controller.snapshot.isAttached == true)
        // An unrelated client detaching must NOT drop us to the empty state
        // (this regressed window switches into a "No tmux sessions" screen).
        transport.pushText("%client-detached client-7\n")
        let stillAttached = await waitUntil(timeout: 0.5) {
            controller.snapshot.isAttached == false
        }
        #expect(stillAttached == false, "isAttached must stay true after %client-detached")
        #expect(controller.snapshot.isAttached == true)
    }

    // ─────────────────────────────────────────────────────────────
    // Viewed window is LOCAL state (multi-client yank protection)
    // ─────────────────────────────────────────────────────────────

    /// Attach + seed a two-window / two-pane world where we view @0 (%0).
    private func seedTwoWindowWorld(
        _ transport: MockTmuxTransport, controller: TmuxSessionController
    ) async {
        _ = await waitUntil { await transport.recordedCommands().count >= 3 }
        transport.pushText("""
        %begin 1 1 0
        $0 1 main
        %end 1 1 0
        %begin 2 2 0
        $0 @0 0 81x24,0,0,0 1 1 main
        $0 @1 1 81x24,0,0,1 0 1 logs
        %end 2 2 0
        %begin 3 3 0
        %0 @0 0 80 24 1 zsh
        %1 @1 0 80 24 1 vim
        %end 3 3 0

        """)
        _ = await waitUntil {
            controller.snapshot.activeWindowId == "@0"
                && controller.snapshot.panes.count == 2
        }
    }

    @Test("another client's %session-window-changed does NOT yank our viewed window")
    func externalWindowChangeIsIgnored() async throws {
        let (controller, transport) = await makeAttachedController()
        await seedTwoWindowWorld(transport, controller: controller)

        // A desktop client (or automation) switches the session's current
        // window to @1. Our view must stay on @0.
        transport.pushText("%session-window-changed $0 @1\n")

        let yanked = await waitUntil(timeout: 0.5) {
            controller.snapshot.activeWindowId == "@1"
        }
        #expect(yanked == false, "external window switch must not move our view")
        #expect(controller.snapshot.activeWindowId == "@0")
    }

    @Test("a discovery refresh flagging another window active does NOT yank our viewed window")
    func refreshDoesNotFollowServerCurrentWindow() async throws {
        let (controller, transport) = await makeAttachedController()
        await seedTwoWindowWorld(transport, controller: controller)

        // Drain the callbacks the seed's pane backfill queued (a
        // display-message per pane, whose handler queues a capture-pane
        // each), PLUS the fresh attach's automatic veil+resync of the
        // active pane %0 (see ensureTerminalsForAllPanes(isFreshAttach:)) —
        // its frame capture and cursor query, sent right after both panes'
        // backfill probes — so the NEXT responses line up with the refetch
        // we trigger below. Order (all within one push, no `await` before
        // the %window-add notification below, so it's all one FIFO batch):
        // backfill probe ×2, auto-resync frame, auto-resync cursor,
        // backfill capture ×2 — matching send order after a 2-pane fresh
        // attach.
        transport.pushText("""
        %begin 4 4 0
        0 0
        %end 4 4 0
        %begin 5 5 0
        0 0
        %end 5 5 0
        %begin 6 6 0
        %end 6 6 0
        %begin 7 7 0
        0 0
        %end 7 7 0
        %begin 8 8 0
        %end 8 8 0
        %begin 9 9 0
        %end 9 9 0

        """)

        // Any server event that re-lists windows (here %window-add) queues a
        // list-windows + list-panes refetch. tmux's reply says @1 is now the
        // session's current window — our local choice @0 must survive.
        transport.pushText("%window-add @9\n")
        _ = await waitUntil { controller.snapshot.windows["@9"] != nil }
        transport.pushText("""
        %begin 10 10 0
        $0 @0 0 81x24,0,0,0 0 1 main
        $0 @1 1 81x24,0,0,1 1 1 logs
        %end 10 10 0
        %begin 11 11 0
        %0 @0 0 80 24 1 zsh
        %1 @1 0 80 24 1 vim
        %end 11 11 0

        """)

        // The refresh landed once @9's placeholder is replaced by the re-list.
        let refreshed = await waitUntil { controller.snapshot.windows["@9"] == nil }
        #expect(refreshed, "the pushed list-windows reply should replace the window set")
        #expect(controller.snapshot.activeWindowId == "@0",
                "list-windows refresh must not follow tmux's current window")
    }

    @Test("our own selectWindow's %session-window-changed echo is accepted (idempotent)")
    func ownSelectWindowEchoAccepted() async throws {
        let (controller, transport) = await makeAttachedController()
        await seedTwoWindowWorld(transport, controller: controller)

        controller.selectWindow("@1")   // optimistic: activeWindowId flips now
        #expect(controller.snapshot.activeWindowId == "@1")

        // tmux confirms our select-window; the echo must keep (not fight) it.
        transport.pushText("%session-window-changed $0 @1\n")
        try? await Task.sleep(for: .milliseconds(100))
        #expect(controller.snapshot.activeWindowId == "@1")
    }

    @Test("closing the viewed window falls back locally — window AND pane — and tmux's follow-up echo holds")
    func windowCloseFallsBackWindowAndPane() async throws {
        let (controller, transport) = await makeAttachedController()
        await seedTwoWindowWorld(transport, controller: controller)

        // Our window dies (%window-close) leaving only @1 — the close handler
        // falls back locally, and a subsequent server-side current-window
        // announcement for @1 must be accepted (it matches our fallback).
        transport.pushText("%window-close @0\n")
        let fellBack = await waitUntil {
            controller.snapshot.activeWindowId == "@1"
        }
        #expect(fellBack, "close of the viewed window must land on a survivor")
        #expect(controller.snapshot.activePaneId == "%1",
                "activePaneId must move off the dead pane immediately")

        transport.pushText("%session-window-changed $0 @1\n")
        try? await Task.sleep(for: .milliseconds(100))
        #expect(controller.snapshot.activeWindowId == "@1")
    }

    @Test("closing the viewed window does not push select-window (no yanking desktop clients back)")
    func windowCloseFallbackIsLocalOnly() async throws {
        let (controller, transport) = await makeAttachedController()
        await seedTwoWindowWorld(transport, controller: controller)
        // Drain commands recorded so far, then close the viewed window.
        let baseline = await transport.recordedCommands().count

        transport.pushText("%window-close @0\n")
        _ = await waitUntil { controller.snapshot.activeWindowId == "@1" }
        try? await Task.sleep(for: .milliseconds(150))

        let after = await transport.recordedCommands()
        let newCommands = after.dropFirst(baseline)
        #expect(!newCommands.contains { $0.hasPrefix("select-window") },
                "local fallback must not move tmux's shared current window")
    }
}

/// The window-width parser behind the multi-client re-pin: when a layout-change
/// shows the active window was sized by another (wide desktop) client,
/// TmuxSessionController reclaims it to the phone width.
@Suite("tmux layout width")
struct TmuxLayoutWidthTests {
    typealias C = TmuxSessionController

    @Test("parses the window width from a simple layout")
    func simple() {
        #expect(C.windowWidth(fromLayout: "bb62,355x62,0,0,1") == 355)
        #expect(C.windowWidth(fromLayout: "1a2b,70x35,0,0,4") == 70)
    }

    @Test("parses the WINDOW width (first WxH) of a split layout")
    func split() {
        #expect(C.windowWidth(fromLayout: "34de,80x24,0,0{40x24,0,0,1,39x24,41,0,2}") == 80)
    }

    @Test("returns nil for a malformed layout")
    func malformed() {
        #expect(C.windowWidth(fromLayout: "") == nil)
        #expect(C.windowWidth(fromLayout: "justchecksum") == nil)
    }
}

/// Per-connection persistence for the last tmux selection — must survive the
/// ActiveSession's death (protocol switch, app relaunch), which is exactly
/// when it's needed: the mosh renderer attaches the REMEMBERED session by
/// name instead of tmux's "most recent".
@Suite("tmux selection store")
struct TmuxSelectionStoreTests {
    private func isolatedDefaults() -> UserDefaults {
        let name = "moshi-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: name)!
        d.removePersistentDomain(forName: name)
        return d
    }

    @Test("round-trips a selection per connection id")
    func roundTrip() {
        let d = isolatedDefaults()
        let id = UUID()
        let sel = TmuxSelection(session: "$3", window: "@7", pane: "%12")
        TmuxSelectionStore.save(sel, for: id, defaults: d)
        #expect(TmuxSelectionStore.load(id, defaults: d) == sel)
        // A different connection id sees nothing.
        #expect(TmuxSelectionStore.load(UUID(), defaults: d) == nil)
    }

    @Test("saving nil clears the stored selection")
    func nilClears() {
        let d = isolatedDefaults()
        let id = UUID()
        TmuxSelectionStore.save(TmuxSelection(session: "$0", window: nil, pane: nil), for: id, defaults: d)
        TmuxSelectionStore.save(nil, for: id, defaults: d)
        #expect(TmuxSelectionStore.load(id, defaults: d) == nil)
    }
}
