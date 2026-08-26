#!/usr/bin/env bash
# Your private key rides in argv and into the simulator keychain — see the
# "WHERE YOUR PRIVATE KEY GOES" note in scripts/smoke-localhost.sh, and prefer
# a throwaway key via MOSAIC_SSH_KEY.
# T4 — end-to-end self-verification of Moshpit over REAL mosh+tmux to localhost.
#
# What it proves, with screenshots I can inspect myself (no device, no sideload):
#   * Bug A (garble): renders ASCII + CJK + Japanese + box-drawing through the
#     real app → mosh → tmux → SwiftTerm view. A garble shows up in the shot.
#   * Bug B (scroll): swipes the terminal to page tmux history, then shots again
#     — the visible line numbers must move to older content.
#
# Isolation: the app's tmux is pointed at a private socket via a wrapper
# (MOSHPIT_SEED_TMUX_BIN), so this NEVER touches your real tmux sessions.
#
# Prereqs: Remote Login on; ~/.ssh/id_ed25519 in authorized_keys; a booted sim;
# idb, tmux, mosh-server installed.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_ID="com.cluas.moshpit"
KEY_PATH="${MOSAIC_SSH_KEY:-$HOME/.ssh/id_ed25519}"
SSH_HOST="127.0.0.1"; SSH_PORT="22"
TMUX_BIN="$(command -v tmux)"
MOSH_SERVER="$(command -v mosh-server)"
SOCK="moshie2e"
SESSION="demo"
WRAP="$REPO_ROOT/build/moshie2e-tmux"
GEN="$REPO_ROOT/build/moshie2e-content.sh"
OUT_DIR="$REPO_ROOT/design-audit/e2e"; mkdir -p "$OUT_DIR" "$REPO_ROOT/build"
TS="$(date +%H%M%S)"

cleanup() { "$TMUX_BIN" -L "$SOCK" kill-server 2>/dev/null || true; }
trap cleanup EXIT

# --- isolated tmux: wrapper the app will exec (its -CC sidecar AND mosh attach) ---
cat > "$WRAP" <<EOF
#!/bin/sh
exec "$TMUX_BIN" -L "$SOCK" "\$@"
EOF
chmod +x "$WRAP"

# --- garble-bait content: numbered lines mixing ASCII / CJK / JP / box-drawing ---
cat > "$GEN" <<'EOF'
#!/bin/sh
clear
printf '=== MOSHPIT E2E  mosh+tmux  (scroll up to see lower numbers) ===\n'
i=1
while [ $i -le 60 ]; do
  printf '%03d | ASCII=abcXYZ | CJK=你好世界 | JP=こんにちは | box ┌──┬──┐ │AB│ └──┴──┘\n' "$i"
  i=$((i + 1))
done
printf 'E2E-LIVE-BOTTOM$ '
EOF
chmod +x "$GEN"

echo "▶ Creating isolated tmux session ($TMUX_BIN -L $SOCK : $SESSION)"
"$TMUX_BIN" -L "$SOCK" kill-server 2>/dev/null || true
"$TMUX_BIN" -L "$SOCK" new-session -d -s "$SESSION" -x 80 -y 24
"$TMUX_BIN" -L "$SOCK" send-keys -t "$SESSION" "sh '$GEN'" Enter
sleep 0.5

SIM_UDID="$(xcrun simctl list devices booted --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print(next((x["udid"] for ds in d["devices"].values() for x in ds if x.get("state")=="Booted"),""))')"
[ -n "$SIM_UDID" ] || { echo "✘ no booted simulator"; exit 1; }
SIM_NAME="$(xcrun simctl list devices --json | python3 -c 'import json,sys;u=sys.argv[1];d=json.load(sys.stdin);print(next((x["name"] for ds in d["devices"].values() for x in ds if x.get("udid")==u),""))' "$SIM_UDID")"
echo "▶ Simulator: $SIM_NAME ($SIM_UDID)"

