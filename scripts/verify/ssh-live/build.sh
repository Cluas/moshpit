#!/bin/bash
# Builds ssh-live: the app's sign-in and connect code, compiled for macOS
# against the very package revisions the app pins, to run against a real
# OpenSSH server (run.sh starts a private one). A SwiftPM package over copies
# of the pinned checkouts (Citadel, the Wellz26 swift-nio-ssh fork it uses,
# swift-nio, swift-crypto with _CryptoExtras, and theirs; NIOSOCKS from
# swift-nio-extras as a target of its own), never modified where they are,
# with these app files UNCHANGED:
#
#   Moshpit/Services/RSASHA2Signing.swift    RSA sign-in with SHA-2 (B129)
#   Moshpit/Services/SSHKeyFactory.swift     the app's keys: generated, read,
#                                            the numbers a sign-in signs with
#   Moshpit/Services/SOCKSProxyDialer.swift  the SOCKS dial
#   Moshpit/Services/SSHDial.swift           signing in over a dialed channel
#   Moshpit/Services/SSHService.swift        the connect itself, and sessions
#   Moshpit/Services/HostKeyValidator.swift  trust on first use
#   Moshpit/Services/KeychainService.swift   (the in-memory keychain here)
#   Moshpit/Services/StoredKey.swift         keys stored as pasted
#   Moshpit/Services/SSHExec.swift           a command's own exec channel
#   Moshpit/Services/SSHCommandPTY.swift     a terminal whose program is a command (B267)
#   Moshpit/Services/HostConnection.swift    what SSHSession serves the features beside a terminal as
#   Moshpit/Services/RemoteFileHandle.swift  (a file it holds open for a stream)
#   Moshpit/Services/Shell.swift, Moshpit/Models/{ServerConnection,RemotePaths}.swift
#   Moshpit/Models/StoredList.swift          (StoredJSON, which the known hosts read with)
#   Packages/MoshpitKit/Sources/MoshpitKit/Log.swift, as the module MoshpitKit
#   Moshpit/Services/Rust/Generated/moshpit_ffi.swift, the app's UniFFI
#                                            bindings: since de2bba14 every key
#                                            is read in Rust (moshpit_ssh::keyfile)
#
# linked against the Rust library built for this Mac
# (target/debug/libmoshpit_ffi.a, `cargo build -p moshpit-ffi`, run here
# first), with crates/ffi/include for its C side, as the app links it;
# plus SSHKeyAlgorithm and PredictMode, cut verbatim out of
# Moshpit/Models/SSHKeyStore.swift and AppSettings.swift (the files around
# them are UI state), FileUploader out of ImageAttachment.swift,
# ProtectedDataFlag and LaunchTrust out of Rust/RustCoreHosts.swift
# (HostKeyValidator's known-hosts store asks the one, its decisions in a Debug
# build the other), a stand-in for the UIKit names they and
# SSHService use, one for AppStores.knownHosts() (HostKeyValidator's default
# store; Rust/RustStoreSwitch.swift reaches every Rust store) as the app has
# it with the store switch off, the unit
# tests' in-process SSH server
# (Tests/MoshpitTests/Support/LoopbackSSHServer.swift, checked here against
# the same client code, with the PosixShell it reads commands with),
# FakeSOCKS.swift and main.swift. Swift 5 language mode, -O, as the app's
# Release build.
#
#   scripts/verify/ssh-live/build.sh [<output binary>]   (default target/ssh-live/ssh-live)
#   scripts/verify/ssh-live/build.sh --typecheck
#
# --typecheck stops at a typecheck: no release build, no binary out, and no
# Rust library (crates/seal/tests/harness_typecheck.rs).
#
# MOSHPIT_SPM_CHECKOUTS=<dir> names the checkouts directory (an Xcode
# DerivedData's SourcePackages/checkouts); by default every
# ~/Library/Developer/Xcode/DerivedData/*/SourcePackages/checkouts is searched
# for the pinned revisions. The first build compiles swift-nio and BoringSSL
# and takes a while; later ones reuse it. MOSHPIT_SSH_LIVE_WORK=<dir> works
# in <dir> instead of target/ssh-live.
set -euo pipefail
typecheck=
if [ "${1:-}" = --typecheck ]; then
    typecheck=1
    shift
