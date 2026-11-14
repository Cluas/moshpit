#!/bin/bash
# Builds the Swift side of the ssh cross-check for macOS: a SwiftPM package
# over copies of the packages the app pins (swift-nio-ssh — the Wellz26 fork
# Citadel uses — swift-nio, swift-crypto and theirs), taken by revision from
# an Xcode SourcePackages checkout of the app and never modified there, with
#
#   - the app's UNCHANGED Moshpit/Services/SSHKeyFactory.swift with the key
#     reading it calls (SSHPrivateKeyFile.swift, BcryptPBKDF.swift), as of
#     81c121a1, the last commit before they moved to Rust,
#     Moshpit/Services/Shell.swift (ExecUploadCommands quotes with it), and
#     Moshpit/Services/SOCKSProxyDialer.swift, the latter over NIOSOCKS's
#     sources from the swift-nio-extras the app pins (as a target of its own:
#     the whole package needs a dozen more);
#   - Citadel's UNCHANGED Sources/Citadel/SSHKeyTypeDetection.swift (the
#     detection SSHService.makeAuthMethod runs before every key sign-in);
#   - HostKeyValidator.fingerprint(of:), cut verbatim out of the app's
#     HostKeyValidator.swift, the SSHKeyAlgorithm and ExecUploadCommands
#     enums, cut verbatim out of SSHKeyStore.swift and SSHService.swift, and
#     the InboundHold handler SOCKSProxyDialer sets up, cut verbatim out of
#     SSHDial.swift (the files around them import UIKit and Citadel);
#   - harness/swift/main.swift.
#
# Same language mode and optimisation as the app's Release build (Swift 5,
# -O). The first build compiles swift-nio and takes minutes; later ones
# reuse it.
#
#   harness/build-swift.sh [<output binary>]   (default target/ssh-harness/ssh-swift)
#   harness/build-swift.sh --typecheck
#
# --typecheck stops at a typecheck: no binary, no release build
# (crates/seal/tests/harness_typecheck.rs).
#
# MOSHPIT_SPM_CHECKOUTS=<dir> names the checkouts directory to take the
# packages from; by default every DerivedData/*/SourcePackages/checkouts is
# searched for the pinned revisions. MOSHPIT_SSH_HARNESS_WORK=<dir> works in
# <dir> instead of target/ssh-harness.
set -euo pipefail
typecheck=
if [ "${1:-}" = --typecheck ]; then
    typecheck=1
    shift
fi
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
work="${MOSHPIT_SSH_HARNESS_WORK:-$repo/target/ssh-harness}"
out="${1:-$work/ssh-swift}"
resolved="$repo/Moshpit.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
packages="swift-nio-ssh swift-nio swift-crypto swift-atomics swift-collections swift-system swift-asn1 citadel swift-nio-extras"

pinned() {
    python3 -I -c 'import json,sys
pins = json.load(open(sys.argv[1]))["pins"]
print(next(p["state"]["revision"] for p in pins if p["identity"] == sys.argv[2]))' "$resolved" "$1"
}

