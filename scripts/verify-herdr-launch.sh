#!/usr/bin/env bash
# Your private key rides in argv and into the simulator keychain — see the
# "WHERE YOUR PRIVATE KEY GOES" note in scripts/smoke-localhost.sh, and prefer
# a throwaway key via MOSAIC_SSH_KEY.
# Moshpit — end-to-end check that a herdr connection boots herdr's TUI.
#
# Phase 0 of docs/design/herdr-multiplexer.md: Moshpit probes for herdr,
# launches it in the remote shell, and renders its full-screen UI. This script
# proves that against a real herdr server over real SSH (localhost), because
# the unit tests can only prove the command STRING is right.
#
# Builds, installs on a simulator, seeds a 127.0.0.1 connection with
# `-MOSHPIT_SEED_MUX herdr`, waits for the handshake, and screenshots the
# terminal. Expect herdr's UI (sidebar + pane), not a bare shell prompt.
#
# Prerequisites on the Mac:
#   - Remote Login enabled (`sudo systemsetup -setremotelogin on`)
#   - `~/.ssh/id_ed25519` exists, public half in `~/.ssh/authorized_keys`
#   - herdr installed (`brew install herdr`)
#
# The run leaves a herdr SERVER alive on this Mac — same as attaching tmux
# would. `--stop-server` at the end shuts it down; pass MOSAIC_KEEP_SERVER=1
# to keep it for inspection.

set -euo pipefail

SIM_NAME="${MOSAIC_SIM:-iPhone 17 Pro}"
BUNDLE_ID="com.cluas.moshpit"
KEY_PATH="${MOSAIC_SSH_KEY:-$HOME/.ssh/id_ed25519}"
SSH_HOST="${MOSAIC_SSH_HOST:-127.0.0.1}"
SSH_PORT="${MOSAIC_SSH_PORT:-22}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$REPO_ROOT/design-audit/herdr"
mkdir -p "$OUT_DIR"

if [ ! -f "$KEY_PATH" ]; then
  echo "✘ SSH key not found at $KEY_PATH" >&2
  exit 1
fi

if ! ssh -i "$KEY_PATH" -o StrictHostKeyChecking=no -o ConnectTimeout=3 \
         -o BatchMode=yes "$(whoami)@$SSH_HOST" echo ok >/dev/null 2>&1; then
  echo "✘ Can't SSH into $SSH_HOST as $(whoami) with $KEY_PATH" >&2
  echo "  - Enable Remote Login: sudo systemsetup -setremotelogin on" >&2
  echo "  - Add public key:      cat ${KEY_PATH}.pub >> ~/.ssh/authorized_keys" >&2
  exit 1
fi

# Probe herdr exactly the way the app does — same command, same PATH prefix —
# so a failure here is the app's failure, not a different question.
PROBE='PATH="$PATH:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin" command -v herdr'
if ! ssh -i "$KEY_PATH" -o BatchMode=yes "$(whoami)@$SSH_HOST" "$PROBE" >/dev/null 2>&1; then
  echo "✘ herdr not found on $SSH_HOST — install it: brew install herdr" >&2
  exit 1
fi

# MOSAIC_COLD=1 tests the other scenario: herdr installed but NO server
# running. That's what actually bit a user — the app showed a black screen
# with no empty state and no error, because the dead-socket check matched
# herdr 0.8.0's structured error while 0.7.3 prints a bare Rust one. The exit
# status decides now, and the empty state offers to start a server.
if [ "${MOSAIC_COLD:-0}" = "1" ]; then
  echo "▶ Cold-host mode: stopping any server so the app must show the empty state"
  herdr server stop >/dev/null 2>&1 || true
  rm -f "$HOME/.config/herdr/session.json"
  sleep 1
fi

# The app deliberately does NOT start a server: it attaches to the session the
# user already has, and offers an explicit "create" empty state when there is
# none (same rule the tmux path follows). So stand one up first — that's the
# scenario under test.
if [ "${MOSAIC_COLD:-0}" != "1" ] && ! timeout 5 herdr api snapshot >/dev/null 2>&1; then
  echo "▶ No herdr server running — starting one (stands in for the user's own)"
  (nohup herdr server >/dev/null 2>&1 </dev/null &)
  for _ in $(seq 1 20); do
    sleep 0.5
    timeout 3 herdr api snapshot >/dev/null 2>&1 && break
  done
  timeout 5 herdr api snapshot >/dev/null 2>&1 || { echo "✘ couldn't start a herdr server" >&2; exit 1; }
