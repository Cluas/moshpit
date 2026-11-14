#!/bin/bash
# The Swift bindings of the Rust library's UniFFI API (crates/ffi: the store,
# the sessions), where the app compiles them:
#
#   Moshpit/Services/Rust/Generated/moshpit_ffi.swift  the Swift side, compiled
#                                                      into the app's module
#   crates/ffi/include/moshpit_ffiFFI.h                its C side: the
#                                                      moshpit_ffiFFI module
#                                                      (include/module.modulemap)
#
# Generated from a host build of the library (UniFFI's library mode reads the
# interface out of the archive; the interface is the same for every target)
# and committed, so the app builds without running the generator. A stale
# copy fails loudly rather than misbehaving: UniFFI checks every function's
# checksum against the linked library the first time the bindings are used.
#
#   scripts/rust/gen-bindings.sh          # write them
#   scripts/rust/gen-bindings.sh --check  # compare; fail on a difference
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
swift_out="$root/Moshpit/Services/Rust/Generated/moshpit_ffi.swift"
header_out="$root/crates/ffi/include/moshpit_ffiFFI.h"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
cd "$root"
nice -n 10 cargo build --quiet --locked -p moshpit-ffi
nice -n 10 cargo run --quiet --locked -p moshpit-ffi --example uniffi-bindgen -- generate \
    --library "$root/target/debug/libmoshpit_ffi.a" --language swift --out-dir "$work"

if [ "${1:-}" = "--check" ]; then
    status=0
    cmp -s "$work/moshpit_ffi.swift" "$swift_out" || { echo "stale: $swift_out" >&2; status=1; }
    cmp -s "$work/moshpit_ffiFFI.h" "$header_out" || { echo "stale: $header_out" >&2; status=1; }
    [ "$status" -eq 0 ] && echo "bindings up to date"
    exit "$status"
fi
cp "$work/moshpit_ffi.swift" "$swift_out"
cp "$work/moshpit_ffiFFI.h" "$header_out"
echo "wrote $swift_out and $header_out"