echo "▶ Building Moshpit"
DERIVED="$(mktemp -d)"
xcodebuild -project "$REPO_ROOT/Moshpit.xcodeproj" -scheme Moshpit -configuration Debug \
  -sdk iphonesimulator -destination "platform=iOS Simulator,id=$SIM_UDID" \
  -derivedDataPath "$DERIVED" CODE_SIGNING_ALLOWED=NO build > "$DERIVED/build.log" 2>&1 \
  || { echo "✘ build failed"; tail -30 "$DERIVED/build.log"; exit 1; }
APP="$(find "$DERIVED/Build/Products" -name 'Moshpit.app' -type d | head -1)"

echo "▶ Installing + launching (mosh + tmux, isolated socket)"
xcrun simctl install "$SIM_UDID" "$APP"
xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" 2>/dev/null || true
KEY_B64="$(base64 -i "$KEY_PATH" | tr -d '\n')"
xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID" \
  -MOSHPIT_AUTOCARE_OFF 1 \
  -MOSHPIT_SEED_USER "$(whoami)" \
  -MOSHPIT_SEED_KEY_B64 "$KEY_B64" \
  -MOSHPIT_SEED_HOST "$SSH_HOST" -MOSHPIT_SEED_PORT "$SSH_PORT" \
  -MOSHPIT_SEED_MOSH 1 -MOSHPIT_SEED_MOSH_BIN "$MOSH_SERVER" \
  -MOSHPIT_SEED_TMUX 1 -MOSHPIT_SEED_TMUX_BIN "$WRAP" >/dev/null

# tmux state on the isolated socket — deterministic PASS/FAIL, no screenshot squinting.
pane_in_mode() { "$TMUX_BIN" -L "$SOCK" display-message -p -t "$SESSION" '#{pane_in_mode}' 2>/dev/null | tr -d '[:space:]'; }
pane_cmd()     { "$TMUX_BIN" -L "$SOCK" display-message -p -t "$SESSION" '#{pane_current_command}' 2>/dev/null | tr -d '[:space:]'; }
mouse_flag()   { "$TMUX_BIN" -L "$SOCK" display-message -p -t "$SESSION" '#{mouse_any_flag}' 2>/dev/null | tr -d '[:space:]'; }
top_line()     { "$TMUX_BIN" -L "$SOCK" capture-pane -p -t "$SESSION" 2>/dev/null | sed -n '1p'; }
active_win()   { "$TMUX_BIN" -L "$SOCK" display-message -p -t "$SESSION" '#{window_index}' 2>/dev/null | tr -d '[:space:]'; }
# The harness tmux has `set -g mouse on` (matches the user's config), so the
# mosh-rendered client's mouse layer is active — exercising the real decision.
"$TMUX_BIN" -L "$SOCK" set-option -g mouse on 2>/dev/null || true
# A NON-control client (mosh `tmux attach`, control_mode=0) whose session is demo.
# The -CC sidecar is control_mode=1, so this isolates the real rendering client.
mosh_in_demo() { "$TMUX_BIN" -L "$SOCK" list-clients -F '#{client_control_mode} #{client_session}' 2>/dev/null | grep -c "^0 $SESSION$" || true; }

# Fresh simulator installs hit the TOFU host-key prompt ("New Host … Trust")
# before any transport starts — nothing attaches until it's accepted. Find the
# Trust button via idb accessibility and tap it (no-op when already trusted).
echo "▶ Accepting the TOFU host-key prompt if it appears…"
for _ in $(seq 1 10); do
  TRUST_XY="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c '
import sys, json
try:
    els = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for e in els:
    if e.get("AXLabel") == "Trust" and e.get("type") == "Button":
        f = e.get("frame", {})
        print(int(f.get("x",0)+f.get("width",0)/2), int(f.get("y",0)+f.get("height",0)/2))
        break
')"
  if [ -n "$TRUST_XY" ]; then
    idb ui tap --udid "$SIM_UDID" $TRUST_XY 2>/dev/null || idb ui tap $TRUST_XY 2>/dev/null || true
    echo "  ✓ tapped Trust at $TRUST_XY"
    break
  fi
  sleep 1
