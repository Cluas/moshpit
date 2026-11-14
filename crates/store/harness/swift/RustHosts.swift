// What the app's Rust hosts (Moshpit/Services/Rust/RustCoreHosts.swift, cut
// by build-swift.sh) need that the harness stands in for, in store-swift
// and store-swift-rust alike: both reach the Rust store, store-swift for
// the shortcuts the app keeps there whatever its switch says.

import Foundation

/// The harness's stand-in for the app's `ProtectedDataFlag` (which asks
/// UIApplication): the case's phone.
final class ProtectedDataFlag: @unchecked Sendable {
    private let probe: () -> Bool
    init(_ probe: @escaping () -> Bool) { self.probe = probe }
    var isAvailable: Bool { probe() }
}