fi

# A server with no persisted state comes up EMPTY — zero workspaces. The app
# offers an explicit "create a session" empty state for exactly this, but the
# scenario under test here is "the user already has a session", so make one.
EMPTY="$(timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
try: snap = json.load(sys.stdin).get('result', {}).get('snapshot', {})
except Exception: raise SystemExit(0)   # no server at all (cold-host mode)
print('yes' if not snap.get('workspaces') else 'no')" || true)"
if [ "${MOSAIC_COLD:-0}" != "1" ] && [ "$EMPTY" = "yes" ]; then
  echo "▶ Server has no workspaces — creating one to attach to"
  timeout 5 herdr workspace create --focus >/dev/null 2>&1 || true
fi

echo "▶ Resolving simulator: $SIM_NAME"
SIM_UDID="$(xcrun simctl list devices --json | python3 -c "
import json, sys
data = json.load(sys.stdin)
target = sys.argv[1]
booted = available = None
for _runtime, devices in data['devices'].items():
    for d in devices:
        if d.get('name') == target and d.get('isAvailable', False):
            if d.get('state') == 'Booted' and booted is None: booted = d['udid']
            elif available is None: available = d['udid']
print(booted or available or '')
" "$SIM_NAME")"
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

echo "▶ Building Moshpit"
DERIVED="$(mktemp -d)"
xcodebuild \
  -project "$REPO_ROOT/Moshpit.xcodeproj" -scheme Moshpit \
  -configuration Debug -sdk iphonesimulator \
  -destination "platform=iOS Simulator,name=$SIM_NAME" \
  -derivedDataPath "$DERIVED" \
  CODE_SIGNING_ALLOWED=NO build > "$DERIVED/build.log" 2>&1 \
  || { echo "✘ build failed"; tail -30 "$DERIVED/build.log"; exit 1; }
APP="$(find "$DERIVED/Build/Products" -name 'Moshpit.app' -type d | head -1)"

echo "▶ Installing app on $SIM_UDID"
xcrun simctl install "$SIM_UDID" "$APP"
xcrun simctl status_bar "$SIM_UDID" override \
  --time "9:41" --batteryState charged --batteryLevel 100 \
  --wifiBars 3 --cellularBars 4 --dataNetwork wifi 2>/dev/null || true
xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" 2>/dev/null || true

# MOSAIC_MOSH=1 runs the same check over mosh. Both transports render herdr's
# TUI identically in Phase 0 (Moshpit is only the renderer), so the one thing
# this actually distinguishes is whether the launch line reaches the shell
# through the mosh keystroke channel rather than the SSH PTY.
TRANSPORT_ARGS=()
LABEL="ssh"
if [ "${MOSAIC_MOSH:-0}" = "1" ]; then
  TRANSPORT_ARGS=(-MOSHPIT_SEED_MOSH 1)
  LABEL="mosh"
fi

echo "▶ Launching with multiplexer=herdr over $LABEL: $(whoami)@$SSH_HOST:$SSH_PORT"
KEY_B64="$(base64 -i "$KEY_PATH" | tr -d '\n')"
# SEED_QUIET turns local notifications off for this run. Not cosmetic: iOS asks
# for notification permission the first time a bundle id ever runs, and a SYSTEM
# alert makes `idb ui describe-all` return only the alert's own elements — every
# assertion below then reads an empty tree and reports "the poller isn't feeding
# the UI" while the app behind it is perfectly fine. (Cost a rename to find:
# com.cluas.ringdown had long since been granted on this simulator, so the
# prompt only reappeared once the bundle id changed.) Agent tracking is
# unaffected — it rides `liveActivityEnabled`, and the assertions here read the
# control plane, not the notification centre.
xcrun simctl launch "$SIM_UDID" "$BUNDLE_ID" \
  -MOSHPIT_SEED_USER "$(whoami)" \
  -MOSHPIT_SEED_KEY_B64 "$KEY_B64" \
  -MOSHPIT_SEED_HOST "$SSH_HOST" \
  -MOSHPIT_SEED_PORT "$SSH_PORT" \
  -MOSHPIT_SEED_MUX herdr \
  -MOSHPIT_SEED_QUIET 1 \
  "${TRANSPORT_ARGS[@]}" >/dev/null

echo "▶ Waiting 6s for the SSH handshake…"
sleep 6

