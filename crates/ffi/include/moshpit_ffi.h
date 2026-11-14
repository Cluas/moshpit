// The C ABI of the one Rust static library the iOS app links
// (crates/ffi, libmoshpit_ffi.a). Swift imports this header as the
// MoshpitFFI module (module.modulemap next to it), with the byte channel's
// (crates/stream/include/moshpit_stream.h), the terminal core's
// (crates/term/include/moshpit_term.h) and the NSH screens' host's
// (ui/include/moshpit_ui.h) beside it. The rest of the core is UniFFI.
//
// What is left here is the library's version: the tmux parser's C ABI went
// with the Rust parser switch of the Swift tmux controller (ABI 3).
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Bumped whenever anything below changes shape. The app checks
/// `mp_ffi_abi_version() == MP_FFI_ABI_VERSION` before it uses the library,
/// so a stale libmoshpit_ffi.a is refused instead of misread.
#define MP_FFI_ABI_VERSION 3

/// The `MP_FFI_ABI_VERSION` the library was built with.
uint32_t mp_ffi_abi_version(void);

#ifdef __cplusplus
}
#endif
