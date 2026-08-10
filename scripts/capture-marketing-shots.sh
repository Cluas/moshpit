#!/usr/bin/env bash
# Marketing + App Store screenshots, shot in a STAGED environment.
#
# Why this exists next to capture-flow-shots.sh: that script documents the app
# for us — empty panes and `smoke-localhost` are fine when the audience already
# knows what they're looking at. This one is for people deciding whether to buy,
# where "~ · 1 tabs" and an empty terminal say nothing about the product.
#
# Isolation (nothing here touches your real setup):
#   herdr  — a private session (`--session moshpit-stage`) through a wrapper the
#            app is pointed at via -MOSHPIT_SEED_HERDR_BIN. Your own herdr server
#            keeps running, untouched and unseen.
#   tmux   — a private socket (`-L moshpit-stage`), same wrapper trick. Attaching
#            to your real session would pin its windows to the phone's grid.
#   repos  — staged under ~/.moshpit-stage, deleted at the end.
#   device — iPhone 17 Pro Max: 1320×2868, the size App Store Connect demands.
#            (capture-flow-shots uses the 6.3" Pro, which CANNOT be uploaded.)
#
# Everything the app renders is live: the tree, the agent states, the sheets,
# the breadcrumb. Only pane CONTENT is staged — see scripts/stage/claude-session.sh
# for why, and keep that file honest.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

SIM_NAME="${MOSHPIT_SIM:-iPhone 17 Pro Max}"
BUNDLE_ID="com.cluas.moshpit"
KEY_PATH="${MOSHPIT_SSH_KEY:-$HOME/.ssh/id_ed25519}"
OUT="${MOSHPIT_SHOT_DIR:-marketing/shots}"
STAGE="$HOME/.moshpit-stage"
HERDR_BIN="$(command -v herdr)"
TMUX_BIN="$(command -v tmux)"
SOCK="moshpit-stage"
mkdir -p "$OUT" build

command -v idb >/dev/null || { echo "✘ idb not installed" >&2; exit 1; }
[ -n "$HERDR_BIN" ] || { echo "✘ herdr not installed" >&2; exit 1; }

# ── wrappers: every herdr/tmux call the APP makes lands in the staged world ──
cat > build/stage-herdr <<EOF
#!/bin/sh
exec "$HERDR_BIN" --session $SOCK "\$@"
EOF
cat > build/stage-tmux <<EOF
#!/bin/sh
exec "$TMUX_BIN" -L $SOCK "\$@"
EOF
chmod +x build/stage-herdr build/stage-tmux
HERDR="$(pwd)/build/stage-herdr"
TMUXW="$(pwd)/build/stage-tmux"

cleanup() {
  xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  "$HERDR" server stop >/dev/null 2>&1 || true
  "$TMUX_BIN" -L "$SOCK" kill-server >/dev/null 2>&1 || true
  rm -rf "$STAGE"
}
trap cleanup EXIT

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

# ── staged repositories: names a developer would actually have ──
echo "▶ Staging repositories under $STAGE"
rm -rf "$STAGE"; mkdir -p "$STAGE"
stage_repo() {  # stage_repo <name> <branch> <file> <content>
  local dir="$STAGE/$1"
  mkdir -p "$dir"
  git -C "$dir" init -q
  printf '%s\n' "$4" > "$dir/$3"
  git -C "$dir" add -A
  git -C "$dir" -c user.email=dev@local -c user.name=Dev commit -qm "Initial commit"
  [ "$2" = "main" ] || git -C "$dir" checkout -q -b "$2"
}
stage_repo payments-api fix-webhook-retry retry.js 'export function retry(fn, times = 3) {
  return fn();
}'
stage_repo dashboard main app.tsx 'export default function App() {
  return <Shell />;
}'
stage_repo infra main main.tf 'resource "aws_s3_bucket" "artifacts" {}'
cp "$(pwd)/scripts/stage/build-session.sh" "$STAGE/run"; chmod +x "$STAGE/run"

