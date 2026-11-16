#!/bin/bash
# Builds the Swift side of the install equivalence harness for macOS: the
# MoshpitKit sources the install code imports (Log, Push/*, the two Island
# files PushRemoteNotification names, as of the last commit that had Push/*)
# as the module MoshpitKit, then the
# app's UNCHANGED Moshpit/Services/Install/{HostScripts,InstallManifest,
# HostChannel,HostInstaller,AgentHookConfig}.swift — gone from the app since
# W15b moved the installer to the core, so read from the last commit that
# had them ($MOSHPIT_SWIFT_INSTALL_REV, 81c121a1 by default) — the app's
# Swift capability probe as it last ran it (Moshpit/Services/
# HostCapabilities.swift before E6a batch 2a handed the probe, its parser,
# the PATH dirs and the install commands to the core: e57f7941, or
# $MOSHPIT_SWIFT_CAPS_REV) and Tests/MoshpitTests/Support/Shell.swift (E7a moved it there)
# (HostCommands.quote) against it, with harness/swift/
# {stubs,main}.swift. Same language mode and optimisation as the app's
# Release build (SWIFT_VERSION 5.9, -O, whole module).
#
#   harness/build-swift.sh [<output binary>]   (default target/install-harness/install-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/install-harness/install-swift}"
work="$(dirname "$out")/kit"
# The app's Swift installer, as it was when the app last ran it.
rev="${MOSHPIT_SWIFT_INSTALL_REV:-81c121a1}"
install="$(dirname "$out")/swift-install"
mkdir -p "$install"
for name in HostScripts InstallManifest HostChannel HostInstaller AgentHookConfig; do
    git -C "$repo" show "$rev:Moshpit/Services/Install/$name.swift" >"$install/$name.swift"
done
# The app's Swift capability probe, as it was when the app last ran it.
caps_rev="${MOSHPIT_SWIFT_CAPS_REV:-e57f7941}"
git -C "$repo" show "$caps_rev:Moshpit/Services/HostCapabilities.swift" >"$install/HostCapabilities.swift"

# The stub's Multiplexer must keep the app's cases.
want="$(sed -n '/^enum Multiplexer/,/^}/p' "$repo/Moshpit/Models/ServerConnection.swift" | grep -E '^ *case [a-z]+$' | tr -d ' ')"
have="$(sed -n '/^enum Multiplexer/,/^}/p' "$here/swift/stubs.swift" | grep -E '^ *case [a-z]+$' | tr -d ' ')"
if [ "$want" != "$have" ]; then
    echo "harness/swift/stubs.swift: Multiplexer's cases differ from Models/ServerConnection.swift" >&2
    exit 1
fi

mkdir -p "$work"
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
# The pairing's Swift helpers as of the same commit (HostInstaller mints its
# self-test's name with them): the app's went with batch C.
git -C "$repo" show "$rev:Packages/MoshpitKit/Sources/MoshpitKit/Push/PushPairing.swift" >"$work/PushPairing.swift"
swiftc -O -wmo -swift-version 5 -parse-as-library -module-name MoshpitKit \
    -emit-module -emit-module-path "$work/MoshpitKit.swiftmodule" \
    -emit-library -static -o "$work/libMoshpitKit.a" \
    "$kit/Log.swift" "$kit/AppLanguage.swift" \
    "$kit/Push/PushSealedBox.swift" "$work/PushPairing.swift" "$kit/Push/PushStanding.swift" \
    "$kit/Push/AppGroupFileLock.swift" \
    "$kit/Push/AgentNotificationCopy.swift" "$kit/Push/PushRemoteNotification.swift" \
    "$kit/Push/PushPrefs.swift" "$kit/Push/PushDiagnostics.swift" \
    "$kit/Island/AgentNotifications.swift" "$kit/Island/AgentControlBridge.swift"
swiftc -O -wmo -swift-version 5 -parse-as-library \
    -I "$work" -L "$work" -lMoshpitKit \
    -o "$out" \
    "$install/HostScripts.swift" "$install/InstallManifest.swift" "$install/HostChannel.swift" \
    "$install/HostInstaller.swift" "$install/AgentHookConfig.swift" "$install/HostCapabilities.swift" \
    "$repo/Tests/MoshpitTests/Support/Shell.swift" \
    "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
