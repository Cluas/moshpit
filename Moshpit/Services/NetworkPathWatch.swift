import Foundation
import MoshpitKit
import Network

/// Reports that the device's network path changed: an interface came or went,
/// Wi-Fi handed over to cellular, a VPN tunnel opened or closed.
///
/// Two monitors, because one is not enough for the case that matters most.
/// The default-path monitor covers interfaces and handovers, but a
/// split-tunnel VPN such as Tailscale — routes for its own address range
/// only, nothing else moves — can come up without the default path changing
/// at all. The `.other` monitor watches exactly that: whether a tunnel
/// interface can carry traffic. Either changing is a change.
///
/// An update is not a change. NWPathMonitor calls its handler whenever the
/// system re-evaluates the path, and a VPN's network extension re-applying
/// its settings makes it do so every few seconds with nothing moved (B303:
/// both hubs logged a path change every 5 s in the foreground, and each one
/// restarted any dial out for over a second, roamed mosh onto a fresh flow
/// and probed SSH). So each update is compared with the last of its monitor
/// on what a connection depends on ([`NetworkPathSnapshot`]), and only a
/// difference is told.
///
/// The consumer decides what a change means; this only says *something
/// moved*. It never says what, because what the hub does about it (redial
/// what is dialing, retry what is down, probe what is up) is the same for
/// every kind of change.
final class NetworkPathWatch: @unchecked Sendable {
    private let monitors: [NWPathMonitor]
    private let queue = DispatchQueue(label: "moshpit.network-path")
    private let lock = NSLock()
    private var changes = NetworkPathChanges()
    private let onChange: @Sendable () -> Void

    init(onChange: @escaping @Sendable () -> Void) {
        self.onChange = onChange
        monitors = [NWPathMonitor(), NWPathMonitor(requiredInterfaceType: .other)]
        for (index, monitor) in monitors.enumerated() {
            monitor.pathUpdateHandler = { [weak self] path in
                self?.deliver(NetworkPathSnapshot(path), monitor: index)
            }
        }
    }

    func start() {
        for monitor in monitors { monitor.start(queue: queue) }
    }

    /// Monitor `monitor`'s update: told as a change only when it differs
    /// from that monitor's last.
    func deliver(_ snapshot: NetworkPathSnapshot, monitor: Int) {
        let verdict = lock.withLock { changes.take(snapshot, monitor: monitor) }
        switch verdict {
        case .first:
            break
        case .unchanged(let firstTime):
            if firstTime {
                Log.ssh.notice("network path: monitor \(monitor, privacy: .public) re-reported an unchanged path; such updates are ignored")
            }
        case .changed(let fields):
            Log.ssh.notice("network path: monitor \(monitor, privacy: .public) changed: \(fields.joined(separator: ", "), privacy: .public)")
            onChange()
        }
    }
}

/// What of a network path a connection depends on: whether it is up, the
/// interfaces that carry it in the order the system prefers them, their
/// gateways (another Wi-Fi network on the same interface), whether it is
/// expensive or constrained, and what it supports. Two paths with the same
/// snapshot are the same path to the app, whatever else NWPathMonitor
/// re-reported.
struct NetworkPathSnapshot: Equatable {
    var status: String
    var interfaces: [String]
    var gateways: [String]
    var expensive: Bool
    var constrained: Bool
    var ipv4: Bool
    var ipv6: Bool
    var dns: Bool

    init(status: String, interfaces: [String], gateways: [String] = [], expensive: Bool = false,
         constrained: Bool = false, ipv4: Bool = true, ipv6: Bool = true, dns: Bool = true) {
        self.status = status
        self.interfaces = interfaces
        self.gateways = gateways
        self.expensive = expensive
        self.constrained = constrained
        self.ipv4 = ipv4
        self.ipv6 = ipv6
        self.dns = dns
    }

    init(_ path: NWPath) {
        self.init(status: "\(path.status)",
                  interfaces: path.availableInterfaces.map { "\($0.name)/\($0.type)" },
                  gateways: path.gateways.map { "\($0)" },
                  expensive: path.isExpensive, constrained: path.isConstrained,
                  ipv4: path.supportsIPv4, ipv6: path.supportsIPv6, dns: path.supportsDNS)
    }

    /// The names of the fields in which `other` differs from this one.
    func differences(from other: NetworkPathSnapshot) -> [String] {
        var fields: [String] = []
        if status != other.status { fields.append("status \(other.status) → \(status)") }
        if interfaces != other.interfaces { fields.append("interfaces") }
        if gateways != other.gateways { fields.append("gateways") }
        if expensive != other.expensive { fields.append("expensive") }
        if constrained != other.constrained { fields.append("constrained") }
        if ipv4 != other.ipv4 || ipv6 != other.ipv6 { fields.append("IP versions") }
        if dns != other.dns { fields.append("DNS") }
        return fields
    }
}

/// Which of the monitors' updates are changes: a monitor's first is the
/// state at start, not a change; after it, one that differs from the
/// monitor's last.
struct NetworkPathChanges {
    enum Verdict: Equatable {
        /// The monitor's first update.
        case first
        /// The same path again (`firstTime`: the first such of the monitor).
        case unchanged(firstTime: Bool)
        /// A different path: the fields that changed.
        case changed([String])
    }

    private var last: [Int: NetworkPathSnapshot] = [:]
    private var repeated: Set<Int> = []

    mutating func take(_ snapshot: NetworkPathSnapshot, monitor: Int) -> Verdict {
        defer { last[monitor] = snapshot }
        guard let previous = last[monitor] else { return .first }
        let fields = snapshot.differences(from: previous)
        if fields.isEmpty { return .unchanged(firstTime: repeated.insert(monitor).inserted) }
        return .changed(fields)
    }
}
