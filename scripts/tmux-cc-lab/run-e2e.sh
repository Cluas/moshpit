#!/bin/bash
# Full-stack SSH+tmux renderer e2e: real tmux (lab.py serve) ↔ TCP bridge ↔
# the production TmuxSessionController + SwiftTerm running in the simulator.
#
# Usage: scripts/tmux-cc-lab/run-e2e.sh [simulator-udid]
set -euo pipefail
cd "$(dirname "$0")/../.."

DATA_PORT=${DATA_PORT:-8765}
CTL_PORT=${CTL_PORT:-8766}
UDID=${1:-$(xcrun simctl list devices booted | grep -oE '[0-9A-F-]{36}' | head -1)}
[ -n "$UDID" ] || { echo "no booted simulator; pass a UDID"; exit 1; }

python3 scripts/tmux-cc-lab/lab.py serve --data-port "$DATA_PORT" --ctl-port "$CTL_PORT" &
SERVE_PID=$!
trap 'kill $SERVE_PID 2>/dev/null || true' EXIT
sleep 2

# TEST_RUNNER_-prefixed vars must be in xcodebuild's ENVIRONMENT (they are
# forwarded to the test runner); passing them as arguments makes them inert
# build settings and the suite silently skips.
TAP_DIR=${TAP_DIR:-$(mktemp -d /tmp/moshpit-cc-tap.XXXXXX)}
echo "cc tap: $TAP_DIR"
env TEST_RUNNER_MOSHPIT_TMUX_LAB_PORT="$DATA_PORT" \
    TEST_RUNNER_MOSHPIT_TMUX_LAB_CTL_PORT="$CTL_PORT" \
    TEST_RUNNER_MOSHPIT_CC_TAP="$TAP_DIR" \
xcodebuild test \
  -project Moshpit.xcodeproj -scheme Moshpit \
  -destination "id=$UDID" \
  -only-testing:MoshpitTests/TmuxLabE2ETests \
  2>&1 | grep -E "Test Suite|Test Case|passed|failed|error:|SKIPPED|diverged" || true
