import Foundation

// Stand-ins for the two app types MoshTransport references whose real home
// (MoshBootstrap.swift) drags in SSHSession/Citadel and HostCapabilities —
// none of which a standalone MoshTransport CLI links. Field-compatible with
// the app's definitions; the driver script
// (scripts/capture/capture-mosh-switch-bytes.sh) hands us an already-parsed
// port + key, so none of MoshBootstrap's parsing needs to exist here.

struct MoshCredentials: Equatable {
    let host: String
    let udpPort: UInt16
    let key: Data
}

enum MoshBootstrap {
    enum BootstrapError: Error {
        case noConnectLine(String)
        case unusablePort(Int)
        case badKey
    }
}
