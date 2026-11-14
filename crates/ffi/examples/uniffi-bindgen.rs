//! UniFFI's bindings generator, for the store's API (src/store.rs):
//!
//!   cargo run -p moshpit-ffi --example uniffi-bindgen -- generate \
//!       --library target/<target>/<profile>/libmoshpit_ffi.a --language swift --out-dir <dir>
//!
//! (`--language kotlin` for Android.) Library mode reads the interface out of
//! the archive itself, so the bindings always match the library they call.

fn main() {
    uniffi::uniffi_bindgen_main()
}