# First connection to a host raises the TOFU "New Host" sheet, which blocks
# the session before herdr is ever launched. Tap Trust when it's there —
# located through the accessibility tree rather than hardcoded coordinates so
# this survives a different simulator or a layout change.
TRUST_POINT="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
for el in (data if isinstance(data, list) else [data]):
    if isinstance(el, dict) and el.get('AXLabel') == 'Trust':
        f = el.get('frame') or {}
        print(int(f['x'] + f['width'] / 2), int(f['y'] + f['height'] / 2))
        break
" || true)"
if [ -n "$TRUST_POINT" ]; then
  echo "▶ Trusting the host key (first connection): tap $TRUST_POINT"
  # shellcheck disable=SC2086
  idb ui tap --udid "$SIM_UDID" $TRUST_POINT >/dev/null 2>&1 || true
fi

# Belt and braces for the hazard above: if ANY system alert is up (a future
# permission, a StoreKit sheet), grant it and move on — an alert left standing
# blinds every assertion in this script.
ALERT_POINT="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
for el in (data if isinstance(data, list) else [data]):
    if isinstance(el, dict) and (el.get('AXLabel') or '') in ('Allow', 'OK'):
        f = el.get('frame') or {}
        print(int(f['x'] + f['width'] / 2), int(f['y'] + f['height'] / 2))
        break
" || true)"
if [ -n "$ALERT_POINT" ]; then
  echo "▶ Dismissing a system alert that would blind the a11y tree: tap $ALERT_POINT"
  # shellcheck disable=SC2086
  idb ui tap --udid "$SIM_UDID" $ALERT_POINT >/dev/null 2>&1 || true
  sleep 1
fi

echo "▶ Waiting 8s for herdr's first paint…"
sleep 8

# Cold host: the app must SAY there is no session and offer to start one —
# not sit on a black screen. Then the button has to actually work.
if [ "${MOSAIC_COLD:-0}" = "1" ]; then
  echo "▶ Cold host: empty state instead of a black screen"
  CREATE_POINT="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
for el in (data if isinstance(data, list) else [data]):
    label = el.get('AXLabel') or '' if isinstance(el, dict) else ''
    if label.startswith('Create '):
        f = el.get('frame') or {}
        print(int(f['x'] + f['width'] / 2), int(f['y'] + f['height'] / 2))
        break
")"
  if [ -z "$CREATE_POINT" ]; then
    echo "  ✘ no empty state — the app is showing nothing and offering nothing" >&2
    exit 1
  fi
  echo "  ✓ empty state offers to create one"
  # shellcheck disable=SC2086
  idb ui tap --udid "$SIM_UDID" --duration 0.25 $CREATE_POINT >/dev/null 2>&1 || true
  for _ in $(seq 1 24); do
    sleep 0.5
    timeout 3 herdr api snapshot >/dev/null 2>&1 && break
  done
  if timeout 5 herdr api snapshot >/dev/null 2>&1; then
    echo "  ✓ the create button started a server on a host that had none"
  else
    echo "  ✘ the create button did nothing — cold hosts are still a dead end" >&2
    exit 1
  fi
  sleep 6   # let the poller pick up the new tree and the frame channel target
fi

STAMP="$(date +%H%M%S)"
OUT="$OUT_DIR/$STAMP-herdr-$LABEL.png"
xcrun simctl io "$SIM_UDID" screenshot --type=png "$OUT" >/dev/null 2>&1

# Independent confirmation that a server actually came up — the screenshot
# alone can't tell "herdr's UI" from "a shell that printed an error".
echo "▶ Server state per the socket API:"
if herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
# Responses are envelopes: {\"id\":…, \"result\":{\"snapshot\":{…}}}
snap = json.load(sys.stdin).get('result', {}).get('snapshot', {})
panes = snap.get('panes', [])
if not panes: raise SystemExit(1)
print('  workspaces: %d  tabs: %d  panes: %d'
      % (len(snap.get('workspaces', [])), len(snap.get('tabs', [])), len(panes)))
print('  protocol:', snap.get('protocol'), ' version:', snap.get('version'))
"; then
  echo "  ✓ herdr server is live"
else
  echo "  ✘ no herdr server answered — the launch line did not take" >&2
  echo "    Screenshot: $OUT" >&2
  exit 1
fi

