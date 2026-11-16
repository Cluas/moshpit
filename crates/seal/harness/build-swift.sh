#!/bin/bash
# Builds the Swift side of the seal equivalence harness for macOS: the app's
# UNCHANGED MoshpitKit push sources (Log, Push/*, and the two Island files
# PushRemoteNotification names) as of the last commit that had them (below)
# plus harness/swift/main.swift, in one module. Same language mode and optimisation as the app's Release build
# (SWIFT_VERSION 5.9, -O, whole module).
#
# But Push/PushPairing.swift, read as of $MOSHPIT_SWIFT_SEAL_PAIRING_REV
# (default da2146f1): the app's own writer of the pairing file went with
# batch C (the app writes it through the Rust store, crates/seal), and the
# harness's cases write it on the Swift side too, as it was.
#
#   harness/build-swift.sh [<output binary>]   (default target/seal-harness/seal-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/seal-harness/seal-swift}"
rev="${MOSHPIT_SWIFT_SEAL_PAIRING_REV:-da2146f1}"
mkdir -p "$(dirname "$out")"
# MoshpitKit's push sources (Log, AppLanguage, Push/*, the two Island files
# PushRemoteNotification names) as the app last ran them: E7b moved the
# notification service onto crates/seal and deleted Push/* (8ef981c5, or
# $MOSHPIT_SWIFT_PUSH_REV).
push_rev="${MOSHPIT_SWIFT_PUSH_REV:-8ef981c5}"
kit="$(dirname "$out")/push-kit"
mkdir -p "$kit/Push" "$kit/Island"
for name in Log AppLanguage Push/PushSealedBox Push/PushStanding Push/AppGroupFileLock \
    Push/AgentNotificationCopy Push/PushRemoteNotification Push/PushPrefs Push/PushDiagnostics \
    Island/AgentNotifications Island/AgentControlBridge; do
    git -C "$repo" show "$push_rev:Packages/MoshpitKit/Sources/MoshpitKit/$name.swift" >"$kit/$name.swift"
done
pairing="$(dirname "$out")/PushPairing.swift"
git -C "$repo" show "$rev:Packages/MoshpitKit/Sources/MoshpitKit/Push/PushPairing.swift" >"$pairing"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -o "$out" \
    "$kit/Log.swift" "$kit/AppLanguage.swift" \
    "$kit/Push/PushSealedBox.swift" "$pairing" "$kit/Push/PushStanding.swift" \
    "$kit/Push/AppGroupFileLock.swift" \
    "$kit/Push/AgentNotificationCopy.swift" "$kit/Push/PushRemoteNotification.swift" \
    "$kit/Push/PushPrefs.swift" "$kit/Push/PushDiagnostics.swift" \
    "$kit/Island/AgentNotifications.swift" "$kit/Island/AgentControlBridge.swift" \
    "$here/swift/main.swift"
echo "$out"
