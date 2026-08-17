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
        // The whole -c argument rides in shell single quotes; a third quote
        // anywhere in the body would end them.
        #expect(line.filter { $0 == "'" }.count == 2)
        #expect(line.hasPrefix("stty raw -echo; "))
        #expect(line.contains("exec python3 -c"))
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
        // Old cached entries (no key) decode to false, not a throw.
        let old = try? JSONDecoder().decode(
            HostCapabilities.self,
            from: Data(#"{"hasTmux":true,"hasMoshServer":true,"os":"Linux"}"#.utf8))
        #expect(old?.hasPython3 == false)
        // Pre-probe optimism: unknown assumes present, so a cold session
        // attempts the upgrade and settles on polling if it was wrong.
        #expect(HostCapabilities.unknown.hasPython3)
    }
}
