#!/bin/bash
# Builds the Rust static library the app links — crates/ffi, out as
# target/<rust target>/<profile>/libmoshpit_ffi.a — for whatever Xcode is
# building. The Moshpit target's "Build Rust" phase (project.yml) runs it
# before any Swift compiles; project.yml's LIBRARY_SEARCH_PATHS point at its
# output. By hand:
#
#   scripts/rust/build-ios.sh                              # the device
#   scripts/rust/build-ios.sh iphonesimulator arm64 x86_64 # the simulator
#
# MOSHPIT_RUST_PACKAGE names another package: the notification service
# extension's own library (moshpit-seal-ffi, E7b), which its target builds
# the same way and links alone — one Rust library per binary.
#
# The cargo profile (Cargo.toml) follows the Xcode configuration: `app` for
# Release — everything that leaves this Mac, Canary and the App Store,
# optimised for size with LTO over the whole crate graph — and `app-test`
# for Debug — every build-for-testing, without that LTO (Cargo.toml).
# Xcode hands the profile over as the MOSHPIT_RUST_PROFILE build setting,
# which project.yml sets per configuration and LIBRARY_SEARCH_PATHS reads
# too: one value says both what cargo builds and what the app links, so a
# Debug app never links a Release library left in target/ (or the other way
# round). An Xcode build ($CONFIGURATION set) whose project doesn't set it
# was generated from an older project.yml and would link from the old
# place: refused, as scripts/release/release-archive.sh refuses a stale
# project. A run by hand (neither set) builds `app`, the profile that ships;
# MOSHPIT_RUST_PROFILE=app-test builds the other.
#
# Cargo runs in a fixed environment (env -i). Xcode exports SDKROOT, CC,
# LD, MACOSX_DEPLOYMENT_TARGET and many more that a shell does not, and
# cargo fingerprints some of them, so a build from Xcode and one from a
# shell would keep invalidating each other's work. In the same environment
# either way, cargo rebuilds nothing when nothing changed, the library keeps
# its timestamp, and the app is not relinked for it (below). The toolchain is the one
# rust-toolchain.toml pins, through rustup; a missing Rust target is added
# the first time it is needed.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
platform="${1:-${PLATFORM_NAME:-iphoneos}}"
if [ "$#" -gt 1 ]; then
    shift
    archs="$*"
else
    archs="${ARCHS:-arm64}"
fi
# The app's deployment target (project.yml). Xcode passes its own value; the
# default must match it, or builds from Xcode and from a shell would each
# rebuild the library (rustc reads the variable, cargo fingerprints it).
deployment="${IPHONEOS_DEPLOYMENT_TARGET:-18.0}"

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
# rustup's proxies (cargo, rustc, rustup): in $CARGO_HOME/bin, where
# rustup.rs puts them, or else wherever the caller's PATH has them: a package
# manager's rustup (Homebrew's, the one the macOS CI image has) keeps them
# out of ~/.cargo. Looked up here, before the fixed environment below, whose
# PATH is that directory and the system's.
if [ -x "$cargo_home/bin/cargo" ]; then
    cargo_bin="$cargo_home/bin"
elif cargo_on_path="$(command -v cargo 2>/dev/null)" && [ -n "$cargo_on_path" ]; then
    cargo_bin="$(cd "$(dirname "$cargo_on_path")" && pwd)"
else
    echo "error: no cargo in $cargo_home/bin, nor on PATH ($PATH). The app links a Rust library (crates/ffi): install rustup from https://rustup.rs and build again." >&2
    exit 1
