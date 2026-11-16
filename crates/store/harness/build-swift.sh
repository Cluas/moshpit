#!/bin/bash
# Builds the Swift side of the store's equivalence harness for macOS:
#
#   store-swift-108   the app's storage sources as 1.0.8 shipped them (the
#                     release commit 8c10490a), with
#                     harness/swift108/main.swift and Common.swift.
#
# The sources are kept in harness/swift108/frozen, at their paths in the
# release and unedited, read from nowhere else: no public clone has the
# release commit (the history was folded on export), so a `git archive` of
# it built the harness in one clone only. Only the files the build reads
# are kept.
#
# The app's own Swift stores are gone (batch C); they had two front ends
# here until then. 1.0.8's is the upgrade guard: what this crate writes,
# 1.0.8 reads; what 1.0.8 writes, this crate reads (equivalence.py).
#
# Two modules, built with the app's language mode and optimisation (Swift
# 5, -O, whole module): MoshpitKit from 1.0.8's Log.swift and
# Push/{PushPairing,PushPrefs,PushSealedBox}.swift, and the app module from
# 1.0.8's
#
#   Models/ServerConnection.swift, Models/ConnectionStore.swift,
#   Models/SSHKeyStore.swift, Models/ThemeStore.swift,
#   Models/AppThemeStore.swift, Models/AppSettings.swift,
#   Services/KeychainService.swift, UI/AppTheme.swift
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
#   Services/Voice/VoiceLanguage.swift `enum VoiceEngineKind` (AppSettings
#   Services/Voice/VoiceDictation.swift `struct DictationRequest`  uses both)
#   Services/HostKeyValidator.swift    `protocol KnownHostsStore` and `class
#                                      KeychainKnownHostsStore`
#
#   harness/build-swift.sh [<output dir>]   (default target/store-harness)
#
# crates/seal/tests/harness_typecheck.rs runs this script with every swiftc
# call typechecked instead of built, so a change that stops the harness
# building fails a test.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
work="${1:-$repo/target/store-harness}"
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

# build <source root>: 1.0.8's sources (harness/swift108/frozen).
build() {
    local root="$1"
    local dir="$work/build-108" binary="$work/store-swift-108"
    rm -rf "$dir"
    mkdir -p "$dir/kit" "$dir/app"
    local kit="$root/Packages/MoshpitKit/Sources/MoshpitKit"
    nice -n 10 swiftc -O -wmo -swift-version 5 -parse-as-library -module-name MoshpitKit \
        -emit-module -emit-module-path "$dir/kit/MoshpitKit.swiftmodule" \
        -emit-library -static -o "$dir/kit/libMoshpitKit.a" \
        "$kit/Log.swift" "$kit/Push/PushPairing.swift" "$kit/Push/PushPrefs.swift" "$kit/Push/PushSealedBox.swift"

    local m="$root/Moshpit" app="$dir/app"
    local files=(
        "$m/Models/ServerConnection.swift" "$m/Models/ConnectionStore.swift" "$m/Models/SSHKeyStore.swift"
        "$m/Models/ThemeStore.swift" "$m/Models/AppThemeStore.swift" "$m/Models/AppSettings.swift"
        "$m/Services/KeychainService.swift" "$m/UI/AppTheme.swift"
    )
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
        cut "$m/Services/HostKeyValidator.swift" "protocol KnownHostsStore: ${US}}" \
            "final class KeychainKnownHostsStore: ${US}}"
    } > "$app/KnownHosts.swift"
    cp "$here/swift108/main.swift" "$app/main.swift"
    cp "$here/swift108/Common.swift" "$app/Common.swift"

    nice -n 10 swiftc -O -wmo -swift-version 5 -parse-as-library -module-name Moshpit \
        -I "$dir/kit" -L "$dir/kit" -lMoshpitKit -o "$binary" "${files[@]}" "$app"/*.swift
}

build "$here/swift108/frozen"

echo "$work/store-swift-108"
