#!/bin/bash
# Builds the Swift side of Home's cross-check for macOS: the app's
# HomeSnapshot (HomeNSHPage.swift) as it was before app::home replaced it
# (95f649ab), with what it decides with from the same revision, and
# harness/home/swift/{stubs,main}.swift for the store, the hub, the metrics
# and the monitor as data.
#
# The app's code is kept in harness/home/swift/frozen, read from nowhere
# else: no public clone has 95f649ab (the history was folded on export), so
# a `git show` of it built the cross-check in one clone only. Cut out of
# their files at 95f649ab and otherwise unedited:
#
#   TmuxTypes.swift           Services/Tmux/TmuxTypes.swift, whole
#   TmuxSnapshot.swift        `struct TmuxSnapshot`, Services/MultiplexerControlling.swift
#   AgentHook.swift           `struct AgentHook`, Services/Tmux/TmuxSessionController.swift
#   TransportConnState.swift  `enum TransportConnState`, UI/Components.swift
#   HerdrSkew.swift           `HerdrSnapshot.clientIsOlder(…)` and `struct
#                             HerdrVersionReport`, Services/Herdr/HerdrSnapshot.swift
#   HomeSnapshot.swift        UI/Home/HomeNSHPage.swift, from "Everything the
#                             NSH Home shows" to "The host's components for Home"
#   TerminalViewModel.part    TerminalViewModel's `enum Status`, `connState`
#                             and `adoptRustStatus(…)`, UI/Terminal/TerminalViewModel.swift
#   SessionHub.part           `ActiveSession.awaitingFirstSession`, Services/SessionHub.swift
#
# The two parts go into stubs.swift where it marks them.
#
#   crates/app/harness/home/build-swift.sh [<output binary>]
#       (default $CARGO_TARGET_DIR or target, /home-harness/home-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
out="${1:-${CARGO_TARGET_DIR:-$repo/target}/home-harness/home-swift}"
src="$(dirname "$out")/src"
frozen="$here/swift/frozen"
rm -rf "$src"
mkdir -p "$src"
cp "$frozen"/*.swift "$src/"

python3 - "$here/swift/stubs.swift" "$frozen" "$src" <<'PY'
import sys
stubs, frozen, src = sys.argv[1:4]
s = open(stubs).read()
for marker, part in [("    //__TERMINAL_VIEW_MODEL__\n", "TerminalViewModel"),
                     ("        //__AWAITING_FIRST_SESSION__\n", "SessionHub")]:
    if marker not in s:
        sys.exit(f"build-swift.sh: stubs.swift has no {marker.strip()}")
    s = s.replace(marker, open(f"{frozen}/{part}.part").read())
open(f"{src}/stubs.swift", "w").write(s)
PY

swiftc -Onone -swift-version 5 -o "$out" "$src"/*.swift "$here/swift/main.swift"
echo "$out"