fi
env_args=(
    HOME="$HOME"
    PATH="$cargo_bin:/usr/bin:/bin:/usr/sbin:/sbin"
    TERM=dumb
    IPHONEOS_DEPLOYMENT_TARGET="$deployment"
    # Two jobs by default: the build machine is shared. Not part of cargo's
    # fingerprints, so changing it rebuilds nothing.
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
    # This checkout's own target directory (the one project.yml's
    # LIBRARY_SEARCH_PATHS reads), whatever a cargo config says: a
    # `build.target-dir` in ~/.cargo/config.toml or in a parent directory's
    # would share one directory between worktrees. Cargo names a workspace
    # member's artifacts and fingerprints by its path inside the workspace,
    # so two checkouts of this repository building into one directory take
    # each other's: after one builds, the other's crates look fresh, cargo
    # rebuilds nothing, and the app links the other checkout's library
    # (it happened by hand: an export of an older commit built with
    # CARGO_TARGET_DIR pointing here). Not fingerprinted, and the same
    # directory as cargo's default: nothing rebuilds.
    CARGO_TARGET_DIR="$root/target"
)
# Cargo runs at nice 10 by default, the build machine being shared; a build
# that holds the machine to itself (Canary) can say MOSHPIT_RUST_NICE=0.
# On this Mac's 4+4 cores a niced build loses the performance cores to the
# app's Swift compile when the two run at once (MOSHPIT_RUST_ASYNC).
niceness="${MOSHPIT_RUST_NICE:-10}"
[ -n "${CARGO_HOME:-}" ] && env_args+=(CARGO_HOME="$CARGO_HOME")
[ -n "${RUSTUP_HOME:-}" ] && env_args+=(RUSTUP_HOME="$RUSTUP_HOME")

if [ -n "${MOSHPIT_RUST_PROFILE:-}" ]; then
    profile="$MOSHPIT_RUST_PROFILE"
elif [ -n "${CONFIGURATION:-}" ]; then
    echo "error: this $CONFIGURATION build's Xcode project doesn't set MOSHPIT_RUST_PROFILE, so which Rust library it links is unknown: it was generated from an older project.yml. Run: xcodegen generate" >&2
    exit 1
else
    profile=app
fi
case "$profile" in
    app | app-test | app-canary) ;;
    *)
        echo "error: MOSHPIT_RUST_PROFILE is \"$profile\": the app links the \`app\`, \`app-canary\` or \`app-test\` cargo profile (Cargo.toml, scripts/rust/build-ios.sh)" >&2
        exit 1
        ;;
esac
package="${MOSHPIT_RUST_PACKAGE:-moshpit-ffi}"
case "$package" in
    moshpit-ffi | moshpit-seal-ffi) ;;
    *)
        echo "error: MOSHPIT_RUST_PACKAGE is \"$package\": the app links moshpit-ffi, its notification service moshpit-seal-ffi (scripts/rust/build-ios.sh)" >&2
        exit 1
        ;;
esac
echo "note: $package for ${CONFIGURATION:-a build by hand} with the cargo profile $profile"

# What the library was before cargo (inode, size, mtime), each target's.
# Xcode does not count it among the link's inputs (`-lmoshpit_ffi` in
# OTHER_LDFLAGS): with no Swift changed it skips the link whatever cargo
# did, and the app ships the previous Rust (Canary 174 and 175). When cargo
# changed the library, the link's output goes, so the link runs. The
# background build (build-ios-background.sh) removes it up front instead.
signature() {
    stat -f '%i %z %m' "$root/target/$1/$profile/lib${package//-/_}.a" 2>/dev/null || echo none
}
relink=""

cd "$root"
for arch in $archs; do
    case "$platform/$arch" in
        iphoneos/arm64) target=aarch64-apple-ios ;;
        iphonesimulator/arm64) target=aarch64-apple-ios-sim ;;
        iphonesimulator/x86_64) target=x86_64-apple-ios ;;
        *)
            echo "error: no Rust target for $platform/$arch (scripts/rust/build-ios.sh)" >&2
            exit 1
            ;;
    esac
    sysroot="$(env -i "${env_args[@]}" rustc --print sysroot)"
    if ! [ -d "$sysroot/lib/rustlib/$target" ]; then
        echo "note: adding the Rust target $target to $(basename "$sysroot")"
        env -i "${env_args[@]}" rustup target add "$target"
    fi
    before="$(signature "$target")"
    env -i "${env_args[@]}" nice -n "$niceness" \
        cargo build --locked -p "$package" --profile "$profile" --target "$target"
    [ "$(signature "$target")" = "$before" ] || relink=yes
done
if [ -n "$relink" ] && [ -z "${MOSHPIT_RUST_IN_BACKGROUND:-}" ] \
    && [ -n "${TARGET_BUILD_DIR:-}" ] && [ -n "${EXECUTABLE_PATH:-}" ]; then
    echo "note: the Rust library changed: the app relinks"
    rm -f "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"
    if [ -n "${OBJECT_FILE_DIR_normal:-}" ] && [ -n "${EXECUTABLE_NAME:-}" ]; then
        rm -f "$OBJECT_FILE_DIR_normal"/*/Binary/"$EXECUTABLE_NAME"
    fi
fi
