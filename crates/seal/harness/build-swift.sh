#!/bin/bash
# Builds the Swift side of the seal equivalence harness for macOS: the app's
# UNCHANGED MoshpitKit push sources (Log, Push/*, and the two Island files
# PushRemoteNotification names) plus harness/swift/main.swift, in one
# module. Same language mode and optimisation as the app's Release build
# (SWIFT_VERSION 5.9, -O, whole module).
#
# The sources are kept in harness/swift/frozen/, read from nowhere else: E7b
# put the notification service on this crate and deleted them from the app,
# and the commits that last had them are in no public clone (the history
# was folded on export), so a `git show` of them worked in one clone only.
# As the app last ran them, B639's bidi strip included (2f406630) — but
# Push/PushPairing.swift, as of da2146f1: the app's own writer of the
# pairing file went with batch C (the app writes it through the Rust store,
# crates/seal), and the harness's cases write it on the Swift side too, as
# it was. Nothing here is edited: the harness is held to the code that ran.
#
#   harness/build-swift.sh [<output binary>]   (default target/seal-harness/seal-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/seal-harness/seal-swift}"
kit="$here/swift/frozen"
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