# Phase 1: the control plane. `herdr api snapshot` above proves the SERVER is
# alive; this proves Moshpit is reading it — the breadcrumb only renders
# session/window crumbs when a snapshot has been decoded into the app's model.
echo "▶ Control plane (breadcrumb driven by herdr api snapshot):"
CRUMBS="$(idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: sys.exit(0)
labels = [el.get('AXLabel') for el in (data if isinstance(data, list) else [data])
          if isinstance(el, dict) and el.get('type') == 'Button' and el.get('AXLabel')]
# The window crumb is the tab's INDEX, plus its name when the name says
# something the number doesn't (see BreadcrumbPlan.make) — so 'n' or 'n:name'.
# Either way it can only come from a decoded tree, while a session name alone
# could be coincidence. Matching 'pane N' too, which is the crumb herdr 0.7.3
# produces (its panes carry no command).
import re
print(next((l for l in labels if l.startswith('pane ') or re.fullmatch(r'\d+(:.+)?', l)), ''))
")"
if [ -n "$CRUMBS" ]; then
  echo "  ✓ app is reading the tree (crumb: $CRUMBS)"
else
  echo "  ✘ no session/window crumb — the snapshot poller isn't feeding the UI" >&2
  echo "    Screenshot: $OUT" >&2
  exit 1
fi

# Phase 2 (SSH only): native single-pane rendering. Over mosh the app still
# runs herdr's own TUI, since mosh can't carry the line-framed protocol.
if [ "$LABEL" = "ssh" ]; then
  echo "▶ Frame channel (native rendering, not herdr's TUI):"
  # Work in a tab this script creates, so the assertions don't depend on
  # whatever layout the server restored from a previous run.
  PROBE_TAB="$(timeout 5 herdr tab create --label moshpit-probe --focus 2>/dev/null | python3 -c "
import json, sys
try: print(json.load(sys.stdin)['result']['tab']['tab_id'])
except Exception: print('')
")"
  [ -n "$PROBE_TAB" ] || { echo "  ✘ couldn't create a probe tab" >&2; exit 1; }
  # Changes made OUT of band — like this one — wait on the control poll, which
  # eases off to 8s when the tree is quiet. In-app actions refresh immediately;
  # this doesn't, so wait past a full idle interval.
  sleep 12

  # A single-pane tab means herdr's layout rect IS the whole area, so the
  # pane's width tells the two modes apart: the frame channel sizes it to the
  # phone's full grid, while herdr's own TUI keeps a sidebar's worth back.
  WIDTH="$(timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
snap = json.load(sys.stdin).get('result', {}).get('snapshot', {})
focused = snap.get('focused_pane_id')
for layout in snap.get('layouts', []):
    for pane in layout.get('panes', []):
        if pane.get('pane_id') == focused:
            print(pane.get('rect', {}).get('width', 0)); raise SystemExit
print(0)
")"
  if [ "${WIDTH:-0}" -ge 48 ]; then
    echo "  ✓ pane is full phone width (${WIDTH} cols) — no sidebar in the way"
  else
    echo "  ✘ pane is only ${WIDTH} cols — looks like herdr's TUI, not the frame channel" >&2
    exit 1
  fi

  # Round-trip a keystroke: typed into the app, it can only reach the pane by
  # travelling as a `terminal.input` message down the frame channel.
  # Tap the terminal first: that's what a person does, and it takes the
  # hardware-keyboard injection below out of a race with the view transition
  # that just happened (creating a tab, or coming off the empty state).
  idb ui tap --udid "$SIM_UDID" --duration 0.15 200 400 >/dev/null 2>&1 || true
  sleep 1
  MARKER="moshpit-frame-probe-$$"
  idb ui text --udid "$SIM_UDID" "echo $MARKER" >/dev/null 2>&1 || true
  idb ui key --udid "$SIM_UDID" 40 >/dev/null 2>&1 || true
  sleep 2
  FOCUSED="$(timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
print(json.load(sys.stdin).get('result', {}).get('snapshot', {}).get('focused_pane_id', ''))")"
  if timeout 5 herdr pane read "$FOCUSED" --source visible --lines 8 2>/dev/null | grep -qi "$MARKER"; then
    echo "  ✓ typing in the app reached the pane over the frame channel"
  else
    echo "  ✘ keystrokes never arrived in $FOCUSED" >&2
    exit 1
  fi

  # Retarget. Switching panes means releasing the current attach and starting
  # a new one on the same channel, and the release makes the server emit a
  # `terminal.closed` for the pane we left. Mishandling that close silently
  # drops input back to raw bytes — frames keep painting while typing goes
  # nowhere, which looks like everything works. Hence: switch, then type.
  echo "▶ Retarget (switch panes, keep typing):"
  NEW_PANE="$(timeout 5 herdr pane split "$FOCUSED" --direction right --focus 2>/dev/null | python3 -c "
