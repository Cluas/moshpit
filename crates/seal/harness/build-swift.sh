#!/bin/bash
# Builds the Swift side of the seal equivalence harness for macOS: the app's
# UNCHANGED MoshpitKit push sources (Log, Push/*, and the two Island files
# PushRemoteNotification names) plus harness/swift/main.swift, in one
# module. Same language mode and optimisation as the app's Release build
# (SWIFT_VERSION 5.9, -O, whole module).
#
#   harness/build-swift.sh [<output binary>]   (default target/seal-harness/seal-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/seal-harness/seal-swift}"
kit="$repo/Packages/MoshpitKit/Sources/MoshpitKit"
mkdir -p "$(dirname "$out")"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -o "$out" \
    "$kit/Log.swift" "$kit/AppLanguage.swift" \
    "$kit/Push/PushSealedBox.swift" "$kit/Push/PushPairing.swift" "$kit/Push/PushStanding.swift" \
    "$kit/Push/AppGroupFileLock.swift" \
    "$kit/Push/AgentNotificationCopy.swift" "$kit/Push/PushRemoteNotification.swift" \
    "$kit/Push/PushPrefs.swift" "$kit/Push/PushDiagnostics.swift" \
    "$kit/Island/AgentNotifications.swift" "$kit/Island/AgentControlBridge.swift" \
    "$here/swift/main.swift"
echo "$out"
