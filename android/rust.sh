#!/usr/bin/env bash
# Builds the app's one Rust library, crates/android, as the APK's
# lib/<abi>/libnsh.so (NSH's Java runtime loads "nsh"): for each ABI, with the
# NDK's clang, 16 KB-aligned (Android 15's 16 KB pages), stripped, with the
# unstripped copy kept for symbolication.
#
#   android/rust.sh <profile> <jniLibs dir> [<symbols dir>]
#
#   profile: release (development builds: fast to build) or app (what ships:
#            opt-level z, LTO, one codegen unit; the workspace manifest says why)
#   ABIS:    the ABIs to build (default arm64-v8a; add x86_64 for x86 emulators)
#
# Both builds call it: android/build.sh (scripted, this machine) and the Gradle
# build (android/app/build.gradle, CI). Cargo.lock is --locked: the Rust half of
# NSH is the revision whose Java half the APK compiles.
set -euo pipefail

PROFILE=${1:?profile}
OUT=${2:?jniLibs dir}
SYMBOLS=${3:-}
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
MIN_SDK=26
: "${ABIS:=arm64-v8a}"
: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
: "${CARGO_BUILD_JOBS:=2}"
export CARGO_BUILD_JOBS
NDK="${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)}"
[ -d "$NDK" ] || { echo "no NDK (ANDROID_NDK_HOME or \$ANDROID_HOME/ndk/<version>)" >&2; exit 1; }
case "$(uname -s)" in
    Darwin) HOST_TAG=darwin-x86_64 ;; # universal binaries
    Linux) HOST_TAG=linux-x86_64 ;;
    *) echo "unsupported host" >&2; exit 1 ;;
esac
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG"
NICE=(nice -n 10)

for abi in $ABIS; do
    case "$abi" in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
        *) echo "unsupported ABI $abi" >&2; exit 1 ;;
    esac
    clang="$TOOLCHAIN/bin/$triple$MIN_SDK-clang"
    [ -x "$clang" ] || { echo "missing $clang" >&2; exit 1; }
    env_triple=${triple//-/_}
    upper=$(echo "$env_triple" | tr '[:lower:]' '[:upper:]')
    echo "== libnsh.so for $abi ($PROFILE)"
    (
        cd "$ROOT"
        export "CC_$env_triple=$clang" "AR_$env_triple=$TOOLCHAIN/bin/llvm-ar" \
            "CARGO_TARGET_${upper}_LINKER=$clang"
        "${NICE[@]}" cargo build --locked -p moshpit-android --target "$triple" --profile "$PROFILE"
    )
    so="$ROOT/target/$triple/$PROFILE/libmoshpit_android.so"
    # Android 15 devices with 16 KB pages: every LOAD segment aligned to 2**14 at least.
    if ! "$TOOLCHAIN/bin/llvm-objdump" -p "$so" | awk '$1 == "LOAD" { split($NF, a, "\\*\\*"); if (a[2] + 0 < 14) bad = 1; n++ } END { exit (bad || n == 0) }'; then
        echo "libnsh.so ($abi) is not 16 KB aligned" >&2
        exit 1
    fi
    mkdir -p "$OUT/$abi"
    if [ -n "$SYMBOLS" ]; then
        mkdir -p "$SYMBOLS/$abi"
        cp "$so" "$SYMBOLS/$abi/libnsh.so"
    fi
    "$TOOLCHAIN/bin/llvm-strip" --strip-unneeded -o "$OUT/$abi/libnsh.so" "$so"
    ls -l "$OUT/$abi/libnsh.so" | awk -v abi="$abi" '{ print "libnsh.so " abi ": " $5 " bytes stripped, 16 KB aligned" }'
done