# ── staged herdr, in a clean environment ──
echo "▶ Starting the staged herdr server (private session: $SOCK)"
"$HERDR" server stop >/dev/null 2>&1 || true
# A herdr server RESTORES its previous workspaces from session.json. Without
# this the staged tree accumulates every earlier run — the first capture came
# back with ten workspaces, three of them called payments-api.
rm -rf "$HOME/.config/herdr/sessions/$SOCK"
env -i HOME="$HOME" PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  TERM=xterm-256color SHELL=/bin/zsh LANG=en_US.UTF-8 \
  nohup "$HERDR" server >/tmp/moshpit-stage-herdr.log 2>&1 &
for _ in $(seq 1 20); do "$HERDR" api snapshot >/dev/null 2>&1 && break; sleep .5; done
"$HERDR" api snapshot >/dev/null 2>&1 || { echo "✘ staged herdr server never came up" >&2; exit 1; }

# A workspace per repo, labelled like the work — this is what turns a tree of
# "~ · 1 tabs" into something a stranger can read.
"$HERDR" workspace create --cwd "$STAGE/payments-api" --label payments-api --focus >/dev/null
"$HERDR" workspace create --cwd "$STAGE/dashboard" --label dashboard >/dev/null
"$HERDR" workspace create --cwd "$STAGE/infra" --label infra >/dev/null
sleep 1

read -r AGENT_PANE WORK_PANE IDLE_PANE <<<"$("$HERDR" api snapshot | python3 -c "
import json, sys
panes = json.load(sys.stdin)['result']['snapshot']['panes']
print(' '.join(p['pane_id'] for p in panes[:3]))")"
[ -n "${IDLE_PANE:-}" ] || { echo "✘ staged workspaces produced fewer than 3 panes" >&2; exit 1; }

# Tabs named after the work: an agent row's location reads
# "payments-api · claude", not "payments-api · 1".
"$HERDR" api snapshot | python3 -c "
import json, sys
tabs = json.load(sys.stdin)['result']['snapshot']['tabs']
print(' '.join(t['tab_id'] for t in tabs[:3]))" | {
  read -r T1 T2 T3
  [ -n "${T1:-}" ] && "$HERDR" tab rename "$T1" claude >/dev/null 2>&1
  [ -n "${T2:-}" ] && "$HERDR" tab rename "$T2" build >/dev/null 2>&1
  [ -n "${T3:-}" ] && "$HERDR" tab rename "$T3" shell >/dev/null 2>&1
}

# Pane content: the agent's turn, held on the permission prompt.
"$HERDR" pane run "$AGENT_PANE" "sh $(pwd)/scripts/stage/claude-session.sh" >/dev/null
"$HERDR" pane run "$WORK_PANE" "sh $STAGE/run" >/dev/null
"$HERDR" pane run "$IDLE_PANE" "sh $STAGE/run" >/dev/null
sleep 2

# Agent states. `report-agent` is herdr's own integration interface — the same
# call `herdr integration install claude` wires up, so what the app reads here
# is exactly what it reads in the field.
"$HERDR" pane report-agent "$AGENT_PANE" --source moshpit-stage --agent "Claude Code" --state blocked >/dev/null
"$HERDR" pane report-agent "$WORK_PANE"  --source moshpit-stage --agent "Codex"       --state working >/dev/null
"$HERDR" pane report-agent "$IDLE_PANE"  --source moshpit-stage --agent "claude"      --state idle    >/dev/null

staged_ok() {
  "$HERDR" api snapshot 2>/dev/null | python3 -c "
import json, sys
panes = json.load(sys.stdin)['result']['snapshot']['panes']
states = {p.get('agent_status') for p in panes}
sys.exit(0 if {'blocked', 'working'} <= states else 1)"
}
staged_ok || { sleep 2; staged_ok || { echo "✘ agent staging didn't take — refusing to shoot an empty Agents section" >&2; exit 1; }; }

