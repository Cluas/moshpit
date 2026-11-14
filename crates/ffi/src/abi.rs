//! The version of the library's hand-written C ABI
//! (`include/moshpit_ffi.h`, the `MoshpitFFI` module in Swift). The app
//! checks `mp_ffi_abi_version() == MP_FFI_ABI_VERSION` before it uses the
//! Rust core's switches, so a stale `libmoshpit_ffi.a` is refused instead of
//! misread.
//!
//! Version 3: the tmux control-mode parser's C ABI (`mp_tmux_parser_*`)
//! went with the Swift tmux controller's Rust parser switch; the session
//! core reads the control stream with `moshpit_tmux` directly.

/// `MP_FFI_ABI_VERSION` as this library was built.
pub const ABI_VERSION: u32 = 3;

/// The ABI version the library was built with (`MP_FFI_ABI_VERSION`).
#[unsafe(no_mangle)]
pub extern "C" fn mp_ffi_abi_version() -> u32 {
    ABI_VERSION
}
