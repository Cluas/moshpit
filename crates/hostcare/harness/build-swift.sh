#!/bin/bash
# Builds the Swift side of the host-care cross-check for macOS: the app's
# host care as it was before W15b moved it to the core (81c121a1, or
# $MOSHPIT_SWIFT_CARE_REV) — HostAutoCare, HostSetupModel, PushService and
# the installer they drive — with harness/swift/{stubs,main}.swift.
#
# PushService is the one source changed, and only where it calls iOS: its
# two imports, the body of registerForRemoteNotifications() (asks
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
rev="${MOSHPIT_SWIFT_CARE_REV:-81c121a1}"
kit="$repo/Packages/MoshpitKit/Sources/MoshpitKit"
mkdir -p "$work" "$src"

for path in Moshpit/Services/Install/HostScripts.swift Moshpit/Services/Install/InstallManifest.swift \
    Moshpit/Services/Install/HostChannel.swift Moshpit/Services/Install/HostInstaller.swift \
    Moshpit/Services/Install/AgentHookConfig.swift Moshpit/Services/Install/HostAutoCare.swift \
    Moshpit/UI/Install/HostSetupModel.swift Moshpit/Services/PushService.swift \
    Moshpit/Services/HostCapabilities.swift Moshpit/Services/Shell.swift Moshpit/Services/WriteLane.swift; do
    git -C "$repo" show "$rev:$path" >"$src/$(basename "$path")"
done
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