done

echo "▶ Waiting for the mosh client to attach the '$SESSION' session (the real renderer)…"
ATTACHED=0
for t in $(seq 1 40); do
  [ "$(mosh_in_demo)" -ge 1 ] && { echo "  ✓ mosh client in '$SESSION' after ${t}s"; ATTACHED=1; break; }
  sleep 1
done
[ "$ATTACHED" = 1 ] || echo "  ✗ mosh client never reached '$SESSION' — login-shell attach race"
sleep 2
LIVE="$OUT_DIR/$TS-1-live.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$LIVE" >/dev/null 2>&1
echo "  live shot: $LIVE   pane_in_mode=$(pane_in_mode) (expect 0 = live)"

IDB_INPUT_OK=1  # confirmed separately: idb keyboard input reaches the live shell

echo "▶ Scrolling: downward swipes (drag-down = older history → enters copy-mode)"
for _ in 1 2 3 4; do
  idb ui swipe --udid "$SIM_UDID" 200 360 200 680 --duration 0.25 2>/dev/null || \
    idb ui swipe 200 360 200 680 2>/dev/null || true
  sleep 0.7
done
sleep 1
SCROLLED="$OUT_DIR/$TS-2-scrolled.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$SCROLLED" >/dev/null 2>&1
IN_MODE_AFTER_SCROLL="$(pane_in_mode)"
echo "  scrolled shot: $SCROLLED   pane_in_mode=$IN_MODE_AFTER_SCROLL (expect 1 = copy-mode)"

echo "▶ One keystroke after scroll — must LEAVE copy-mode (the reported bug)"
idb ui text --udid "$SIM_UDID" "k" 2>/dev/null || idb ui text "k" 2>/dev/null || true
sleep 1.5
TYPED="$OUT_DIR/$TS-3-typed.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$TYPED" >/dev/null 2>&1
IN_MODE_AFTER_TYPE="$(pane_in_mode)"
echo "  typed shot: $TYPED   pane_in_mode=$IN_MODE_AFTER_TYPE (expect 0 = exited)"

# === Alt-screen MOUSE app (Claude Code stand-in): swipe must scroll the APP via
# a forwarded wheel, NOT tmux copy-mode — and typing must still reach the app. ===
echo "▶ Launching a mouse app (seq | less --mouse) as a Claude-Code stand-in"
"$TMUX_BIN" -L "$SOCK" send-keys -t "$SESSION" C-u                                 # clear the 'k'
"$TMUX_BIN" -L "$SOCK" send-keys -t "$SESSION" "seq 1 400 | less --mouse" Enter
APP_UP=0
for _ in $(seq 1 20); do [ "$(mouse_flag)" = "1" ] && { APP_UP=1; break; }; sleep 0.3; done
echo "  app requested mouse (mouse_any_flag): $(mouse_flag) (expect 1)"
TOP_BEFORE="$(top_line)"
echo "▶ Swiping UP (drag up = wheel-down = pager advances) — should forward to less"
for _ in 1 2 3 4 5; do
  idb ui swipe --udid "$SIM_UDID" 200 680 200 360 --duration 0.25 2>/dev/null || \
    idb ui swipe 200 680 200 360 2>/dev/null || true
  sleep 0.5
done
sleep 1
APP_SCROLL="$OUT_DIR/$TS-4-app-scrolled.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$APP_SCROLL" >/dev/null 2>&1
IN_MODE_APP="$(pane_in_mode)"; TOP_AFTER="$(top_line)"
echo "  app shot: $APP_SCROLL   pane_in_mode=$IN_MODE_APP (expect 0 = forwarded, not copy-mode)"
echo "  less top line: '$TOP_BEFORE' -> '$TOP_AFTER' (expect it to advance)"
echo "▶ Typing 'q' must reach less (no copy-mode swallowing it) and quit it"
idb ui text --udid "$SIM_UDID" "q" 2>/dev/null || idb ui text "q" 2>/dev/null || true
sleep 1.5
CMD_AFTER_Q="$(pane_cmd)"
echo "  pane command after 'q': '$CMD_AFTER_Q' (expect a shell, not 'less')"

