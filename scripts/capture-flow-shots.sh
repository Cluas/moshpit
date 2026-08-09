#!/usr/bin/env bash
# Capture the real screens for the connection→usage flow prototype, on BOTH
# multiplexers, so the prototype shows the shipped app rather than a drawing of
# it. Anything not captured here has to be drawn — and labelled as drawn.
#
# Isolation, and why it matters: the tmux path is pointed at a PRIVATE socket
# through a wrapper (the same trick verify-e2e-mosh-tmux.sh uses). Attaching to
# your real session would pin its windows to the phone's grid, squeezing the
# desktop client you're actually working in. This never touches it.
#
# herdr has no equivalent hazard for sessions, but its direct attach IS
# exclusive per pane, so the app is terminated at the end — a simulator left
# polling steals the attach from any other client on the same host.

set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

SIM_NAME="${MOSAIC_SIM:-iPhone 17 Pro}"
BUNDLE_ID="com.cluas.offhook"
KEY_PATH="${MOSAIC_SSH_KEY:-$HOME/.ssh/id_ed25519}"
OUT="build/serve/docs/shots"
TMUX_BIN="$(command -v tmux)"
SOCK="offhook-proto"
WRAP="build/proto-tmux"
mkdir -p "$OUT" build

SIM_UDID="$(xcrun simctl list devices --json | python3 -c "
import json, sys
data = json.load(sys.stdin); target = sys.argv[1]
booted = available = None
for _r, ds in data['devices'].items():
    for d in ds:
        if d.get('name') == target and d.get('isAvailable', False):
            if d.get('state') == 'Booted' and booted is None: booted = d['udid']
            elif available is None: available = d['udid']
print(booted or available or '')" "$SIM_NAME")"
[ -n "$SIM_UDID" ] || { echo "✘ no simulator named '$SIM_NAME'" >&2; exit 1; }

cleanup() {
  xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  pkill -f "herdr terminal session control" >/dev/null 2>&1 || true
  "$TMUX_BIN" -L "$SOCK" kill-server >/dev/null 2>&1 || true
}
trap cleanup EXIT

cat > "$WRAP" <<EOF
#!/bin/sh
exec "$TMUX_BIN" -L "$SOCK" "\$@"
EOF
chmod +x "$WRAP"
WRAP_ABS="$(cd "$(dirname "$WRAP")" && pwd)/$(basename "$WRAP")"

echo "▶ Building"
DERIVED="$(mktemp -d)"
xcodebuild -project Offhook.xcodeproj -scheme Offhook -configuration Debug \
  -sdk iphonesimulator -destination "platform=iOS Simulator,name=$SIM_NAME" \
  -derivedDataPath "$DERIVED" CODE_SIGNING_ALLOWED=NO build >/dev/null 2>&1
APP="$(find "$DERIVED/Build/Products" -name 'Offhook.app' -type d | head -1)"
xcrun simctl install "$SIM_UDID" "$APP"
xcrun simctl status_bar "$SIM_UDID" override --time "9:41" --batteryState charged \
  --batteryLevel 100 --wifiBars 3 --cellularBars 4 --dataNetwork wifi 2>/dev/null || true

KEY_B64="$(base64 -i "$KEY_PATH" | tr -d '\n')"
shot() { sleep "${2:-2}"; xcrun simctl io "$SIM_UDID" screenshot --type=png "$OUT/$1.png" >/dev/null 2>&1; echo "  · $1"; }
tapl() { idb ui tap --udid "$SIM_UDID" --duration 0.25 "$1" "$2" >/dev/null 2>&1 || true; }
find_label() {   # find_label <label> → "x y" centre, empty if absent
  idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
want = sys.argv[1]
for el in (data if isinstance(data, list) else [data]):
    if isinstance(el, dict) and (el.get('AXLabel') or '') == want:
        f = el.get('frame') or {}
        print(int(f['x']+f['width']/2), int(f['y']+f['height']/2)); break
" "$1"
}
launch() {   # launch <mux> [extra args…]
  xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  local mux="$1"; shift
  xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID" \
    -OFFHOOK_SEED_USER "$(whoami)" -OFFHOOK_SEED_KEY_B64 "$KEY_B64" \
    -OFFHOOK_SEED_HOST 127.0.0.1 -OFFHOOK_SEED_PORT 22 \
    -OFFHOOK_SEED_MUX "$mux" -OFFHOOK_SEED_QUIET 1 "$@" >/dev/null
}

