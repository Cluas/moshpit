#!/bin/bash
# Builds the Swift side of the controller equivalence harness for macOS with
# SwiftPM: the app's TmuxSessionController.swift (substitutions listed in
# generate.py), the app sources it needs, and harness/controller/swift/.
# Release configuration, two jobs, niced.
#
#   harness/controller/build-swift.sh [<package dir>]
#   (default target/w8-harness/swift-controller; prints the binary's path)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
pkg="${1:-$repo/target/w8-harness/swift-controller}"
mkdir -p "$pkg"
python3 "$here/generate.py" "$pkg"
nice -n 10 swift build -c release -j 2 --package-path "$pkg" >&2
echo "$pkg/.build/release/controller-swift"