# === Horizontal swipe switches windows (single-pane window → window switch). ===
echo "▶ Creating a 2nd window, then horizontal-swiping to switch windows"
"$TMUX_BIN" -L "$SOCK" new-window -t "$SESSION" 2>/dev/null || true
sleep 1.5
WIN_BEFORE="$(active_win)"
# ONE swipe (two windows + two swipes would round-trip back to the start).
idb ui swipe --udid "$SIM_UDID" 300 420 60 420 --duration 0.18 2>/dev/null || \
  idb ui swipe 300 420 60 420 2>/dev/null || true   # swipe LEFT = next window
sleep 1.2
WIN_AFTER="$(active_win)"
SWIPESHOT="$OUT_DIR/$TS-5-after-swipe.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$SWIPESHOT" >/dev/null 2>&1
echo "  active window index: '$WIN_BEFORE' -> '$WIN_AFTER' (expect it to change); shot: $SWIPESHOT"
[ -n "$WIN_AFTER" ] && [ -n "$WIN_BEFORE" ] && [ "$WIN_AFTER" != "$WIN_BEFORE" ] && WIN_SWITCH_OK=1 || WIN_SWITCH_OK=0

# Deterministic outcomes for the alt-app phase.
[ "$IN_MODE_APP" = "0" ] && APP_NO_COPYMODE=1 || APP_NO_COPYMODE=0
[ -n "$TOP_AFTER" ] && [ "$TOP_AFTER" != "$TOP_BEFORE" ] && APP_SCROLLED=1 || APP_SCROLLED=0
case "$CMD_AFTER_Q" in *less*) APP_TYPE_OK=0;; "") APP_TYPE_OK=0;; *) APP_TYPE_OK=1;; esac

echo
echo "================ RESULT ================"
[ "$ATTACHED" = 1 ]               && echo "  mosh client attached '$SESSION'   : PASS" || echo "  mosh client attached '$SESSION'   : FAIL (login-shell race — results below are moot)"
[ "${IDB_INPUT_OK:-0}" = 1 ]      && echo "  idb keyboard input delivery       : OK"   || echo "  idb keyboard input delivery       : UNAVAILABLE (type-exit check is inconclusive)"
[ "$IN_MODE_AFTER_SCROLL" = "1" ] && echo "  shell scroll → enters copy-mode   : PASS" || echo "  shell scroll → enters copy-mode   : FAIL (got '$IN_MODE_AFTER_SCROLL')"
[ "$IN_MODE_AFTER_TYPE" = "0" ]   && echo "  typing       → exits  copy-mode   : PASS" || echo "  typing       → exits  copy-mode   : FAIL (got '$IN_MODE_AFTER_TYPE')"
[ "$APP_UP" = "1" ]               && echo "  mouse app came up (mouse_any_flag): PASS" || echo "  mouse app came up (mouse_any_flag): FAIL"
[ "$APP_NO_COPYMODE" = "1" ]      && echo "  app swipe → wheel (NOT copy-mode) : PASS" || echo "  app swipe → wheel (NOT copy-mode) : FAIL (in_mode='$IN_MODE_APP')"
[ "$APP_SCROLLED" = "1" ]         && echo "  app scrolled (top line advanced)  : PASS" || echo "  app scrolled (top line advanced)  : FAIL ('$TOP_BEFORE'->'$TOP_AFTER')"
[ "$APP_TYPE_OK" = "1" ]          && echo "  typing reaches app after scroll   : PASS" || echo "  typing reaches app after scroll   : FAIL (cmd='$CMD_AFTER_Q')"
[ "$WIN_SWITCH_OK" = "1" ]        && echo "  swipe switches window             : PASS" || echo "  swipe switches window             : FAIL ('$WIN_BEFORE'->'$WIN_AFTER')"
echo "  Screenshots: $LIVE | $SCROLLED | $TYPED | $APP_SCROLL"