# ---------------------------------------------------------------- shared
echo "▶ Shared: home + connection form"
launch herdr -OFFHOOK_SEED_HOME 1
shot 01-home 6
P="$(find_label Add)"; [ -n "$P" ] && tapl $P
shot 02-add-connection 3
# Scroll to ADVANCED, where the multiplexer picker lives.
for _ in 1 2 3; do idb ui swipe --udid "$SIM_UDID" --duration 0.35 200 750 200 250 >/dev/null 2>&1; sleep 1; done
shot 03-advanced 2
P="$(find_label 'Multiplexer, tmux')"; [ -n "$P" ] || P="$(find_label 'Multiplexer, herdr')"
[ -n "$P" ] && tapl $P
shot 04-multiplexer-picker 2

# ---------------------------------------------------------------- tmux
echo "▶ tmux (private socket — your own sessions are untouched)"
"$TMUX_BIN" -L "$SOCK" kill-server >/dev/null 2>&1 || true
"$TMUX_BIN" -L "$SOCK" new-session -d -s demo -x 52 -y 30
"$TMUX_BIN" -L "$SOCK" new-window -t demo -n logs
"$TMUX_BIN" -L "$SOCK" send-keys -t demo "printf 'offhook · tmux path\\n'" C-m
launch tmux -OFFHOOK_SEED_TMUX_BIN "$WRAP_ABS"
shot 10-tmux-terminal 16
P="$(find_label '0')"; [ -n "$P" ] || P="200 89"
tapl 202 89
shot 11-tmux-windows-sheet 3

# ---------------------------------------------------------------- herdr
echo "▶ herdr"
timeout 5 herdr api snapshot >/dev/null 2>&1 || {
  (nohup herdr server >/dev/null 2>&1 </dev/null &); sleep 3
  timeout 5 herdr workspace create --focus >/dev/null 2>&1 || true
}
launch herdr
shot 20-herdr-terminal 16
tapl 202 89
shot 21-herdr-tabs-sheet 3
# The tabs sheet is still up — a tap "on the crumb" would only hit its scrim
# and dismiss it, which is how this shot spent a day showing no sheet at all.
# Dismiss first, then open the workspaces sheet.
tapl 200 400
sleep 2
tapl 143 89
shot 22-herdr-workspaces-sheet 3
tapl 200 400   # leave no sheet behind for the next section
sleep 1

# ---------------------------------------------------------------- agent workbench
# W1 + W2. Needs a repo with a pane sitting in it (that's how the form finds
# repos) and two agents in different states so the ordering is visible.
echo "▶ Agent workbench (W1 + W2)"
DEMO_REPO="$HOME/offhook-demo"
rm -rf "$DEMO_REPO"; mkdir -p "$DEMO_REPO"
git -C "$DEMO_REPO" init -q
git -C "$DEMO_REPO" commit -q --allow-empty -m init
timeout 5 herdr workspace create --cwd "$DEMO_REPO" --label demo --focus >/dev/null 2>&1 || true
sleep 1
SHOT_PANES="$(timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
snap = json.load(sys.stdin).get('result', {}).get('snapshot', {})
print(' '.join(p['pane_id'] for p in snap.get('panes', [])[:3]))")"
set -- $SHOT_PANES
timeout 5 herdr pane report-agent "${1:-w1:p1}" --source offhook-shots \
  --agent "Claude Code" --state blocked >/dev/null 2>&1 || true
timeout 5 herdr pane report-agent "${2:-w1:p1}" --source offhook-shots \
  --agent "Codex" --state working >/dev/null 2>&1 || true
# A third, idle-by-name agent, so the section's quiet bottom row is on film
# too. Optional garnish — no verification, 0.7.3 may refuse the state.
[ -n "${3:-}" ] && timeout 5 herdr pane report-agent "$3" --source offhook-shots \
  --agent "agy" --state idle >/dev/null 2>&1 || true

