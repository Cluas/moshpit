#!/bin/bash
# Builds the Swift side of Home's cross-check for macOS: the app's
# HomeSnapshot (HomeNSHPage.swift) as it was before app::home replaced it
# (95f649ab, or $MOSHPIT_SWIFT_HOME_REV), with what it decides with taken
# from the same revision — TmuxTypes.swift whole; TmuxSnapshot, AgentHook,
# TransportConnState, HerdrSnapshot.clientIsOlder and HerdrVersionReport,
# TerminalViewModel's Status, connState and adoptRustStatus, and
# ActiveSession.awaitingFirstSession cut out of their files unchanged — and
# harness/home/swift/{stubs,main}.swift for the store, the hub, the metrics
# and the monitor as data.
#
#   crates/app/harness/home/build-swift.sh [<output binary>]
#       (default $CARGO_TARGET_DIR or target, /home-harness/home-swift)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
out="${1:-${CARGO_TARGET_DIR:-$repo/target}/home-harness/home-swift}"
src="$(dirname "$out")/src"
rev="${MOSHPIT_SWIFT_HOME_REV:-95f649ab}"
mkdir -p "$src"

python3 - "$repo" "$rev" "$here/swift/stubs.swift" "$src" <<'PY'
import subprocess, sys
repo, rev, stubs, src = sys.argv[1:5]

def show(path):
    return subprocess.run(["git", "-C", repo, "show", f"{rev}:{path}"], check=True,
                          capture_output=True, text=True).stdout

def block(text, marker):
    """From the start of the line holding `marker` to the brace closing the
    first one opened after it (comments and string literals skipped)."""
    at = text.index(marker)
    start = text.rindex("\n", 0, at) + 1
    i, depth, opened = at, 0, False
    while True:
        if text.startswith("//", i):
            i = text.index("\n", i)
            continue
        c = text[i]
        if c == '"':
            i += 1
            while text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif c == "{":
            depth, opened = depth + 1, True
        elif c == "}":
            depth -= 1
            if opened and depth == 0:
                return text[start:i + 1] + "\n"
        i += 1

def write(name, text):
    open(f"{src}/{name}", "w").write("import Foundation\n\n" + text)

write("TmuxTypes.swift", show("Moshpit/Services/Tmux/TmuxTypes.swift").replace("import Foundation\n", "", 1))
write("TmuxSnapshot.swift", block(show("Moshpit/Services/MultiplexerControlling.swift"), "struct TmuxSnapshot"))
write("AgentHook.swift", block(show("Moshpit/Services/Tmux/TmuxSessionController.swift"), "struct AgentHook"))
write("TransportConnState.swift", block(show("Moshpit/UI/Components.swift"), "enum TransportConnState"))
herdr = show("Moshpit/Services/Herdr/HerdrSnapshot.swift")
write("HerdrSkew.swift", "enum HerdrSnapshot {\n" + block(herdr, "static func clientIsOlder(")
      + "}\n\n" + block(herdr, "struct HerdrVersionReport"))
page = show("Moshpit/UI/Home/HomeNSHPage.swift")
start = page.index("/// Everything the NSH Home shows")
end = page.index("// MARK: - The host's components for Home")
write("HomeSnapshot.swift", page[start:end])

vm = show("Moshpit/UI/Terminal/TerminalViewModel.swift")
hub = show("Moshpit/Services/SessionHub.swift")
s = open(stubs).read()
s = s.replace("    //__TERMINAL_VIEW_MODEL__\n", block(vm, "enum Status: Equatable")
              + block(vm, "var connState: TransportConnState") + block(vm, "func adoptRustStatus("))
s = s.replace("        //__AWAITING_FIRST_SESSION__\n", block(hub, "var awaitingFirstSession: Bool"))
open(f"{src}/stubs.swift", "w").write(s)
PY

swiftc -Onone -swift-version 5 -o "$out" "$src"/*.swift "$here/swift/main.swift"
echo "$out"