fi
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
work="${MOSHPIT_SSH_LIVE_WORK:-$repo/target/ssh-live}"
out="${1:-$work/ssh-live}"
resolved="$repo/Moshpit.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
packages="citadel swift-nio-ssh swift-nio swift-crypto swift-asn1 swift-atomics swift-collections swift-system swift-log bigint swift-nio-extras"

pinned() {
    python3 -I -c 'import json,sys
pins = json.load(open(sys.argv[1]))["pins"]
print(next(p["state"]["revision"] for p in pins if p["identity"] == sys.argv[2]))' "$resolved" "$1"
}

checkout() {
    local identity="$1" want dir name
    want="$(pinned "$identity")"
    local dirs=()
    if [ -n "${MOSHPIT_SPM_CHECKOUTS:-}" ]; then
        dirs=("$MOSHPIT_SPM_CHECKOUTS")
    else
        for d in "$HOME"/Library/Developer/Xcode/DerivedData/*/SourcePackages/checkouts; do
            dirs+=("$d")
        done
    fi
    for d in "${dirs[@]}"; do
        for name in "$identity" Citadel BigInt; do
            dir="$d/$name"
            if [ -d "$dir" ] && [ "$(echo "$name" | tr 'A-Z' 'a-z')" = "$identity" ] \
                && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$want" ]; then
                echo "$dir"
                return
            fi
        done
    done
    echo "no checkout of $identity at $want (build the app once, or set MOSHPIT_SPM_CHECKOUTS)" >&2
    exit 1
}

mkdir -p "$work/pkgs"
for identity in $packages; do
    src="$(checkout "$identity")"
    dest="$work/pkgs/$identity"
    stamp="$dest/.moshpit-revision"
    if [ ! -f "$stamp" ] || [ "$(cat "$stamp")" != "$(pinned "$identity")" ]; then
        rm -rf "$dest"
        mkdir -p "$dest"
        (cd "$src" && tar --exclude .git -cf - .) | (cd "$dest" && tar -xf -)
        # SwiftPM leaves its checkouts read-only.
        chmod -R u+w "$dest"
        pinned "$identity" > "$stamp"
    fi
done

# Citadel and swift-nio-ssh name their dependencies by URL (and an example's,
# and a DocC plugin the build doesn't need): point them at the copies.
# swift-nio and swift-crypto do the same themselves under
# SWIFTCI_USE_LOCAL_DEPS.
python3 -I - "$work/pkgs" <<'EOF'
import re, sys, os
pkgs = sys.argv[1]

def local(text, names):
    for url_name, dir_name in names:
        text = re.sub(r'\.package\(\s*url:\s*"https://github\.com/[^"]+/%s(?:\.git)?"[^)]*\)' % re.escape(url_name),
                      '.package(path: "../%s")' % dir_name, text)
    return text

path = os.path.join(pkgs, "swift-nio-ssh", "Package.swift")
text = open(path).read()
text = re.sub(r'\s*\.package\(url: "https://github.com/apple/swift-docc-plugin"[^)]*\),', "", text)
text = local(text, [("swift-nio", "swift-nio"), ("swift-crypto", "swift-crypto"), ("swift-atomics", "swift-atomics")])
open(path, "w").write(text)

path = os.path.join(pkgs, "citadel", "Package.swift")
text = open(path).read()
text = re.sub(r'\s*\.package\(url: "https://github.com/mtynior/ColorizeSwift[^)]*\),', "", text)
text = local(text, [("swift-nio-ssh", "swift-nio-ssh"), ("swift-nio", "swift-nio"), ("swift-log", "swift-log"),
                    ("BigInt", "bigint"), ("swift-crypto", "swift-crypto")])
# Only the library: the example and the tests name what was just removed.
text = re.sub(r'\.executableTarget\(\s*name: "CitadelServerExample".*?\]\),', "", text, flags=re.S)
text = re.sub(r'\.testTarget\(\s*name: "CitadelTests".*?\]\s*\),', "", text, flags=re.S)
open(path, "w").write(text)
EOF

pkg="$work/swift"
sources="$pkg/Sources/ssh-live"
rm -rf "$pkg/Sources"
mkdir -p "$sources"
cp -R "$work/pkgs/swift-nio-extras/Sources/NIOSOCKS" "$pkg/Sources/NIOSOCKS"
for file in RSASHA2Signing SSHKeyFactory SOCKSProxyDialer SSHDial \
            SSHService HostKeyValidator KeychainService StoredKey Shell SSHExec SSHCommandPTY \
            HostConnection RemoteFileHandle; do
    cp "$repo/Moshpit/Services/$file.swift" "$sources/"
done
cp "$repo/Moshpit/Models/ServerConnection.swift" "$repo/Moshpit/Models/RemotePaths.swift" \
   "$repo/Moshpit/Models/StoredList.swift" "$repo/Moshpit/Services/Rust/Generated/moshpit_ffi.swift" "$sources/"
# The one MoshpitKit file the SSH code imports (Log), as the module it is in
# the app; and the UIKit names HostKeyValidator.swift and SSHService.swift
# use, which macOS lacks.
mkdir -p "$pkg/Sources/MoshpitKit" "$pkg/Sources/UIKit"
cp "$repo/Packages/MoshpitKit/Sources/MoshpitKit/Log.swift" "$pkg/Sources/MoshpitKit/"
cat > "$pkg/Sources/UIKit/UIKit.swift" <<'SWIFT'
// Stands in for UIKit on macOS: the names HostKeyValidator.swift (the unlock
// notification) and SSHService.swift (whether protected data is available)
// use.
import Foundation

public final class UIApplication: @unchecked Sendable {
    public static let shared = UIApplication()

    /// A Mac has no data-protection lock: protected data is always there.
    public var isProtectedDataAvailable: Bool { true }

    public static let protectedDataDidBecomeAvailableNotification =
        Notification.Name("UIApplicationProtectedDataDidBecomeAvailableNotification")

    public static let protectedDataWillBecomeUnavailableNotification =
        Notification.Name("UIApplicationProtectedDataWillBecomeUnavailableNotification")
}
SWIFT
cat > "$sources/AppStores.swift" <<'SWIFT'
// Stands in for AppStores (Moshpit/Services/Rust/RustStoreSwitch.swift, which
// makes every store on the Rust side or the Swift one): the trusted host keys
// as the app makes them with the store switch off. ssh-live's validators use
// stores of their own; this is HostKeyValidator's default argument.
enum AppStores {
    static func knownHosts() -> KnownHostsStore { KeychainKnownHostsStore.app() }
}
SWIFT
cp "$here/main.swift" "$here/FakeSOCKS.swift" "$repo/Tests/MoshpitTests/Support/LoopbackSSHServer.swift" \
   "$repo/Tests/MoshpitTests/Support/PosixShell.swift" "$sources/"

# Cuts the lines from the first that starts with $2 up to the first after it
# that is exactly $3, out of file $1; fails when either is missing.
cut_out() {
    awk -v start="$2" -v end="$3" '
        index($0, start) == 1 { on = 1 }
        on { print; if ($0 == end) { found = 1; exit } }
        END { if (!found) exit 1 }' "$1" || {
        echo "build.sh: no '$2' … '$3' in $1" >&2
        exit 1
    }
}
{
    echo "// Cut verbatim from Moshpit/Models/SSHKeyStore.swift by scripts/verify/ssh-live/build.sh."
    cut_out "$repo/Moshpit/Models/SSHKeyStore.swift" "enum SSHKeyAlgorithm: String" "}"
} > "$sources/SSHKeyAlgorithm.swift"
{
    echo "// Cut verbatim from Moshpit/Models/AppSettings.swift by scripts/verify/ssh-live/build.sh."
    cut_out "$repo/Moshpit/Models/AppSettings.swift" "enum PredictMode: String" "}"
} > "$sources/PredictMode.swift"
{
    echo "// Cut verbatim from Moshpit/Services/Rust/RustCoreHosts.swift by scripts/verify/ssh-live/build.sh."
    echo "import Foundation"
    echo "import UIKit"
    cut_out "$repo/Moshpit/Services/Rust/RustCoreHosts.swift" "final class ProtectedDataFlag: @unchecked Sendable {" "}"
    echo
    cut_out "$repo/Moshpit/Services/Rust/RustCoreHosts.swift" "enum LaunchTrust {" "}"
} > "$sources/RustCoreHosts.swift"
{
    echo "import Foundation"
    echo "// Cut verbatim from Moshpit/Services/ImageAttachment.swift by scripts/verify/ssh-live/build.sh."
    cut_out "$repo/Moshpit/Services/ImageAttachment.swift" "protocol FileUploader: Sendable" "}"
} > "$sources/FileUploader.swift"

cat > "$pkg/Package.swift" <<'EOF'
// swift-tools-version:6.0
// Generated by scripts/verify/ssh-live/build.sh.
import PackageDescription

let package = Package(
    name: "ssh-live",
    platforms: [.macOS(.v15)],
    dependencies: [
        .package(path: "../pkgs/citadel"),
        .package(path: "../pkgs/swift-nio-ssh"),
        .package(path: "../pkgs/swift-nio"),
        .package(path: "../pkgs/swift-crypto"),
    ],
    targets: [
        .target(
            name: "NIOSOCKS",
            dependencies: [
                .product(name: "NIO", package: "swift-nio"),
                .product(name: "NIOCore", package: "swift-nio"),
            ]
        ),
        .target(name: "MoshpitKit", swiftSettings: [.swiftLanguageMode(.v5)]),
        .target(name: "UIKit", swiftSettings: [.swiftLanguageMode(.v5)]),
        .executableTarget(
            name: "ssh-live",
            dependencies: [
                "NIOSOCKS",
                "MoshpitKit",
                "UIKit",
                .product(name: "Citadel", package: "citadel"),
                .product(name: "NIOSSH", package: "swift-nio-ssh"),
                .product(name: "NIOCore", package: "swift-nio"),
                .product(name: "NIOPosix", package: "swift-nio"),
                .product(name: "NIOConcurrencyHelpers", package: "swift-nio"),
                .product(name: "Crypto", package: "swift-crypto"),
                .product(name: "_CryptoExtras", package: "swift-crypto"),
            ],
            swiftSettings: [.swiftLanguageMode(.v5), .unsafeFlags(["-I", "@FFI_INCLUDE@"])],
            linkerSettings: [
                .unsafeFlags(["-L", "@RUST_LIB@", "-lmoshpit_ffi"]),
                .linkedFramework("Security"), .linkedFramework("SystemConfiguration"),
            ]
        ),
    ]
)
EOF
rust_lib="${CARGO_TARGET_DIR:-$repo/target}/debug"
python3 -I - "$pkg/Package.swift" "$repo/crates/ffi/include" "$rust_lib" <<'EOF'
import sys
path, include, lib = sys.argv[1:4]
text = open(path).read().replace("@FFI_INCLUDE@", include).replace("@RUST_LIB@", lib)
open(path, "w").write(text)
EOF

if [ -n "$typecheck" ]; then
    # SwiftPM links the rig even when it only prepares it: the Rust
    # library's symbols are left to a Debug binary nobody runs, so a
    # typecheck needs no library, nor one as new as the bindings.
    python3 -I - "$pkg/Package.swift" "$rust_lib" <<'EOF'
import sys
path, lib = sys.argv[1:3]
text = open(path).read()
link = '["-L", "%s", "-lmoshpit_ffi"]' % lib
if text.count(link) != 1:
    sys.exit("build.sh: no link of the Rust library in Package.swift")
open(path, "w").write(text.replace(link, '["-Xlinker", "-undefined", "-Xlinker", "dynamic_lookup"]'))
EOF
    # SwiftPM's module-only build: the packages' modules without their code
    # (minutes the first time, seconds after), the rig's own target
    # typechecked in full.
    (cd "$pkg" && SWIFTCI_USE_LOCAL_DEPS=1 nice -n 10 swift build --experimental-prepare-for-indexing -j 2 2>&1 \
        | awk '!/^\[[0-9]+ \/ [0-9]+\]/' | tail -40)
    exit
fi
(cd "$repo" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" nice -n 10 cargo build --quiet -p moshpit-ffi)

(cd "$pkg" && SWIFTCI_USE_LOCAL_DEPS=1 nice -n 10 swift build -c release -j 2 2>&1 | grep -E "error|warning: unre|Compiling ssh-live|Build complete" | tail -30)
mkdir -p "$(dirname "$out")"
# A fresh file, not an overwrite: macOS kills a binary rewritten in place
# whose code signature it has already cached.
rm -f "$out"
cp "$pkg/.build/release/ssh-live" "$out"
echo "$out"