# ── staged tmux: real window names, so the tmux shots say something ──
"$TMUX_BIN" -L "$SOCK" kill-server >/dev/null 2>&1 || true
"$TMUX_BIN" -L "$SOCK" new-session -d -s payments-api -n server -x 60 -y 34 -c "$STAGE/payments-api"
"$TMUX_BIN" -L "$SOCK" new-window -t payments-api -n tests -c "$STAGE/payments-api"
"$TMUX_BIN" -L "$SOCK" new-window -t payments-api -n logs -c "$STAGE/payments-api"
"$TMUX_BIN" -L "$SOCK" new-session -d -s dashboard -n dev -x 60 -y 34 -c "$STAGE/dashboard"
for _w in payments-api:server payments-api:tests dashboard:dev; do
  "$TMUX_BIN" -L "$SOCK" send-keys -t "$_w" "sh $STAGE/run" C-m
done
sleep 1

# ── build & install ──
echo "▶ Building"
DERIVED="$(mktemp -d)"
xcodebuild -project Moshpit.xcodeproj -scheme Moshpit -configuration Debug \
  -sdk iphonesimulator -destination "platform=iOS Simulator,name=$SIM_NAME" \
  -derivedDataPath "$DERIVED" CODE_SIGNING_ALLOWED=NO build >/dev/null 2>&1
APP="$(find "$DERIVED/Build/Products" -name 'Moshpit.app' -type d | head -1)"
xcrun simctl boot "$SIM_UDID" >/dev/null 2>&1 || true
xcrun simctl bootstatus "$SIM_UDID" -b >/dev/null 2>&1 || true
xcrun simctl install "$SIM_UDID" "$APP"
xcrun simctl status_bar "$SIM_UDID" override --time "9:41" --batteryState charged \
  --batteryLevel 100 --wifiBars 3 --cellularBars 4 --dataNetwork wifi 2>/dev/null || true

KEY_B64="$(base64 -i "$KEY_PATH" | tr -d '\n')"
shot() { sleep "${2:-2}"; xcrun simctl io "$SIM_UDID" screenshot --type=png "$OUT/$1.png" >/dev/null 2>&1; echo "  · $1"; }
tapl() { idb ui tap --udid "$SIM_UDID" --duration 0.25 "$1" "$2" >/dev/null 2>&1 || true; }
# Find a tappable element by SUBSTRING of its accessibility label, optionally
# narrowed to a y-range and an element type.
#
# Substring, not prefix: a row's label is everything inside it joined up —
# the connection card reads "M, mac-studio, SSH, LIVE, cluas@127.0.0.1 · 1h up",
# so "starts with mac-studio" finds nothing and the whole run screenshots a
# disconnected app. (It did.)
#
# Coordinates come back in POINTS, which is also what `idb ui tap` wants. This
# device is 1320×2868 pixels = 440×956 points; a tap at x=460 lands off-screen
# and silently does nothing — hence no hardcoded coordinates anywhere below.
find_label() {   # find_label <substring> [ymin] [ymax] [type]
  idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
want = sys.argv[1]
ymin = float(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2] else 0
ymax = float(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3] else 1e9
kind = sys.argv[4] if len(sys.argv) > 4 and sys.argv[4] else None
for el in (data if isinstance(data, list) else [data]):
    if not isinstance(el, dict): continue
    if kind and el.get('type') != kind: continue
    if want not in (el.get('AXLabel') or ''): continue
    f = el.get('frame') or {}
    y = f.get('y', 0)
    if not (ymin <= y <= ymax): continue
    print(int(f['x']+f['width']/2), int(f['y']+f['height']/2)); break
" "$1" "${2:-}" "${3:-}" "${4:-}"
}

tap_label() {   # tap_label <substring> [ymin] [ymax] [type] — returns 1 if absent
  local point
  point="$(find_label "$1" "${2:-}" "${3:-}" "${4:-}")"
  [ -n "$point" ] || return 1
  # shellcheck disable=SC2086
  tapl $point
  return 0
}
launch() {   # launch <mux> [extra args…]
  xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  local mux="$1"; shift
  xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID" \
    -MOSHPIT_SEED_USER "$(whoami)" -MOSHPIT_SEED_KEY_B64 "$KEY_B64" \
    -MOSHPIT_SEED_HOST 127.0.0.1 -MOSHPIT_SEED_PORT 22 \
    -MOSHPIT_SEED_NAME "mac-studio" \
    -MOSHPIT_SEED_MUX "$mux" -MOSHPIT_SEED_QUIET 1 "$@" >/dev/null
}