import json, sys
try: print(json.load(sys.stdin)['result']['pane']['pane_id'])
except Exception: print('')
")"
  if [ -z "$NEW_PANE" ]; then
    echo "  ✘ couldn't split a second pane to switch to" >&2
    exit 1
  fi
  sleep 12  # same out-of-band wait as above: poll, then release + restart
  # Tap the terminal first: that's what a person does, and it takes the
  # hardware-keyboard injection below out of a race with the view transition
  # that just happened (creating a tab, or coming off the empty state).
  idb ui tap --udid "$SIM_UDID" --duration 0.15 200 400 >/dev/null 2>&1 || true
  sleep 1
  MARKER2="moshpit-retarget-probe-$$"
  idb ui text --udid "$SIM_UDID" "echo $MARKER2" >/dev/null 2>&1 || true
  idb ui key --udid "$SIM_UDID" 40 >/dev/null 2>&1 || true
  sleep 3
  if timeout 5 herdr pane read "$NEW_PANE" --source visible --lines 8 2>/dev/null | grep -qi "$MARKER2"; then
    echo "  ✓ input follows the switch into $NEW_PANE"
  else
    echo "  ✘ after switching to $NEW_PANE, typing no longer reaches the pane" >&2
    exit 1
  fi
  # Agent status. herdr reports it natively, so nothing is installed on the
  # host — `pane report-agent` is the same interface herdr's own integrations
  # use. Reaching the breadcrumb proves it made it through the decoder into
  # the app's model, which is what the Vibe Island reads.
  echo "▶ Agent status (no host-side hooks):"
  AGENT_PANE="$(timeout 5 herdr api snapshot 2>/dev/null | python3 -c "
import json, sys
print(json.load(sys.stdin).get('result', {}).get('snapshot', {}).get('focused_pane_id', ''))")"
  timeout 5 herdr pane report-agent "$AGENT_PANE" --source moshpit-verify \
    --agent "Claude Code" --state working >/dev/null 2>&1 || true
  # The control poll eases off to 8s once the tree stops changing, so a change
  # made OUT of band — like this one — can take that long to show up. Wait past
  # a full idle interval rather than racing it.
  sleep 12
  if idb ui describe-all --udid "$SIM_UDID" 2>/dev/null | python3 -c "
import sys, json
try: data = json.load(sys.stdin)
except Exception: raise SystemExit(1)
labels = [e.get('AXLabel') or '' for e in (data if isinstance(data, list) else [data]) if isinstance(e, dict)]
raise SystemExit(0 if any('Claude Code' in l for l in labels) else 1)
"; then
    echo "  ✓ herdr's agent state reached the app (breadcrumb names the agent)"
  else
    echo "  ✘ agent state never surfaced in the app" >&2
    exit 1
  fi
  timeout 5 herdr pane release-agent "$AGENT_PANE" --source moshpit-verify \
    --agent "Claude Code" >/dev/null 2>&1 || true

  timeout 5 herdr tab close "$PROBE_TAB" >/dev/null 2>&1 || true
fi

# Leave no client behind. herdr's direct attach is exclusive per terminal, so
# a simulator left running keeps re-attaching every couple of seconds and will
# fight anyone else on the same host for the same pane — twice during this
# feature's development that quietly broke a real device's session, with the
# screen repainting while every keystroke went nowhere.
if [ "${MOSAIC_KEEP_APP:-0}" != "1" ]; then
  echo "▶ Terminating the simulator app so it stops competing for the attach"
  xcrun simctl terminate "$SIM_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
fi

if [ "${MOSAIC_KEEP_SERVER:-0}" != "1" ]; then
  echo "▶ Stopping the herdr server this run started"
  herdr server stop >/dev/null 2>&1 || true
fi

echo
echo "✓ Done."
echo "  Screenshot: $OUT"
echo "  Open with:  open $OUT"
if [ "$LABEL" = "ssh" ]; then
  echo "  Expect ONE full-width pane under Moshpit's own breadcrumb — herdr's"
  echo "  sidebar and tab bar should be absent (that's the frame channel)."
else
  echo "  Expect herdr's own TUI (sidebar + tab bar): mosh can't carry the"
  echo "  frame protocol, so it renders herdr's interface as-is."
fi
