#!/bin/bash
# Builds the Swift side of the install equivalence harness for macOS: the
# MoshpitKit sources the install code imports (Log, Push/*, the two Island
# files PushRemoteNotification names) as the module MoshpitKit, then the
# app's UNCHANGED Moshpit/Services/Install/{HostScripts,InstallManifest,
# HostChannel,HostInstaller,AgentHookConfig}.swift — gone from the app since
# W15b moved the installer to the core, so read from the last commit that
# had them ($MOSHPIT_SWIFT_INSTALL_REV, 81c121a1 by default) —
# Moshpit/Services/HostCapabilities.swift and Moshpit/Services/Shell.swift
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
kit="$repo/Packages/MoshpitKit/Sources/MoshpitKit"
# The app's Swift installer, as it was when the app last ran it.
rev="${MOSHPIT_SWIFT_INSTALL_REV:-81c121a1}"
install="$(dirname "$out")/swift-install"
mkdir -p "$install"
for name in HostScripts InstallManifest HostChannel HostInstaller AgentHookConfig; do
    git -C "$repo" show "$rev:Moshpit/Services/Install/$name.swift" >"$install/$name.swift"
done

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
    "$install/HostInstaller.swift" "$install/AgentHookConfig.swift" "$repo/Moshpit/Services/HostCapabilities.swift" \
    "$repo/Moshpit/Services/Shell.swift" \
    "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
