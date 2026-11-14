#!/bin/bash
# Builds the Swift side of the client equivalence runs for macOS: the app's
# HerdrControlClient.swift and WriteLane.swift with their clocks swapped for
# the harness's virtual one (the only substitutions; each is counted, so a
# change in the app's source fails the build instead of testing stale code),
# the app sources they need compiled unchanged, W2's stubs and extract, and
# harness/client/swift/. Same language mode and optimisation as the app's
# Release build. Niced.
#
#   harness/client/build-swift.sh [<output binary>]
#   (default target/w2b-harness/client-swift; prints the binary's path)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
repo="$(cd "$crate/../.." && pwd)"
out="${1:-$repo/target/w2b-harness/client-swift}"
work="$(dirname "$out")/swift-build"
mkdir -p "$work/kit" "$work/src"

python3 - "$repo" "$work/src" <<'PY'
import os, sys
repo, dest = sys.argv[1], sys.argv[2]
def substitute(path, rules):
    text = open(os.path.join(repo, path), encoding="utf-8").read()
    for old, new, count in rules:
        found = text.count(old)
        if found != count:
            sys.exit(f"build-swift.sh: {old!r} found {found} times in {path}, expected {count}")
        text = text.replace(old, new)
    name = os.path.basename(path)
    open(os.path.join(dest, name), "w", encoding="utf-8").write(
        f"// Generated from {path} by crates/herdr/harness/client/build-swift.sh: the\n"
        "// app's source with its clocks swapped for the harness's. Do not edit.\n" + text)
substitute("Moshpit/Services/Herdr/HerdrControlClient.swift", [
    ("Task.sleep(", "HarnessClock.sleep(", 6),
    ("ContinuousClock.now", "HarnessClock.continuousNow", 3),
    ("Date()", "HarnessClock.date()", 1),
])
substitute("Moshpit/Services/WriteLane.swift", [("Task.sleep(", "HarnessClock.sleep(", 1)])
PY

python3 "$crate/harness/extract.py" "$work/extract.swift"

nice -n 10 swiftc -O -wmo -swift-version 5 -parse-as-library \
    -emit-library -static -emit-module -module-name MoshpitKit \
    -emit-module-path "$work/kit/MoshpitKit.swiftmodule" \
    -o "$work/kit/libMoshpitKit.a" \
    "$repo/Packages/MoshpitKit/Sources/MoshpitKit/Log.swift"

herdr="$repo/Moshpit/Services/Herdr"
nice -n 10 swiftc -O -wmo -swift-version 5 -parse-as-library \
    -I "$work/kit" -L "$work/kit" -lMoshpitKit \
    -o "$out" \
    "$work/src/HerdrControlClient.swift" "$work/src/WriteLane.swift" \
    "$herdr/HerdrSnapshot.swift" "$herdr/HerdrLaunch.swift" \
    "$repo/Moshpit/Services/MultiplexerControlling.swift" \
    "$repo/Moshpit/Services/Tmux/TmuxTypes.swift" \
    "$repo/Moshpit/Models/AgentTaskRequest.swift" \
    "$repo/Moshpit/Services/Shell.swift" "$repo/Moshpit/Services/BracketedPaste.swift" \
    "$work/extract.swift" "$crate/harness/swift/stubs.swift" \
    "$here/swift/harness.swift" "$here/swift/main.swift"
echo "$out"
