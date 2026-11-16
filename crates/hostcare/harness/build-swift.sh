#!/bin/bash
# Builds the Swift side of the host-care cross-check for macOS: the app's
# host care as it was before it moved to the core — HostAutoCare,
# HostSetupModel, PushService and the installer they drive — with
# harness/swift/{stubs,main}.swift.
#
# The app's sources are kept in the repo, read from nowhere else: they are
# gone from the app, and the commits that last had them are in no public
# clone (the history was folded on export), so a `git show` of them built
# the cross-check in one clone only. Unedited, as the app last ran them:
# harness/swift/frozen holds host care's own (81c121a1: HostAutoCare,
# HostSetupModel, PushService, HostCapabilities, Shell, WriteLane), and the
# installer and MoshpitKit's push sources are the install harness's
# (crates/install/harness/swift/frozen: the installer and PushPairing at
# 81c121a1, the rest of the kit at 8ef981c5).
#
# PushService is the one source changed, on a copy, and only where it calls
# iOS: its two imports, the body of registerForRemoteNotifications() (asks
# UNUserNotificationCenter, which a command-line process can't), and the
# app delegate. Everything it decides is as the app had it.
#
#   harness/build-swift.sh [<output binary>]   (default target/hostcare-harness/care-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
out="${1:-$repo/target/hostcare-harness/care-swift}"
work="$(dirname "$out")/kit"
src="$(dirname "$out")/src"
frozen="$here/swift/frozen"
install="$repo/crates/install/harness/swift/frozen"
kit="$install/kit"
rm -rf "$src"
mkdir -p "$work" "$src"
cp "$install"/Install/{HostScripts,InstallManifest,HostChannel,HostInstaller,AgentHookConfig}.swift "$src/"
cp "$frozen"/{HostAutoCare,HostSetupModel,PushService,HostCapabilities,Shell,WriteLane}.swift "$src/"
python3 - "$src/PushService.swift" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("import UIKit\n", "").replace("import UserNotifications\n", "")
start = s.index("    func registerForRemoteNotifications() {")
depth, i = 0, s.index("{", start)
while True:
    if s[i] == "{": depth += 1
    elif s[i] == "}":
        depth -= 1
        if depth == 0: break
    i += 1
s = s[:start] + "    func registerForRemoteNotifications() {\n        Log.push.info(\"harness: no iOS to ask for a device token\")\n    }" + s[i + 1:]
s = s[:s.index("/// Minimal app delegate")]
open(p, "w").write(s)
PY

swiftc -O -wmo -swift-version 5 -parse-as-library -module-name MoshpitKit \
    -emit-module -emit-module-path "$work/MoshpitKit.swiftmodule" \
    -emit-library -static -o "$work/libMoshpitKit.a" \
    "$kit/Log.swift" "$kit/AppLanguage.swift" \
    "$kit/Push/PushSealedBox.swift" "$kit/Push/PushPairing.swift" "$kit/Push/PushStanding.swift" \
    "$kit/Push/AppGroupFileLock.swift" \
    "$kit/Push/AgentNotificationCopy.swift" "$kit/Push/PushRemoteNotification.swift" \
    "$kit/Push/PushPrefs.swift" "$kit/Push/PushDiagnostics.swift" \
    "$kit/Island/AgentNotifications.swift" "$kit/Island/AgentControlBridge.swift"
swiftc -Onone -swift-version 5 \
    -I "$work" -L "$work" -lMoshpitKit \
    -o "$out" \
    "$src"/*.swift "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
