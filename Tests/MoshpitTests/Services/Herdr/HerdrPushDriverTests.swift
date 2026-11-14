import Foundation
import Testing
@testable import Moshpit

@Suite("herdr push driver")
@MainActor
struct HerdrPushDriverTests {

    // MARK: - Boot line

    @Test("Boot line is single-line, single-quote-safe, and kills the line discipline")
    func bootLineShape() {
        let line = HerdrPushBoot.bootLine()
        // A raw newline would submit the shell command early — the script's
        // newlines must live as \n escapes inside python's exec() string.
        #expect(!line.contains("\n"))
        // python3 found as the capability probe finds it (B267: the pump is
        // its channel's program, run by a non-interactive shell).
        let prefix = "stty raw -echo; PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" exec python3 -c "
        #expect(line.hasPrefix(prefix))
        // The -c argument is one word that every login shell reads alike
        // (B138): hand-quoted, fish read its `\\n` as `\n` and python met
        // a raw newline in a bytes literal.
        let word = String(line.dropFirst(prefix.count))
        let program = PosixShell.unquote(word)
        #expect(program != nil)
        #expect(PosixShell.unquoteAsFish(word) == program)
        #expect(program?.hasPrefix("exec(\"import os,socket,threading\\n") == true)
        #expect(program?.contains("b\\\"MOSHPIT_PUMP_READY\\\\n\\\")") == true)
        #expect(line.contains("~/.config/herdr/herdr.sock"))
    }

    // MARK: - Subscription kinds

    @Test("Global kinds never include a pane-scoped kind (bare = server hangup)")
    func globalKindsExcludePaneScoped() {
        let global = Set(HerdrPushDriver.globalKinds)
        #expect(global.isDisjoint(with: HerdrPushDriver.paneScopedKinds))
        // No duplicates sneaking a kind in twice.
        #expect(global.count == HerdrPushDriver.globalKinds.count)
    }

    // MARK: - Activation

    /// Answers the subscribe request with subscription_started, echoing the id.
    private func makeActivatedDriver(
        invalidations: @escaping @MainActor () -> Void
    ) async -> HerdrPushDriver {
        // The driver's write closure replies through its own feed — the same
        // scripted-server loop the socket client tests use.
        final class Box: @unchecked Sendable { var driver: HerdrPushDriver? }
        let box = Box()
        let driver = HerdrPushDriver(
            write: { data in
                let line = String(decoding: data, as: UTF8.self)
                guard let object = HerdrSnapshot.firstJSONObject(in: line),
                      let id = object["id"] as? String else { return }
                let reply = #"{"id":"\#(id)","result":{"type":"subscription_started"}}"# + "\n"
                await box.driver?.feed(Data(reply.utf8))
            },
            onInvalidate: invalidations)
        box.driver = driver
        // activate() waits for the pump's readiness marker before writing —
        // see HerdrPushBoot.readyMarker. A test server that never says it is
        // there would (correctly) never be subscribed to.
        await driver.feed(Data((HerdrPushBoot.readyMarker + "\n").utf8))
        return driver
    }

    @Test("A confirmed subscribe activates push")
    func activateSucceeds() async {
        let driver = await makeActivatedDriver { }
        #expect(await driver.activate())
        #expect(driver.isActive)
        await driver.shutdown()
        #expect(!driver.isActive)
    }

    /// A herdr server restart ends the pump's transport. The driver went
    /// inactive and told nobody, so its owner kept counting on push (B188).
    @Test("Push that ends on its own says so, once; a shutdown says nothing")
    func endingIsTold() async throws {
        final class Count: @unchecked Sendable { var value = 0 }
        let ended = Count()
        let driver = await makeActivatedDriver { }
        driver.onEnded = { ended.value += 1 }
        #expect(await driver.activate())

        await driver.transportEnded()
        let deadline = ContinuousClock.now + .seconds(5)
        while ended.value == 0, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        #expect(ended.value == 1, "push ended and its owner was never told")
        #expect(!driver.isActive)
        await driver.shutdown()
        try await Task.sleep(for: .milliseconds(100))
        #expect(ended.value == 1)

        let quiet = Count()
        let stopped = await makeActivatedDriver { }
        stopped.onEnded = { quiet.value += 1 }
        #expect(await stopped.activate())
        await stopped.shutdown()
        try await Task.sleep(for: .milliseconds(100))
        #expect(quiet.value == 0, "the owner's own shutdown was reported as push dying")
    }

