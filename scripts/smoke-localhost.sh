#!/usr/bin/env bash
# Mosaic — end-to-end localhost SSH smoke test.
#
# Builds Beacon, installs onto an iPhone simulator, launches with seed
# args that:
#   1. Write `~/.ssh/id_ed25519` into the app's keychain
#   2. Register a `127.0.0.1:22` connection
#   3. Auto-navigate straight to TerminalContainerView
#
# Sleeps a few seconds to let the SSH session open, then screenshots the
# terminal so a reviewer can confirm the shell prompt rendered. Exits 0
# on success.
#
# Prerequisites on the Mac:
#   - Remote Login enabled (`sudo systemsetup -setremotelogin on`)
#   - `~/.ssh/id_ed25519` exists with its public key in `~/.ssh/authorized_keys`
#   - At least one iPhone simulator runtime installed (defaults to iPhone 17 Pro)

set -euo pipefail

SIM_NAME="${MOSAIC_SIM:-iPhone 17 Pro}"
BUNDLE_ID="com.cluas.beacon"
KEY_PATH="${MOSAIC_SSH_KEY:-$HOME/.ssh/id_ed25519}"
SSH_HOST="${MOSAIC_SSH_HOST:-127.0.0.1}"
SSH_PORT="${MOSAIC_SSH_PORT:-22}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$REPO_ROOT/design-audit/smoke"
mkdir -p "$OUT_DIR"

if [ ! -f "$KEY_PATH" ]; then
  echo "✘ SSH key not found at $KEY_PATH" >&2
  echo "  generate one with: ssh-keygen -t ed25519 -f $KEY_PATH -N ''" >&2
  exit 1
fi

if ! ssh -i "$KEY_PATH" -o StrictHostKeyChecking=no -o ConnectTimeout=3 \
         -o BatchMode=yes "$(whoami)@$SSH_HOST" echo ok >/dev/null 2>&1; then
  echo "✘ Can't SSH into $SSH_HOST as $(whoami) with $KEY_PATH" >&2
  echo "  - Enable Remote Login: sudo systemsetup -setremotelogin on" >&2
  echo "  - Add public key:      cat ${KEY_PATH}.pub >> ~/.ssh/authorized_keys" >&2
  exit 1
fi

echo "▶ Resolving simulator: $SIM_NAME"
resolve_udid() {
  xcrun simctl list devices --json | python3 -c "
import json, sys
data = json.load(sys.stdin)
target = sys.argv[1]
booted, available = None, None
for _runtime, devices in data['devices'].items():
    for d in devices:
        if d.get('name') == target and d.get('isAvailable', False):
            if d.get('state') == 'Booted' and booted is None: booted = d['udid']
            elif available is None: available = d['udid']
print(booted or available or '')
" "$1"
}
SIM_UDID="$(resolve_udid "$SIM_NAME")"
[ -n "$SIM_UDID" ] || { echo "✘ no simulator named '$SIM_NAME'"; exit 1; }

STATE="$(xcrun simctl list devices --json | python3 -c "
import json, sys
data = json.load(sys.stdin)
for _r, ds in data['devices'].items():
    for d in ds:
        if d.get('udid') == sys.argv[1]:
            print(d.get('state', 'Unknown')); sys.exit(0)
" "$SIM_UDID")"
if [ "$STATE" != "Booted" ]; then
  echo "▶ Booting $SIM_UDID"
  xcrun simctl boot "$SIM_UDID"
  open -a Simulator
  sleep 10
fi

echo "▶ Building Beacon"
DERIVED="$(mktemp -d)"
xcodebuild \
  -project "$REPO_ROOT/Beacon.xcodeproj" -scheme Beacon \
  -configuration Debug -sdk iphonesimulator \
  -destination "platform=iOS Simulator,name=$SIM_NAME" \
  -derivedDataPath "$DERIVED" \
  CODE_SIGNING_ALLOWED=NO build > "$DERIVED/build.log" 2>&1 \
  || { echo "✘ build failed"; tail -30 "$DERIVED/build.log"; exit 1; }
APP="$(find "$DERIVED/Build/Products" -name 'Beacon.app' -type d | head -1)"

echo "▶ Installing app on $SIM_UDID"
xcrun simctl install "$SIM_UDID" "$APP"
xcrun simctl status_bar "$SIM_UDID" override \
  --time "9:41" --batteryState charged --batteryLevel 100 \
  --wifiBars 3 --cellularBars 4 --dataNetwork wifi 2>/dev/null || true
xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" 2>/dev/null || true

echo "▶ Launching with seed args: $(whoami)@$SSH_HOST:$SSH_PORT"
KEY_B64="$(base64 -i "$KEY_PATH" | tr -d '\n')"
xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID" \
  -BEACON_SEED_USER "$(whoami)" \
  -BEACON_SEED_KEY_B64 "$KEY_B64" \
  -BEACON_SEED_HOST "$SSH_HOST" \
  -BEACON_SEED_PORT "$SSH_PORT" >/dev/null

echo "▶ Waiting 6s for SSH handshake + first PTY output…"
sleep 6

OUT="$OUT_DIR/$(date +%H%M%S)-localhost.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$OUT" >/dev/null 2>&1
echo
echo "✓ Done."
echo "  Screenshot: $OUT"
echo
echo "  Open with: open $OUT"
echo "  Expect the terminal viewport to show a shell login banner + prompt."