# The whole point of shot 40 is a POPULATED Agents section. report-agent can
# fail silently (server restarted, panes vanished between snapshot and now),
# and the one time it did, the published page spent an afternoon captioning an
# empty list with a paragraph about ordering. Verify, or say why we stopped.
staged_ok() {
  timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
snap = json.load(sys.stdin).get('result', {}).get('snapshot', {})
states = {p.get('agent_status') for p in snap.get('panes', [])}
sys.exit(0 if 'blocked' in states and 'working' in states else 1)"
}
if ! staged_ok; then
  sleep 2
  staged_ok || { echo "✘ agent staging didn't take (report-agent failed?) — refusing to shoot an empty Agents section" >&2; exit 1; }
fi

launch herdr -OFFHOOK_SEED_HOME 1
sleep 6
tapl 201 299          # connect the card
sleep 20
# The "needs you" notification fires as soon as the app sees the blocked
# agent and covers the nav bar — swipe it away before shooting.
dismiss_banner() {
  idb ui swipe --udid "$SIM_UDID" --duration 0.25 200 90 200 20 >/dev/null 2>&1 || true
  sleep 2
}
dismiss_banner
shot 40-agents-home 2
P="$(find_label Add)"   # the AGENTS header's +
AGENTS_PLUS="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
for el in (data if isinstance(data, list) else [data]):
    if isinstance(el, dict) and el.get('type') == 'Button' and (el.get('AXLabel') or '') == 'Add':
        f = el.get('frame') or {}
        if 340 < f.get('y', 0) < 420:   # the section header, not the nav bar
            print(int(f['x']+f['width']/2), int(f['y']+f['height']/2)); break
")"
if [ -n "$AGENTS_PLUS" ]; then
  # shellcheck disable=SC2086
  tapl $AGENTS_PLUS
  sleep 4
  dismiss_banner
  shot 41-new-agent-task 2
  P="$(find_label Cancel)"; [ -n "$P" ] && tapl $P
  sleep 2
fi

# The agent breadcrumb (L2): tap the blocked agent's row — same path a tree
# pane row takes — and the terminal's third crumb becomes "● Claude Code"
# with the amber needs-you tint, session crumb squeezed to its icon.
AGENT_ROW="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
for el in (data if isinstance(data, list) else [data]):
    if isinstance(el, dict) and (el.get('AXLabel') or '').startswith('Claude Code,'):
        f = el.get('frame') or {}
        print(int(f['x']+f['width']/2), int(f['y']+f['height']/2)); break
")"
if [ -n "$AGENT_ROW" ]; then
  # Any leftover banner goes BEFORE entering the terminal — the dismiss swipe
  # starts at y=90, which on the terminal screen is the breadcrumb, and
  # "dismissing" there taps the window crumb and photobombs the shot with the
  # Tabs sheet. Nothing re-notifies between here and the screenshot.
  dismiss_banner
  # shellcheck disable=SC2086
  tapl $AGENT_ROW
  sleep 8            # push + frame channel attach
  shot 43-agent-breadcrumb 2
fi

# Clean up everything this section created.
timeout 5 herdr pane release-agent "${1:-w1:p1}" --source offhook-shots --agent "Claude Code" >/dev/null 2>&1 || true
timeout 5 herdr pane release-agent "${2:-w1:p1}" --source offhook-shots --agent "Codex" >/dev/null 2>&1 || true
[ -n "${3:-}" ] && timeout 5 herdr pane release-agent "$3" --source offhook-shots --agent "agy" >/dev/null 2>&1 || true
DEMO_WS="$(timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
snap = json.load(sys.stdin).get('result', {}).get('snapshot', {})
print(next((w['workspace_id'] for w in snap.get('workspaces', []) if w.get('label') == 'demo'), ''))")"
[ -n "$DEMO_WS" ] && timeout 5 herdr workspace close "$DEMO_WS" >/dev/null 2>&1
rm -rf "$DEMO_REPO"

echo
echo "✓ shots in $OUT"
ls -1 "$OUT" | sed 's/^/  /'