checkout() {
    local identity="$1" want dir
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
        for name in "$identity" "$(echo "$identity" | sed 's/^citadel$/Citadel/')"; do
            dir="$d/$name"
            if [ -d "$dir" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$want" ]; then
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

# swift-nio-ssh names its dependencies by URL (and the DocC plugin, which
# the build doesn't need); point them at the copies. swift-nio and
# swift-crypto do the same themselves under SWIFTCI_USE_LOCAL_DEPS.
manifest="$work/pkgs/swift-nio-ssh/Package.swift"
python3 -I - "$manifest" <<'EOF'
import re, sys
path = sys.argv[1]
text = open(path).read()
text = re.sub(r'\s*\.package\(url: "https://github.com/apple/swift-docc-plugin"[^)]*\),', "", text)
for name in ("swift-nio", "swift-crypto", "swift-atomics"):
    text = re.sub(r'\.package\(url: "https://github.com/apple/%s\.git",[^)]*\)' % name,
                  '.package(path: "../%s")' % name, text)
open(path, "w").write(text)
EOF

# The harness package, with its sources assembled.
pkg="$work/swift"
sources="$pkg/Sources/ssh-swift"
rm -rf "$pkg/Sources"
mkdir -p "$sources"
cp -R "$work/pkgs/swift-nio-extras/Sources/NIOSOCKS" "$pkg/Sources/NIOSOCKS"
# The app's Swift key reader moved to Rust (crates/ssh/src/keyfile); the
# cross-check holds Rust to the Swift it replaced, as of the last commit
# that had it.
swift_keys_rev=81c121a1
for file in SSHKeyFactory SSHPrivateKeyFile BcryptPBKDF; do
    git -C "$repo" show "$swift_keys_rev:Moshpit/Services/$file.swift" > "$sources/$file.swift"
done
git -C "$repo" show "$swift_keys_rev:Moshpit/Models/SSHKeyStore.swift" > "$work/SSHKeyStore-$swift_keys_rev.swift"
cp "$repo/Moshpit/Services/Shell.swift" "$sources/"
cp "$repo/Moshpit/Services/SOCKSProxyDialer.swift" "$sources/"
cp "$work/pkgs/citadel/Sources/Citadel/SSHKeyTypeDetection.swift" "$sources/"
cp "$here/swift/main.swift" "$sources/"

# Cuts the lines from the first that starts with $2 up to the first after it
# that is exactly $3, out of file $1; fails when either is missing.
cut_out() {
    awk -v start="$2" -v end="$3" '
        index($0, start) == 1 { on = 1 }
        on { print; if ($0 == end) { found = 1; exit } }
        END { if (!found) exit 1 }' "$1" || {
        echo "build-swift.sh: no '$2' … '$3' in $1" >&2
        exit 1
    }
}
{
    echo "// Cut verbatim from Moshpit/Services/HostKeyValidator.swift by harness/build-swift.sh."
    echo "import Foundation"
    echo "import Crypto"
    echo "import NIOCore"
    echo "import NIOSSH"
    echo
    echo "enum HostKeyValidator {"
    cut_out "$repo/Moshpit/Services/HostKeyValidator.swift" \
        "    static func fingerprint(of key: NIOSSHPublicKey) -> String {" "    }"
    echo "}"
} > "$sources/HostKeyFingerprint.swift"
{
    echo "// Cut verbatim from Moshpit/Models/SSHKeyStore.swift by harness/build-swift.sh."
    cut_out "$work/SSHKeyStore-$swift_keys_rev.swift" "enum SSHKeyAlgorithm: String" "}"
} > "$sources/SSHKeyAlgorithm.swift"
{
    echo "// Cut verbatim from Moshpit/Services/SSHService.swift by harness/build-swift.sh."
    echo "import Foundation"
    echo
    cut_out "$repo/Moshpit/Services/SSHService.swift" "enum ExecUploadCommands {" "}"
} > "$sources/ExecUploadCommands.swift"
{
    echo "// Cut verbatim from Moshpit/Services/SSHDial.swift by harness/build-swift.sh."
    echo "import NIOCore"
    echo
    cut_out "$repo/Moshpit/Services/SSHDial.swift" "final class InboundHold:" "}"
} > "$sources/InboundHold.swift"

cat > "$pkg/Package.swift" <<'EOF'
// swift-tools-version:5.10
// Generated by crates/ssh/harness/build-swift.sh.
import PackageDescription

let package = Package(
    name: "ssh-harness",
    platforms: [.macOS(.v14)],
    dependencies: [
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
        .executableTarget(
            name: "ssh-swift",
            dependencies: [
                "NIOSOCKS",
                .product(name: "NIOSSH", package: "swift-nio-ssh"),
                .product(name: "NIOConcurrencyHelpers", package: "swift-nio"),
                .product(name: "NIOCore", package: "swift-nio"),
                .product(name: "NIOPosix", package: "swift-nio"),
                .product(name: "Crypto", package: "swift-crypto"),
            ]
        ),
    ]
)
EOF

if [ -n "$typecheck" ]; then
    # SwiftPM's module-only build: the packages' modules without their code
    # (minutes the first time, seconds after), the rig's own target
    # typechecked in full.
    (cd "$pkg" && SWIFTCI_USE_LOCAL_DEPS=1 nice -n 10 swift build --experimental-prepare-for-indexing -j 2 2>&1 \
        | awk '!/^\[[0-9]+ \/ [0-9]+\]/' | tail -40)
    exit
fi
(cd "$pkg" && SWIFTCI_USE_LOCAL_DEPS=1 nice -n 10 swift build -c release -j 2 2>&1 | tail -20)
mkdir -p "$(dirname "$out")"
cp "$pkg/.build/release/ssh-swift" "$out"
echo "$out"
