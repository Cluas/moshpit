#!/bin/sh
# What an attach actually costs, measured on a REAL device.
#
# There is no pixel evidence here on purpose. XCUITest is the only way to tap or
# screenshot an iPhone/iPad, and it needs a provisioning profile for the test
# RUNNER's own bundle id — which this repo cannot mint (no Apple ID in Xcode;
# developerservices2 rejects the ASC API key, see scripts/release-archive.sh).
# `idevicescreenshot` is no help either: the screenshotr service is gone on
# iOS 17+. So the evidence is the control-mode byte stream, which is the thing
# that actually costs time on a slow link, and `-MOSHPIT_CC_TAP documents`
# records it into the app sandbox for `devicectl device copy from` to fetch.
#
# The session is an ISOLATED tmux socket built here — the user's real panes are
# never attached to, because a phone-sized -CC client re-pins window sizes.
#
# Usage: scripts/verify-device-attach.sh <device-udid> <host-reachable-from-device>
set -eu
DEV=${1:?device udid (xcrun devicectl list devices)}
HOST=${2:?an address of THIS mac that the device can reach}
BUNDLE=com.cluas.moshpit
T=$(command -v tmux)
S=deviceattach
OUT=$(mktemp -d)
WRAP="$(pwd)/build/deviceattach-tmux"

cleanup() { "$T" -L $S kill-server 2>/dev/null || true; rm -f "$WRAP"; }
trap cleanup EXIT INT TERM
say() { printf '%s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

mkdir -p build
printf '#!/bin/sh\nexec "%s" -L %s "$@"\n' "$T" "$S" > "$WRAP"
chmod +x "$WRAP"

# Two windows of four panes, each with real scrollback, so "only the on-screen
# window is filled" is a claim with something to be wrong about.
"$T" -L $S kill-server 2>/dev/null || true
"$T" -L $S new-session -d -s work -x 300 -y 90
W0=$("$T" -L $S list-windows -t work -F '#{window_id}' | head -1)
for _ in 1 2 3; do "$T" -L $S split-window -t "$W0" -d; done
"$T" -L $S new-window -t work -d
W1=$("$T" -L $S list-windows -t work -F '#{window_id}' | tail -1)
for _ in 1 2 3; do "$T" -L $S split-window -t "$W1" -d; done
"$T" -L $S select-window -t "$W0"
for P in $("$T" -L $S list-panes -s -t work -F '#{pane_id}'); do
  "$T" -L $S send-keys -t "$P" "seq 1 3000 | sed 's/^/line /'" Enter
done
sleep 4
PANES=$("$T" -L $S list-panes -s -t work | wc -l | tr -d ' ')
W0_PANES=$("$T" -L $S list-panes -t "$W0" -F '#{pane_id}' | tr '\n' ' ')
W1_PANES=$("$T" -L $S list-panes -t "$W1" -F '#{pane_id}' | tr '\n' ' ')
say "probe: $PANES panes — on screen ($W0): $W0_PANES| off screen ($W1): $W1_PANES"

# The tap is APPEND-ONLY inside the app sandbox, so a previous run's evidence is
# still in there. Note how long it is now and analyse only what this run adds —
# without this, a second run inherits the first one's conclusions and every
# assertion below passes for the wrong reason (it did: a re-run "proved" that
# attach fills every pane, using lines written by the run before it).
pull_raw() {
  rm -f "$OUT/prev.log"
  xcrun devicectl device copy from --device "$DEV" \
    --domain-type appDataContainer --domain-identifier "$BUNDLE" \
    --source Documents/cc-pairing.log --destination "$OUT/prev.log" >/dev/null 2>&1 || true
  wc -l < "$OUT/prev.log" 2>/dev/null | tr -d ' ' || echo 0
}
BASE=$(pull_raw); BASE=${BASE:-0}
say "tap already holds $BASE lines from earlier runs — ignoring them"

KEY=$(base64 -i "$HOME/.ssh/id_ed25519" | tr -d '\n')
# A fixed id so the deep link below can name this connection. The app picks a
# random one otherwise.
CONN=3F2504E0-4F89-11D3-9A0C-0305E82C3301
xcrun devicectl device process launch --device "$DEV" --terminate-existing "$BUNDLE" \
  -MOSHPIT_CC_TAP documents -MOSHPIT_AUTOCARE_OFF 1 -MOSHPIT_SEED_ID "$CONN" \
  -MOSHPIT_SEED_USER "$(whoami)" -MOSHPIT_SEED_KEY_B64 "$KEY" \
  -MOSHPIT_SEED_HOST "$HOST" -MOSHPIT_SEED_PORT 22 \
  -MOSHPIT_SEED_TMUX 1 -MOSHPIT_SEED_TMUX_BIN "$WRAP" >/dev/null 2>&1

cc() { "$T" -L $S list-clients -F '#{client_name} #{client_control_mode}' 2>/dev/null \
  | awk '$2==1 {print $1; exit}'; }
printf 'waiting for the app to attach'
CC=""
for _ in $(seq 1 25); do CC=$(cc); [ -n "$CC" ] && break; printf '.'; sleep 2; done
printf '\n'
[ -z "$CC" ] && fail "never attached — no control-mode client, so nothing below would mean anything"
say "attached on $CC"
sleep 10

pull() {
  rm -f "$OUT/raw.log" "$OUT/cc-pairing.log"
  xcrun devicectl device copy from --device "$DEV" \
    --domain-type appDataContainer --domain-identifier "$BUNDLE" \
    --source Documents/cc-pairing.log --destination "$OUT/raw.log" >/dev/null 2>&1
  # THIS run only.
  tail -n +$((BASE + 1)) "$OUT/raw.log" > "$OUT/cc-pairing.log" 2>/dev/null || true
}
dumped() {  # panes given a scrollback dump so far
  grep -oE 'capture-pane -p -e -S -[0-9]+ -E -1 -t %[0-9]+' "$OUT/cc-pairing.log" 2>/dev/null \
    | sed -E 's/.*-t //' | sort -u | tr '\n' ' '
}

pull
[ -s "$OUT/cc-pairing.log" ] || fail "no tap file came back from the device"

say ""
say "== attach =="
PROBES=$(grep -c 'alternate_on' "$OUT/cc-pairing.log" || true)
DEPTH=$(grep -oE 'capture-pane -p -e -S -[0-9]+' "$OUT/cc-pairing.log" | sort -u | head -1)
FIRST="$(dumped)"
say "   per-pane alternate_on probes : $PROBES"
say "   history depth                : ${DEPTH:-none}"
say "   panes filled                 : $FIRST"

[ "$PROBES" = "0" ] || fail "the per-pane probe is back — that is a whole round-trip depth"
echo "$DEPTH" | grep -q -- "-S -400" || fail "unexpected history depth: $DEPTH"
for P in $W1_PANES; do
  case " $FIRST " in *" $P "*) fail "off-screen pane $P was filled on attach" ;; esac
