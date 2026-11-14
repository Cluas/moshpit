#!/bin/bash
# Builds mosh-live: the app's mosh code, compiled for macOS, to run against a
# real mosh-server (run.sh starts one through a private sshd). These app files
# go in UNCHANGED:
#
#   Moshpit/Services/Mosh/MoshTransport.swift    the transport (NWConnection UDP)
#   Moshpit/Services/Mosh/MoshWire.swift, MoshCrypto.swift, OCB3.swift,
#   Moshpit/Services/Mosh/MoshCompression.swift  what it speaks
#   Moshpit/Services/Mosh/MoshBootstrap.swift    the `mosh-server new` line and its parser
#   Moshpit/Services/Mosh/MoshServerReap.swift   when the app stops a server (round 22 on)
#   Moshpit/Services/Herdr/HerdrLaunch.swift,
#   Moshpit/Services/Shell.swift                 herdr's raw-attach loop and its retarget
#
# plus HostCapabilities.extraPathDirs cut verbatim out of
# Moshpit/Services/HostCapabilities.swift, shim.swift (the app's SSHSession as
# the system's ssh), rules.swift and main.swift. Swift 5 language mode, -O.
#
#   scripts/verify/mosh-live/build.sh [<commit>]
#
# With a commit, the app files are read from that commit (`git show`, no
# checkout): the code before a fix, built the same way. rules.swift then runs
# what that code's SessionHub did where it has no MoshServerReap rule. Prints
# the binary's path: target/mosh-live/mosh-live[-<commit>].
#
# MOSHPIT_MOSH_LIVE_WORK=<dir> builds in <dir> instead of target/mosh-live
# (crates/seal/tests/harness_typecheck.rs: its typecheck leaves the built
# rig alone).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
commit="${1:-}"
work="${MOSHPIT_MOSH_LIVE_WORK:-$repo/target/mosh-live}"
src="$work/src${commit:+-$commit}"
out="$work/mosh-live${commit:+-$commit}"
rm -rf "$src"
mkdir -p "$src"

# Copies app file $1 into the build, from $commit when given; fails if absent
# unless $2 is "optional".
take() {
    local path="$1" dest="$src/$(basename "$1")"
    if [ -n "$commit" ]; then
        if ! git -C "$repo" show "$commit:$path" > "$dest" 2>/dev/null; then
            rm -f "$dest"
            [ "${2:-}" = optional ] && return 1
            echo "build.sh: no $path at $commit" >&2
            exit 1
        fi
    else
        if [ ! -f "$repo/$path" ]; then
            [ "${2:-}" = optional ] && return 1
            echo "build.sh: no $path" >&2
            exit 1
        fi
        cp "$repo/$path" "$dest"
    fi
}

for file in MoshTransport MoshWire MoshCrypto OCB3 MoshCompression MoshBootstrap; do
    take "Moshpit/Services/Mosh/$file.swift"
done
# herdr-retarget: the raw-attach loop and the retarget, as the app builds them,
# and whether the app takes a retarget as landed only on the host's word (B208).
take Moshpit/Services/Herdr/HerdrLaunch.swift
take Moshpit/Services/Shell.swift
flags=()
grep -q "func retargetLanded" "$src/HerdrLaunch.swift" && flags+=(-D MOSHPIT_RETARGET_LANDED)
# herdr-life (round 27): whether the loop parks and the app watches it.
grep -q "func rawAttachWatchCommand" "$src/HerdrLaunch.swift" && flags+=(-D MOSHPIT_RENDERER_PARKS)
grep -q "func retargetMissing" "$src/HerdrLaunch.swift" && flags+=(-D MOSHPIT_RETARGET_CHECKS)
# herdr-life follow (round 28): --takeover only on the person's word.
grep -q "takeover: Bool" "$src/HerdrLaunch.swift" && flags+=(-D MOSHPIT_RETARGET_TAKEOVER)
# herdr-life retype (round 28): the loop-up check before the first target.
grep -q "func rawAttachLoopUpCommand" "$src/HerdrLaunch.swift" && flags+=(-D MOSHPIT_LOOP_UP_CHECK)
if take Moshpit/Services/Mosh/MoshServerReap.swift optional; then
    grep -q "func onReturnPathDead" "$src/MoshServerReap.swift" && flags+=(-D MOSHPIT_REAP_RETURN_PATH)
    grep -q "func onTeardown" "$src/MoshServerReap.swift" && flags+=(-D MOSHPIT_REAP_TEARDOWN)
fi
grep -q "onReturnPathDead: ((_ heardFromServer: Bool) -> Void)?" "$src/MoshTransport.swift" \
    && flags+=(-D MOSHPIT_RETURN_PATH_HEARD)

# The one HostCapabilities name MoshBootstrap uses, cut verbatim.
take Moshpit/Services/HostCapabilities.swift
{
    echo "// Cut verbatim from Moshpit/Services/HostCapabilities.swift by scripts/verify/mosh-live/build.sh."
    echo "enum HostCapabilities {"
    awk '/static let extraPathDirs =/ { on = 1 } on { print } on && /"$/ && !/=$/ { exit }' \
        "$src/HostCapabilities.swift"
    echo "}"
} > "$src/HostCapabilitiesPath.swift"
rm "$src/HostCapabilities.swift"
grep -q "extraPathDirs" "$src/HostCapabilitiesPath.swift" || { echo "build.sh: no extraPathDirs" >&2; exit 1; }

cp "$here/shim.swift" "$here/rules.swift" "$here/main.swift" "$here/herdr.swift" "$here/herdr-life.swift" "$src/"
rm -f "$out"
(cd "$src" && nice -n 10 swiftc -swift-version 5 -O ${flags[@]+"${flags[@]}"} -o "$out" ./*.swift)
echo "$out"
