import Foundation
import Network

/// Reports that the device's network path changed: an interface came or went,
/// Wi-Fi handed over to cellular, a VPN tunnel opened or closed.
///
/// Two monitors, because one is not enough for the case that matters most.
/// The default-path monitor covers interfaces and handovers, but a
/// split-tunnel VPN such as Tailscale — routes for its own address range
/// only, nothing else moves — can come up without the default path changing
/// at all. The `.other` monitor watches exactly that: whether a tunnel
/// interface can carry traffic. Either firing is a change.
///
/// The consumer decides what a change means; this only says *something
/// moved*. It never says what, because what the hub does about it (redial
/// what is dialing, retry what is down, probe what is up) is the same for
/// every kind of change.
final class NetworkPathWatch: @unchecked Sendable {
    private let monitors: [NWPathMonitor]
    private let queue = DispatchQueue(label: "moshpit.network-path")
    private let lock = NSLock()
    /// Which monitors have delivered their first update. That first update is
    /// the state at start, not a change, and must not kick anything.
    private var primed = Set<Int>()

    init(onChange: @escaping @Sendable () -> Void) {
        monitors = [NWPathMonitor(), NWPathMonitor(requiredInterfaceType: .other)]
        for (index, monitor) in monitors.enumerated() {
            monitor.pathUpdateHandler = { [weak self] _ in
                guard let self else { return }
                let isFirst = self.lock.withLock { self.primed.insert(index).inserted }
                if !isFirst { onChange() }
            }
        }
    }

    func start() {
        for monitor in monitors { monitor.start(queue: queue) }
    }

    func cancel() {
        for monitor in monitors { monitor.cancel() }
    }
}
