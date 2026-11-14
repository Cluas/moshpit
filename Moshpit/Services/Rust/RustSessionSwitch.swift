import Foundation
import MoshpitFFI

/// A connection's terminal runs on the Rust core's session layer
/// (crates/session through crates/ffi, docs/nsh/DESIGN.md §3.6), not
/// ``SessionHub``'s Swift transports.
///
/// No switch any more (F0): every build runs every connection on the core
/// whenever its library is the one the app was compiled against, as Canary
/// builds did. The Swift transports go with batches E1–E4.
enum RustSessionSwitch {
    /// Whether this build can run sessions on the Rust core: the library is
    /// linked and is the one the app was compiled against.
    static let available = mp_ffi_abi_version() == UInt32(MP_FFI_ABI_VERSION)

    /// Whether `connection` runs on the core: every one, in a build that can.
    @MainActor
    static func runsOnRust(_ connection: ServerConnection) -> Bool {
        available
    }
}
