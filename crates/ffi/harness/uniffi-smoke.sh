#!/bin/bash
# Generates the store's Swift bindings from the macOS build of the library
# (UniFFI library mode: the interface is read out of the archive) and runs
# harness/smoke/main.swift over them: a PreferencesHost on a real
# UserDefaults suite, a KeychainHost on a dictionary, the Rust stores behind
# them. Builds one thing at a time, niced.
#
#   harness/uniffi-smoke.sh [<work dir>]   (default target/ffi-smoke)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
work="${1:-$repo/target/ffi-smoke}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
cd "$repo"
nice -n 10 cargo build --quiet -p moshpit-ffi
nice -n 10 cargo run --quiet -p moshpit-ffi --example uniffi-bindgen -- generate \
    --library "$repo/target/debug/libmoshpit_ffi.a" --language swift --out-dir "$work/bindings"
nice -n 10 swiftc -swift-version 5 -parse-as-library \
    -I "$work/bindings" -Xcc -fmodule-map-file="$work/bindings/moshpit_ffiFFI.modulemap" \
    "$work/bindings/moshpit_ffi.swift" "$here/smoke/main.swift" \
    -L "$repo/target/debug" -lmoshpit_ffi -o "$work/smoke"
"$work/smoke"
