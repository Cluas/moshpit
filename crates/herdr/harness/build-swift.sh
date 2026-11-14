#!/bin/bash
# Builds the Swift side of the herdr equivalence harness for macOS: the app's
# UNCHANGED herdr sources (Moshpit/Services/Herdr/*.swift) and what they
# need (MultiplexerControlling.swift, TmuxTypes.swift, WriteLane.swift,
# AgentTaskRequest.swift, Shell.swift, BracketedPaste.swift, MoshpitKit's
# Log.swift as module MoshpitKit),
# verbatim copies of SessionHub's two frame-input statics and AgentHook
# (harness/extract.py), harness/swift/stubs.swift and harness/swift/main.swift.
# Same language mode and optimisation as the app's Release build
# (SWIFT_VERSION 5.9, -O, whole module).
#
#   harness/build-swift.sh [<output binary>]   (default target/herdr-harness/herdr-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/herdr-harness/herdr-swift}"
work="$(dirname "$out")/swift-build"
mkdir -p "$work/kit"

# The stub's PATH list must stay the app's.
want="$(grep -A1 'static let extraPathDirs =' "$repo/Moshpit/Services/HostCapabilities.swift" | tail -1 | tr -d ' ')"
have="$(grep -A1 'static let extraPathDirs =' "$here/swift/stubs.swift" | tail -1 | tr -d ' ')"
if [ "$want" != "$have" ]; then
    echo "harness/swift/stubs.swift: extraPathDirs differs from HostCapabilities.swift" >&2
    exit 1
fi

python3 "$here/extract.py" "$work/extract.swift"

swiftc -O -wmo -swift-version 5 -parse-as-library \
    -emit-library -static -emit-module -module-name MoshpitKit \
    -emit-module-path "$work/kit/MoshpitKit.swiftmodule" \
    -o "$work/kit/libMoshpitKit.a" \
    "$repo/Packages/MoshpitKit/Sources/MoshpitKit/Log.swift"

herdr="$repo/Moshpit/Services/Herdr"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -I "$work/kit" -L "$work/kit" -lMoshpitKit \
    -o "$out" \
    "$herdr/HerdrFrameChannel.swift" "$herdr/HerdrSnapshot.swift" \
    "$herdr/HerdrSocketClient.swift" "$herdr/HerdrPushDriver.swift" \
    "$herdr/HerdrLaunch.swift" "$herdr/HerdrControlClient.swift" \
    "$repo/Moshpit/Services/MultiplexerControlling.swift" \
    "$repo/Moshpit/Services/Tmux/TmuxTypes.swift" \
    "$repo/Moshpit/Services/WriteLane.swift" \
    "$repo/Moshpit/Models/AgentTaskRequest.swift" \
    "$repo/Moshpit/Services/Shell.swift" "$repo/Moshpit/Services/BracketedPaste.swift" \
    "$work/extract.swift" "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