# First connection from a fresh simulator raises the host-key sheet, and it
# blocks everything behind it — including every screenshot below. (Found the
# hard way: this device had never trusted 127.0.0.1, while the 6.3" one the
# older script uses had, years of runs ago.) Trust persists per host, so one
# accepted sheet covers the rest of the run.
trust_host() {
  if tap_label Trust "" "" Button; then sleep 6; fi
}

# iOS asks before an app may read the pasteboard, and the alert is a system
# one — it sits above the app, swallows the tap that was meant for the app,
# and stays there. Two shots came back wrong because of it: the Mosh pane was
# a bare prompt (the paste never landed) and the clipboard shot was a picture
# of the alert itself rather than the app's paste control.
allow_paste() {
  sleep 1
  tap_label "Allow Paste" "" "" Button && sleep 2
}

# Put text on the pasteboard and get it into the pane, alert and all.
paste_into_pane() {   # paste_into_pane <text>
  printf '%s\n' "$1" | xcrun simctl pbcopy "$SIM_UDID" || true
  sleep 1
  tap_label "Paste" "" "" Button || return 1
  allow_paste
  return 0
}

# Tap the saved card, get past the sheet, wait for the tree to arrive.
connect_card() {
  tap_label "mac-studio" "" "" Button || echo "  ! no connection card to tap" >&2
  sleep 6
  trust_host
  sleep "${1:-14}"
}

echo "▶ Agent workbench (herdr)"
launch herdr -MOSHPIT_SEED_HERDR_BIN "$HERDR" -MOSHPIT_SEED_HOME 1
sleep 8
connect_card 20
shot 01-agents 4

# The pane the blocked agent lives in — tapping its row is the app's own path
# into that terminal, so the shot shows a real navigation, not a deep link.
if tap_label "Claude Code," "" "" Button; then
  sleep 12
  # The terminal takes focus on entry, so the keyboard covers half the frame.
  # Drop it for the clean shot; raise it again for the keyboard-bar shot.
  tap_label "keyboard.chevron.compact.down" || tap_label "Hide" || true
  shot 02-agent-terminal 3
  # Sheets, opened from the breadcrumb the same way a person would. The crumb
  # buttons carry the tab/workspace names, so they are findable by label.
  if tap_label "1:" 80 130 Button; then shot 03-tabs-sheet 3; tap_label "payments-api" 400 900 Button >/dev/null; sleep 2; fi
  if tap_label "payments-api" 80 130 Button; then shot 04-workspaces-sheet 3; tap_label "payments-api" 400 900 Button >/dev/null; sleep 2; fi
  # Keyboard bar — the shortcut row above the keys, which is where the
  # customisable keys live.
  tapl 220 420; shot 05-keyboard 4
fi

echo "▶ New agent task"
xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
launch herdr -MOSHPIT_SEED_HERDR_BIN "$HERDR" -MOSHPIT_SEED_HOME 1
sleep 8
connect_card 18
PLUS="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
for el in (data if isinstance(data, list) else [data]):
    if isinstance(el, dict) and el.get('type') == 'Button' and (el.get('AXLabel') or '') == 'Add':
        f = el.get('frame') or {}
        if 380 < f.get('y', 0) < 520:
            print(int(f['x']+f['width']/2), int(f['y']+f['height']/2)); break
")"
if [ -n "$PLUS" ]; then
  tapl $PLUS
  sleep 5
  idb ui text --udid "$SIM_UDID" "fix-webhook-retry" >/dev/null 2>&1 || true
  shot 06-new-task 3
  P="$(find_label Cancel)"; [ -n "$P" ] && tapl $P
fi

echo "▶ tmux"
launch tmux -MOSHPIT_SEED_TMUX_BIN "$TMUXW"
sleep 10; trust_host
shot 07-tmux-terminal 14
if tap_label ":" 80 130 Button; then shot 08-tmux-windows 3; fi

