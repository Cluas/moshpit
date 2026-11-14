#!/bin/bash
# Builds the Swift side of the G2 equivalence harness for macOS: the app's
# UNCHANGED Moshpit/Services/Tmux/TmuxControlClient.swift plus
# harness/swift/main.swift. Same language mode and optimisation as the app's
# Release build (SWIFT_VERSION 5.9, -O, whole module).
#
#   harness/build-swift.sh [<output binary>]   (default target/g2/cc-events-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/g2/cc-events-swift}"
mkdir -p "$(dirname "$out")"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -o "$out" \
    "$repo/Moshpit/Services/Tmux/TmuxControlClient.swift" \
    "$here/swift/main.swift"
echo "$out"
