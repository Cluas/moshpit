#!/bin/bash
# Builds the Swift sides of the store's equivalence harness for macOS:
#
#   store-swift       the app's storage sources as checked out here, with
#                     harness/swift/main.swift, the stores on their own Swift
#                     code (harness/swift/Stores.swift) — but the shortcuts,
#                     which the app keeps on the Rust store whatever its
#                     switch says (b85f84d1), so this one links the Rust
#                     library and the app's bindings too, as below;
#   store-swift-rust  the same, with the same main.swift, the stores on the
#                     app's Rust-store adapters (harness/adapters/Stores.swift
#                     over Moshpit/Services/Rust/RustStores.swift, UNCHANGED,
#                     and the app's UniFFI bindings, Services/Rust/Generated)
#                     linked against the Rust library built for this Mac
#                     (target/debug/libmoshpit_ffi.a, `cargo build -p
#                     moshpit-ffi`) — the app with MOSHPIT_RUST_STORE on;
#   store-swift-108   the same sources as 1.0.8 shipped them (the release
#                     commit 8c10490a, read out of git), with
#                     harness/swift108/main.swift.
#
# Each is two modules, built with the app's language mode and optimisation
# (Swift 5, -O, whole module): MoshpitKit from the package's UNCHANGED
# Log.swift, AppLanguage.swift (not in 1.0.8) and
# Push/{PushPairing,PushPrefs,PushSealedBox}.swift, and the app module from
# these UNCHANGED files —
#
#   Models/ServerConnection.swift, Models/ConnectionStore.swift,
#   Models/StoredList.swift (not in 1.0.8), Models/SSHKeyStore.swift,
#   Models/ThemeStore.swift, Models/AppThemeStore.swift,
#   Models/AppSettings.swift, Services/KeychainService.swift, UI/AppTheme.swift
#   (1.0.8's; the current one is cut, below), and, but in 1.0.8,
#   Services/Rust/RustStores.swift and the app's UniFFI bindings
#   (Services/Rust/Generated/moshpit_ffi.swift)
#
# — and, where a file imports UIKit (or Citadel/NIOSSH), pieces of it cut
# out verbatim, nothing edited inside a line:
#
#   Models/DefaultsReadiness.swift     all of it but `import UIKit` and the
#                                      `static let system` member, which asks
#                                      UIApplication (a stand-in that says
#                                      "readable" replaces it; the harness
#                                      never uses it: every store gets its own
#                                      readiness)
#   Models/TerminalShortcut.swift      all of it but `import UIKit` (SwiftUI,
#                                      which has the `move(fromOffsets:)` it
#                                      uses, is imported instead)
#   Models/TerminalTheme.swift         all of it but `import UIKit` and
#                                      `var hexString`, which goes through
#                                      UIColor
#   UI/AppTheme.swift                  (but in 1.0.8) all of it but its
#                                      `extension View`, `appThemeRoot()`,
#                                      which tints the app with
#                                      UI/DesignTokens.swift's `Ink`
#   Services/Voice/VoiceLanguage.swift `enum VoiceEngineKind` (AppSettings
#   Services/Voice/VoiceDictation.swift `struct DictationRequest`  uses both)
#   Services/SSHKeyFactory.swift       `rsaBits(ofPublicKeyLine:)` (not in
#                                      1.0.8; it asks the Rust key reader)
#   Services/HostKeyValidator.swift    `protocol KnownHostsStore`,
#                                      `struct KnownHostsUnreadable` (not in
#                                      1.0.8) and `class KeychainKnownHostsStore`
#   Services/Rust/RustCoreHosts.swift  (but in 1.0.8) `class
#                                      RustPreferencesHost`, `enum PlistNodes`,
#                                      `class RustKeychainHost` and `enum
#                                      LaunchTrust` — the hosts the adapters
#                                      and the shortcuts sit on; a stand-in
#                                      for `ProtectedDataFlag`, which asks
#                                      UIApplication, is in
#                                      harness/swift/RustHosts.swift
#
# One more harness file but in 1.0.8: harness/swift/ShortcutJSON.swift, how
# a case's shortcut JSON is read (the app's decoder until the stored form
# moved into the Rust store, b85f84d1).
#
#   harness/build-swift.sh [<output dir>]   (default target/store-harness)
#
# crates/seal/tests/harness_typecheck.rs runs this script with every swiftc
# call typechecked instead of built (no Rust library needed), so a change to
# the app's sources that stops the harness building fails a test.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
work="${1:-$repo/target/store-harness}"
release_108=8c10490a
mkdir -p "$work"

