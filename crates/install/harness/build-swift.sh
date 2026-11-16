#!/bin/bash
# Builds the Swift side of the install equivalence harness for macOS: the
# MoshpitKit sources the install code imports (Log, Push/*, the two Island
# files PushRemoteNotification names) as the module MoshpitKit, then the
# app's UNCHANGED Moshpit/Services/Install/{HostScripts,InstallManifest,
# HostChannel,HostInstaller,AgentHookConfig}.swift and its Swift capability
# probe, with Tests/MoshpitTests/Support/Shell.swift (HostCommands.quote,
# E7a moved it there) and harness/swift/{stubs,main}.swift. Same language
# mode and optimisation as the app's Release build (SWIFT_VERSION 5.9, -O,
# whole module).
#
# The app's sources are kept in harness/swift/frozen, read from nowhere
# else: they are gone from the app, and the commits that last had them are
# in no public clone (the history was folded on export), so a `git show` of
# them built the harness in one clone only. Unedited, as the app last ran
# them:
#
#   Install/*.swift        81c121a1, before the installer moved to the core
#   HostCapabilities.swift e57f7941, before E6a batch 2a handed the probe,
#                          its parser, the PATH dirs and the install commands
#                          to the core
#   kit/Push/PushPairing   81c121a1 (HostInstaller mints its self-test's name
#                          with its helpers; the app's went with batch C)
#   kit/ (the rest)        8ef981c5, before E7b put the notification service
#                          on crates/seal and deleted Push/*
#
# The host-care cross-check (crates/hostcare/harness) builds on the same.
#
#   harness/build-swift.sh [<output binary>]   (default target/install-harness/install-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/install-harness/install-swift}"
work="$(dirname "$out")/kit"
frozen="$here/swift/frozen"
install="$frozen/Install"
kit="$frozen/kit"

# The stub's Multiplexer must keep the app's cases.
want="$(sed -n '/^enum Multiplexer/,/^}/p' "$repo/Moshpit/Models/ServerConnection.swift" | grep -E '^ *case [a-z]+$' | tr -d ' ')"
have="$(sed -n '/^enum Multiplexer/,/^}/p' "$here/swift/stubs.swift" | grep -E '^ *case [a-z]+$' | tr -d ' ')"
if [ "$want" != "$have" ]; then
    echo "harness/swift/stubs.swift: Multiplexer's cases differ from Models/ServerConnection.swift" >&2
    exit 1
fi

mkdir -p "$work"
swiftc -O -wmo -swift-version 5 -parse-as-library -module-name MoshpitKit \
    -emit-module -emit-module-path "$work/MoshpitKit.swiftmodule" \
    -emit-library -static -o "$work/libMoshpitKit.a" \
    "$kit/Log.swift" "$kit/AppLanguage.swift" \
    "$kit/Push/PushSealedBox.swift" "$kit/Push/PushPairing.swift" "$kit/Push/PushStanding.swift" \
    "$kit/Push/AppGroupFileLock.swift" \
    "$kit/Push/AgentNotificationCopy.swift" "$kit/Push/PushRemoteNotification.swift" \
    "$kit/Push/PushPrefs.swift" "$kit/Push/PushDiagnostics.swift" \
    "$kit/Island/AgentNotifications.swift" "$kit/Island/AgentControlBridge.swift"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -I "$work" -L "$work" -lMoshpitKit \
    -o "$out" \
    "$install/HostScripts.swift" "$install/InstallManifest.swift" "$install/HostChannel.swift" \
    "$install/HostInstaller.swift" "$install/AgentHookConfig.swift" "$frozen/HostCapabilities.swift" \
    "$repo/Tests/MoshpitTests/Support/Shell.swift" \
    "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
