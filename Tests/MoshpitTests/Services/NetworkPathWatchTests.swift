import Foundation
import Network
import Testing
@testable import Moshpit

/// B303: in the foreground the app logged a network path change every 5 s
/// (both hubs), and each one restarted any dial out for over a second,
/// roamed mosh onto a fresh flow and probed SSH. NWPathMonitor calls its
/// handler on every re-evaluation (a VPN re-applying its settings), with
/// nothing moved; the watch told each one after the first as a change. Only
/// a path that differs from its monitor's last is a change.
@Suite("Network path watch")
struct NetworkPathWatchTests {
    /// How many changes the watch told.
    final class Told: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        func tell() { lock.withLock { count += 1 } }
        var value: Int { lock.withLock { count } }
    }

    static let wifi = NetworkPathSnapshot(status: "satisfied", interfaces: ["en0/wifi", "utun3/other"],
                                          gateways: ["192.0.2.1:0"])

    static func watch() -> (NetworkPathWatch, Told) {
        let told = Told()
        return (NetworkPathWatch { told.tell() }, told)
    }

    @Test("a monitor's first update is the state at start, not a change")
    func first() {
        let (watch, told) = Self.watch()
        watch.deliver(Self.wifi, monitor: 0)
        watch.deliver(Self.wifi, monitor: 1)
        #expect(told.value == 0)
    }

    @Test("the same path re-reported is no change, however often (B303)")
    func reReported() {
        let (watch, told) = Self.watch()
        for _ in 0..<12 {
            watch.deliver(Self.wifi, monitor: 0)
            watch.deliver(Self.wifi, monitor: 1)
        }
        #expect(told.value == 0)
    }

    @Test("a path that moved is a change, told once", arguments: [
        ("an interface went", NetworkPathSnapshot(status: "satisfied", interfaces: ["en0/wifi"],
                                                  gateways: ["192.0.2.1:0"])),
        ("Wi-Fi handed over to cellular", NetworkPathSnapshot(status: "satisfied",
                                                              interfaces: ["pdp_ip0/cellular", "utun3/other"],
                                                              gateways: ["198.51.100.1:0"], expensive: true)),
        ("the preferred interface changed", NetworkPathSnapshot(status: "satisfied",
                                                                interfaces: ["utun3/other", "en0/wifi"],
                                                                gateways: ["192.0.2.1:0"])),
        ("another Wi-Fi network", NetworkPathSnapshot(status: "satisfied", interfaces: ["en0/wifi", "utun3/other"],
                                                      gateways: ["203.0.113.1:0"])),
        ("no network", NetworkPathSnapshot(status: "unsatisfied", interfaces: [])),
        ("Low Data Mode", NetworkPathSnapshot(status: "satisfied", interfaces: ["en0/wifi", "utun3/other"],
                                              gateways: ["192.0.2.1:0"], constrained: true)),
    ])
    func moved(_ what: String, _ next: NetworkPathSnapshot) {
        let (watch, told) = Self.watch()
        watch.deliver(Self.wifi, monitor: 0)
        watch.deliver(next, monitor: 0)
        #expect(told.value == 1, "\(what)")
        watch.deliver(next, monitor: 0)
        #expect(told.value == 1, "\(what), re-reported")
        watch.deliver(Self.wifi, monitor: 0)
        #expect(told.value == 2, "\(what), and back")
    }

    @Test("each monitor is compared with its own last update")
    func perMonitor() {
        let (watch, told) = Self.watch()
        let tunnel = NetworkPathSnapshot(status: "satisfied", interfaces: ["utun3/other"])
        watch.deliver(Self.wifi, monitor: 0)
        watch.deliver(tunnel, monitor: 1)
        watch.deliver(Self.wifi, monitor: 0)
        watch.deliver(tunnel, monitor: 1)
        #expect(told.value == 0)
        watch.deliver(NetworkPathSnapshot(status: "unsatisfied", interfaces: []), monitor: 1)
        #expect(told.value == 1)
    }

    @Test("a real path's snapshot is the same path each time it is read")
    func realPath() {
        let path = NWPathMonitor().currentPath
        #expect(NetworkPathSnapshot(path) == NetworkPathSnapshot(path))
        #expect(NetworkPathSnapshot(path).differences(from: NetworkPathSnapshot(path)).isEmpty)
    }
}