done
for P in $W0_PANES; do
  case " $FIRST " in *" $P "*) ;; *) fail "on-screen pane $P was NOT filled" ;; esac
done
say "   ok  only the on-screen window paid for a dump"

# The other half of the promise: deferred, not dropped.
#
# The switch has to come from the APP, and that is not a quibble. Driving
# `tmux select-window` on the host does nothing here BY DESIGN: `parseListWindows`
# adopts tmux's active window only "when OUR choice is gone or belongs to another
# session", so a phone keeps showing the window its user chose rather than
# following whatever a desktop client just did. The first version of this script
# switched from the host, watched the off-screen panes stay empty, and reported a
# bug that was not one — the app was right and the test was wrong.
say ""
say "== switch to the off-screen window, from the app =="
TARGET=$(printf '%s' "$W1_PANES" | awk '{print $1}')
say "   deep link → pane $TARGET (in $W1)"
xcrun devicectl device process launch --device "$DEV" \
  --payload-url "moshpit://connection/$CONN?pane=$TARGET" "$BUNDLE" >/dev/null 2>&1
sleep 14
pull
AFTER="$(dumped)"
say "   panes filled now             : $AFTER"
MISSING=""
for P in $W1_PANES; do
  case " $AFTER " in *" $P "*) ;; *) MISSING="$MISSING $P" ;; esac
done
[ -n "$MISSING" ] && fail "deferred became dropped — never filled:$MISSING"
say "   ok  the deferred window filled when it came on screen"

say ""
say "PASS — deferred, not dropped, on real hardware."
