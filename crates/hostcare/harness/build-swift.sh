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
mkdir -p "$work" "$src"
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

# The pairing file's Swift writer as of the same commit: the app's went with
# batch C (the Rust store writes it), and PushService here still pairs
# through it.
git -C "$repo" show "$rev:Packages/MoshpitKit/Sources/MoshpitKit/Push/PushPairing.swift" >"$work/PushPairing.swift"
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
    "$kit/Push/PushSealedBox.swift" "$work/PushPairing.swift" "$kit/Push/PushStanding.swift" \
    "$kit/Push/AppGroupFileLock.swift" \
    "$kit/Push/AgentNotificationCopy.swift" "$kit/Push/PushRemoteNotification.swift" \
    "$kit/Push/PushPrefs.swift" "$kit/Push/PushDiagnostics.swift" \
    "$kit/Island/AgentNotifications.swift" "$kit/Island/AgentControlBridge.swift"
swiftc -Onone -swift-version 5 \
    -I "$work" -L "$work" -lMoshpitKit \
    -o "$out" \
    "$src"/*.swift "$here/swift/stubs.swift" "$here/swift/main.swift"
echo "$out"