# The cuts, done by one small script: `keep` copies a file without the
# listed lines and blocks, `cut` prints the blocks alone. A block runs from
# the first line starting with START to the next line that is exactly END;
# a missing one fails the build.
cutter="$work/cut.py"
cat > "$cutter" <<'EOF'
import sys

def blocks(lines, start, end):
    for i, line in enumerate(lines):
        if line.startswith(start):
            for j in range(i + 1, len(lines)):
                if lines[j] == end:
                    return i, j
            break
    sys.exit("build-swift.sh: no '%s' ... '%s' in %s" % (start, end, sys.argv[2]))

mode, path = sys.argv[1], sys.argv[2]
specs = sys.argv[3:]
lines = open(path, encoding="utf-8").read().split("\n")
if mode == "keep":
    drop = set()
    for spec in specs:
        if spec.startswith("line:"):
            target = spec[5:]
            hits = [i for i, l in enumerate(lines) if l == target]
            if not hits:
                sys.exit("build-swift.sh: no line '%s' in %s" % (target, path))
            drop.update(hits)
        else:
            start, end = spec.split("\x1f")
            i, j = blocks(lines, start, end)
            drop.update(range(i, j + 1))
    print("\n".join(l for i, l in enumerate(lines) if i not in drop))
else:
    for spec in specs:
        start, end = spec.split("\x1f")
        i, j = blocks(lines, start, end)
        print("\n".join(lines[i:j + 1]))
EOF
US=$'\x1f'
keep() { python3 -I "$cutter" keep "$@"; }
cut() { python3 -I "$cutter" cut "$@"; }