echo "▶ Mosh"
launch none -MOSHPIT_SEED_MOSH 1
sleep 10; trust_host
sleep 8
paste_into_pane 'sh ~/.moshpit-stage/run'
tap_label "return" "" "" Button || idb ui key --udid "$SIM_UDID" 40 >/dev/null 2>&1 || true
shot 09-mosh 10

echo "▶ Settings: themes, shortcuts, appearance"
xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
launch herdr -MOSHPIT_SEED_HERDR_BIN "$HERDR" -MOSHPIT_SEED_HOME 1
sleep 7
tap_label "gearshape" "" "" Button && sleep 3
open_settings() {
  xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  launch herdr -MOSHPIT_SEED_HERDR_BIN "$HERDR" -MOSHPIT_SEED_HOME 1
  sleep 6
  tap_label "gearshape" "" "" Button || tap_label "Settings" "" 200 Button || return 1
  sleep 3
}

settings_shot() {   # settings_shot <row substring> <shot name>
  open_settings || { echo "  ! could not open Settings for $2" >&2; return; }
  # idb reports the WHOLE scroll view's accessibility tree, including rows far
  # below the fold — "Shortcuts," came back at y=1509 on a 956-point screen.
  # Finding a label therefore proves nothing about being able to tap it, and
  # tapping an off-screen point silently does nothing: three of these shots
  # came back as identical pictures of the Settings root. So scroll until the
  # row is inside the viewport, and only then tap it.
  local tries=0
  until [ -n "$(find_label "$1" 130 880)" ]; do
    tries=$((tries + 1))
    [ "$tries" -gt 8 ] && { echo "  ! settings row never reached the viewport: $1" >&2; return; }
    idb ui swipe --udid "$SIM_UDID" --duration 0.3 220 700 220 300 >/dev/null 2>&1 || true
    sleep 1
  done
  tap_label "$1" 130 880 || return
  sleep 3
  # Prove we actually left the Settings root — otherwise this shot is a
  # duplicate and the site would publish it as if it showed the feature.
  if [ -n "$(find_label 'Accent,' 130 880)" ]; then
    echo "  ! tap on '$1' did not navigate — refusing to shoot $2" >&2
    return
  fi
  shot "$2" 2
}
settings_shot "Theme,"     10-themes
settings_shot "Shortcuts," 11-shortcuts
settings_shot "App Icon,"  12-icons

echo "▶ Docs: the screens five pages describe but could not show"

# SSH keys and the host fingerprint — /docs/keys.
settings_shot "SSH Keys," 30-ssh-keys

# The error a failed connection actually shows — /docs/troubleshooting.
# Port 9 discards everything, so the connection fails the way a wrong port
# does in the field rather than by an invented error path.
launch herdr -MOSHPIT_SEED_HERDR_BIN "$HERDR" -MOSHPIT_SEED_PORT 9 -MOSHPIT_SEED_HOME 1
sleep 6
tap_label "mac-studio" "" "" Button && sleep 12
shot 31-connection-error 2

# Scrollback, paste and CJK all need a live pane — /docs/scrolling,
# /docs/clipboard, /docs/ime.
launch herdr -MOSHPIT_SEED_HERDR_BIN "$HERDR"
sleep 8; trust_host; sleep 10

# Scrollback: on the tmux connection, whose pane carries a full run of output.
# Shooting this on the agent pane produced an identical frame — that pane holds
# one frozen screen and has nothing above it to scroll into.
launch tmux -MOSHPIT_SEED_TMUX_BIN "$TMUXW"
sleep 10; trust_host; sleep 6
idb ui swipe --udid "$SIM_UDID" --duration 0.4 220 380 220 760 >/dev/null 2>&1 || true
sleep 2
shot 32-scrollback 2
idb ui swipe --udid "$SIM_UDID" --duration 0.4 220 760 220 380 >/dev/null 2>&1 || true
sleep 2

# Paste: put something multi-line on the pasteboard and open the paste control.
paste_into_pane 'export PAYMENTS_WEBHOOK_SECRET=whsec_9f2b
npm run migrate -- --to 2026_08_backoff
npm test -- --run'
shot 33-paste 2

echo
echo "✓ shots in $OUT"
ls -1 "$OUT" | sed 's/^/  /'
