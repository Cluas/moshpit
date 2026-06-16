#!/usr/bin/env bash
# T4 — end-to-end self-verification of Moshi over REAL mosh+tmux to localhost.
#
# What it proves, with screenshots I can inspect myself (no device, no sideload):
#   * Bug A (garble): renders ASCII + CJK + Japanese + box-drawing through the
#     real app → mosh → tmux → SwiftTerm view. A garble shows up in the shot.
#   * Bug B (scroll): swipes the terminal to page tmux history, then shots again
#     — the visible line numbers must move to older content.
#
# Isolation: the app's tmux is pointed at a private socket via a wrapper
# (MOSAIC_SEED_TMUX_BIN), so this NEVER touches your real tmux sessions.
#
# Prereqs: Remote Login on; ~/.ssh/id_ed25519 in authorized_keys; a booted sim;
# idb, tmux, mosh-server installed.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_ID="com.cluas.moshi"
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
printf '=== MOSHI E2E  mosh+tmux  (scroll up to see lower numbers) ===\n'
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

echo "▶ Building Moshi"
DERIVED="$(mktemp -d)"
xcodebuild -project "$REPO_ROOT/Moshi.xcodeproj" -scheme Moshi -configuration Debug \
  -sdk iphonesimulator -destination "platform=iOS Simulator,id=$SIM_UDID" \
  -derivedDataPath "$DERIVED" CODE_SIGNING_ALLOWED=NO build > "$DERIVED/build.log" 2>&1 \
  || { echo "✘ build failed"; tail -30 "$DERIVED/build.log"; exit 1; }
APP="$(find "$DERIVED/Build/Products" -name 'Moshi.app' -type d | head -1)"

echo "▶ Installing + launching (mosh + tmux, isolated socket)"
xcrun simctl install "$SIM_UDID" "$APP"
xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" 2>/dev/null || true
KEY_B64="$(base64 -i "$KEY_PATH" | tr -d '\n')"
xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID" \
  -MOSAIC_SEED_USER "$(whoami)" \
  -MOSAIC_SEED_KEY_B64 "$KEY_B64" \
  -MOSAIC_SEED_HOST "$SSH_HOST" -MOSAIC_SEED_PORT "$SSH_PORT" \
  -MOSAIC_SEED_MOSH 1 -MOSAIC_SEED_MOSH_BIN "$MOSH_SERVER" \
  -MOSAIC_SEED_TMUX 1 -MOSAIC_SEED_TMUX_BIN "$WRAP" >/dev/null

echo "▶ Waiting 14s for SSH→mosh→tmux attach + render…"
sleep 14
LIVE="$OUT_DIR/$TS-1-live.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$LIVE" >/dev/null 2>&1
echo "  live shot: $LIVE"

echo "▶ Scrolling: 4 downward swipes on the terminal (drag-down = older history)"
for _ in 1 2 3 4; do
  idb ui swipe --udid "$SIM_UDID" 200 360 200 680 --duration 0.25 2>/dev/null || \
    idb ui swipe 200 360 200 680 2>/dev/null || true
  sleep 0.7
done
sleep 1
SCROLLED="$OUT_DIR/$TS-2-scrolled.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$SCROLLED" >/dev/null 2>&1
echo "  scrolled shot: $SCROLLED"

echo
echo "✓ Done. Inspect:"
echo "   LIVE     $LIVE     (expect bottom of 60 lines + clean 你好世界/こんにちは/box)"
echo "   SCROLLED $SCROLLED (expect LOWER line numbers = tmux history paged in)"