# build <source root> <main.swift> <output binary> <108|current|adapters>
build() {
    local root="$1" main="$2" binary="$3" version="$4"
    local dir="$work/build-$version"
    rm -rf "$dir"
    mkdir -p "$dir/kit" "$dir/app"
    local kit="$root/Packages/MoshpitKit/Sources/MoshpitKit"
    local kit_files=("$kit/Log.swift" "$kit/Push/PushPairing.swift" "$kit/Push/PushPrefs.swift"
        "$kit/Push/PushSealedBox.swift")
    [ "$version" != 108 ] && kit_files+=("$kit/AppLanguage.swift")
    nice -n 10 swiftc -O -wmo -swift-version 5 -parse-as-library -module-name MoshpitKit \
        -emit-module -emit-module-path "$dir/kit/MoshpitKit.swiftmodule" \
        -emit-library -static -o "$dir/kit/libMoshpitKit.a" "${kit_files[@]}"

    local m="$root/Moshpit" app="$dir/app"
    local files=(
        "$m/Models/ServerConnection.swift" "$m/Models/ConnectionStore.swift" "$m/Models/SSHKeyStore.swift"
        "$m/Models/ThemeStore.swift" "$m/Models/AppThemeStore.swift" "$m/Models/AppSettings.swift"
        "$m/Services/KeychainService.swift"
    )
    local swiftc_rust=()
    if [ "$version" = 108 ]; then
        files+=("$m/UI/AppTheme.swift")
    else
        files+=("$m/Models/StoredList.swift" "$m/Services/Rust/RustStores.swift"
            "$m/Services/Rust/Generated/moshpit_ffi.swift")
        swiftc_rust=(-I "$root/crates/ffi/include" -L "$root/target/debug" -lmoshpit_ffi)
    fi

    {
        echo "// Cut verbatim from Moshpit/Models/DefaultsReadiness.swift by harness/build-swift.sh."
        keep "$m/Models/DefaultsReadiness.swift" "line:import UIKit" \
            "    static let system = DefaultsReadiness {${US}    }"
        echo "// The harness's stand-in for the member that asks UIApplication."
        echo "extension DefaultsReadiness { static let system = DefaultsReadiness { true } }"
    } > "$app/DefaultsReadiness.swift"
    {
        echo "// Cut verbatim from Moshpit/Models/TerminalShortcut.swift by harness/build-swift.sh."
        echo "import SwiftUI"
        keep "$m/Models/TerminalShortcut.swift" "line:import UIKit"
    } > "$app/TerminalShortcut.swift"
    {
        echo "// Cut verbatim from Moshpit/Models/TerminalTheme.swift by harness/build-swift.sh."
        keep "$m/Models/TerminalTheme.swift" "line:import UIKit" "    var hexString: String {${US}    }"
    } > "$app/TerminalTheme.swift"
    {
        echo "// Cut verbatim from Moshpit/Services/Voice by harness/build-swift.sh."
        echo "import Foundation"
        cut "$m/Services/Voice/VoiceLanguage.swift" "enum VoiceEngineKind: String${US}}"
        cut "$m/Services/Voice/VoiceDictation.swift" "struct DictationRequest: ${US}}"
    } > "$app/Voice.swift"
    {
        echo "// Cut verbatim from Moshpit/Services/HostKeyValidator.swift by harness/build-swift.sh."
        echo "import Foundation"
        echo "import MoshpitKit"
        if [ "$version" != 108 ]; then
            cut "$m/Services/HostKeyValidator.swift" "protocol KnownHostsStore: ${US}}" \
                "struct KnownHostsUnreadable: ${US}}" "final class KeychainKnownHostsStore: ${US}}"
        else
            cut "$m/Services/HostKeyValidator.swift" "protocol KnownHostsStore: ${US}}" \
                "final class KeychainKnownHostsStore: ${US}}"
        fi
    } > "$app/KnownHosts.swift"
    if [ "$version" != 108 ]; then
        {
            echo "// Cut verbatim from Moshpit/UI/AppTheme.swift by harness/build-swift.sh."
            keep "$m/UI/AppTheme.swift" "extension View {${US}}"
        } > "$app/AppTheme.swift"
        {
            echo "// Cut verbatim from Moshpit/Services/Rust/RustCoreHosts.swift by harness/build-swift.sh."
            echo "import Foundation"
            cut "$m/Services/Rust/RustCoreHosts.swift" \
                "final class RustPreferencesHost: ${US}}" "enum PlistNodes {${US}}" \
                "final class RustKeychainHost: ${US}}" "enum LaunchTrust {${US}}"
        } > "$app/RustCoreHosts.swift"
    fi
    if [ "$version" != 108 ]; then
        {
            echo "// Cut verbatim from Moshpit/Services/SSHKeyFactory.swift by harness/build-swift.sh."
            echo "import Foundation"
            echo "enum SSHKeyFactory {"
            cut "$m/Services/SSHKeyFactory.swift" "    static func rsaBits(ofPublicKeyLine line: String) -> Int? {${US}    }"
            echo "}"
        } > "$app/SSHKeyFactory.swift"
    fi
    cp "$main" "$app/main.swift"
    cp "$here/swift/Common.swift" "$app/Common.swift"
    if [ "$version" != 108 ]; then
        cp "$here/swift/RustHosts.swift" "$app/RustHosts.swift"
        cp "$here/swift/ShortcutJSON.swift" "$app/ShortcutJSON.swift"
    fi
    case "$version" in
        current) cp "$here/swift/Stores.swift" "$app/Stores.swift" ;;
        adapters) cp "$here/adapters/Stores.swift" "$app/Stores.swift" ;;
    esac

    nice -n 10 swiftc -O -wmo -swift-version 5 -parse-as-library -module-name Moshpit \
        -I "$dir/kit" -L "$dir/kit" -lMoshpitKit ${swiftc_rust[@]+"${swiftc_rust[@]}"} \
        -o "$binary" "${files[@]}" "$app"/*.swift
}

build "$repo" "$here/swift/main.swift" "$work/store-swift" current
build "$repo" "$here/swift/main.swift" "$work/store-swift-rust" adapters

src108="$work/src108"
rm -rf "$src108"
mkdir -p "$src108"
git -C "$repo" archive "$release_108" Moshpit Packages/MoshpitKit | tar -x -C "$src108"
build "$src108" "$here/swift108/main.swift" "$work/store-swift-108" 108

echo "$work/store-swift"
echo "$work/store-swift-rust"
echo "$work/store-swift-108"