    @Test("Silence (no python3 on the host) fails activation without throwing")
    func activateTimesOutQuietly() async {
        let driver = HerdrPushDriver(
            requestTimeout: .milliseconds(80),
            write: { _ in /* the pump never came up; nothing answers */ },
            onInvalidate: { })
        #expect(await driver.activate() == false)
        #expect(!driver.isActive)
    }

    // MARK: - Debounce

    @Test("An event burst collapses into one invalidation; a later event fires again")
    func debounceCollapsesBursts() async throws {
        final class Counter: @unchecked Sendable {
            private let lock = NSLock()
            private var value = 0
            func bump() { lock.withLock { value += 1 } }
            var count: Int { lock.withLock { value } }
        }
        let counter = Counter()
        let driver = await makeActivatedDriver { counter.bump() }
        #expect(await driver.activate())

        // The bootstrap-replay shape: a burst of events in one gulp.
        let event = #"{"event":"pane_updated","data":{}}"# + "\n"
        await driver.feed(Data(String(repeating: event, count: 5).utf8))
        try await Task.sleep(for: .milliseconds(500))
        #expect(counter.count == 1)

        await driver.feed(Data(event.utf8))
        try await Task.sleep(for: .milliseconds(500))
        #expect(counter.count == 2)
        await driver.shutdown()
    }

    /// The bug this pins was invisible to the burst test above, because the
    /// broken guard ("is a timer already running?") DOES collapse events that
    /// arrive in one gulp. It only shows when they are spread out: it then
    /// fires once per interval for as long as the stream lasts. Measured
    /// against a real server the first time push actually came up — the
    /// subscribe's own replay produced 37 snapshot reads in 8 seconds.
    @Test("A stream of events spread over time is still ONE invalidation")
    func debounceResetsRatherThanRepeating() async throws {
        final class Counter: @unchecked Sendable {
            private let lock = NSLock()
            private var value = 0
            func bump() { lock.withLock { value += 1 } }
            var count: Int { lock.withLock { value } }
        }
        let counter = Counter()
        let driver = await makeActivatedDriver { counter.bump() }
        #expect(await driver.activate())

        let event = #"{"event":"pane_updated","data":{}}"# + "\n"
        // 9 events over ~900ms: longer than the debounce, shorter than the cap.
        for _ in 0..<9 {
            await driver.feed(Data(event.utf8))
            try await Task.sleep(for: .milliseconds(100))
        }
        try await Task.sleep(for: .milliseconds(500))
        #expect(counter.count == 1, "a spread-out stream must not fire per interval")
        await driver.shutdown()
    }

    /// …and the cap that keeps the reset from starving a read forever, for a
    /// stream that never stops (an agent printing output bumps its pane on
    /// every chunk).
    @Test("A stream longer than the cap still gets read")
    func debounceCapPreventsStarvation() async throws {
        final class Counter: @unchecked Sendable {
            private let lock = NSLock()
            private var value = 0
            func bump() { lock.withLock { value += 1 } }
            var count: Int { lock.withLock { value } }
        }
        let counter = Counter()
        let driver = await makeActivatedDriver { counter.bump() }
        #expect(await driver.activate())
        let event = #"{"event":"pane_updated","data":{}}"# + "\n"
        for _ in 0..<26 {
            await driver.feed(Data(event.utf8))
            try await Task.sleep(for: .milliseconds(100))
        }
        #expect(counter.count >= 1, "a never-ending stream must not defer the read forever")
        await driver.shutdown()
    }

    // MARK: - Capability gate

    @Test("The probe covers python3 and parses it")
    func probeCoversPython3() {
        #expect(HostCapabilities.probeCommand.contains("python3"))
        let caps = HostCapabilities.parse("""
        /opt/homebrew/bin/herdr
        /usr/bin/python3
        ::Darwin::/opt/homebrew/bin/brew
        """)
        #expect(caps.hasPython3)
        #expect(!HostCapabilities.parse("::Linux::").hasPython3)
        // Pre-probe optimism: unknown assumes present, so a cold session
        // attempts the upgrade and settles on polling if it was wrong.
        #expect(HostCapabilities.unknown.hasPython3)
    }
}
