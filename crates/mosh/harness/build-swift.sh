#!/bin/bash
# Builds the Swift side of the mosh equivalence harness for macOS: the app's
# UNCHANGED Moshpit/Services/Mosh/{OCB3,MoshCrypto,MoshCompression,MoshWire,
# MoshTransport,MoshBootstrap}.swift plus harness/swift/{main,stubs}.swift.
# Same language mode and optimisation as the app's Release build
# (SWIFT_VERSION 5.9, -O, whole module).
#
#   harness/build-swift.sh [<output binary>]   (default target/mosh-harness/mosh-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/mosh-harness/mosh-swift}"
mosh="$repo/Moshpit/Services/Mosh"

# The stub's PATH list must stay the app's.
want="$(grep -A1 'static let extraPathDirs =' "$repo/Moshpit/Services/HostCapabilities.swift" | tail -1 | tr -d ' ')"
have="$(grep -A1 'static let extraPathDirs =' "$here/swift/stubs.swift" | tail -1 | tr -d ' ')"
if [ "$want" != "$have" ]; then
    echo "harness/swift/stubs.swift: extraPathDirs differs from HostCapabilities.swift" >&2
    exit 1
fi

mkdir -p "$(dirname "$out")"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -o "$out" \
    "$mosh/OCB3.swift" "$mosh/MoshCrypto.swift" "$mosh/MoshCompression.swift" \
    "$mosh/MoshWire.swift" "$mosh/MoshTransport.swift" "$mosh/MoshBootstrap.swift" \
    "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
