#!/usr/bin/env bash
# Deterministic reproduction rig for the "white cursor-sized blocks" bug.
#
# Field report: the FIRST pane view after connecting over mosh is clean; the
# blocks appear only AFTER SWITCHING panes. The app's switch is a
# `select-pane -Z` issued out-of-band on a separate SSH -CC control lane, so
# the mosh screen just receives tmux's zoom-relayout repaint burst. No packet
# loss is involved — which means it must reproduce over loopback.
#
# What this captures, using the app's OWN SSP client (MoshTransport, the same
# source the app ships — compiled here into a CLI exactly like
# spike-mosh-per-window.sh does):
#
#   phaseN.bin   every host byte mosh delivered during phase N
#   truthN.txt   `tmux capture-pane -p -e` of the pane visible during phase N
#                — tmux's OWN cell model, the ground truth SwiftTerm must match
#   meta.json    grid, phase→pane map, file list
#
# MoshpitTests/Services/MoshSwitchReplayTests.swift replays the .bin files
# into a headless SwiftTerm `Terminal` and diffs every cell against the
# truth. Divergence = the bug, and the mismatching row tells you which of the
# content generator's tagged cases seeded it.
#
# Isolation: a private tmux socket (-L) with `-f /dev/null`, same trick as
# spike-mosh-per-window.sh — never touches real tmux sessions.
#
# Prereqs: Remote Login on (ssh 127.0.0.1 works with keys); tmux, mosh-server
# and python3 on PATH; Xcode command-line tools.
#
# Env knobs:  COLS/ROWS (default 90x40, the user-report shape), OUT, KEEP=1
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCK="moshswitch"
COLS="${COLS:-90}"
ROWS="${ROWS:-40}"
OUT="${OUT:-$REPO_ROOT/build/mosh-switch-capture-${COLS}x${ROWS}}"
TMUX_BIN="$(command -v tmux)"
MOSH_SERVER="$(command -v mosh-server)"
BIN="$REPO_ROOT/build/capture-mosh-switch-bytes"
TMUX="$TMUX_BIN -f /dev/null -L $SOCK"

kill_server() {
  $TMUX kill-server 2>/dev/null || true
  pkill -f "mosh-server new.*$SOCK" 2>/dev/null || true
}
# KEEP=1 leaves the socket up for poking at by hand — but the NEXT run must
# still start from a clean server, so only the exit trap honours it.
cleanup() { [ "${KEEP:-0}" = "1" ] || kill_server; }
trap cleanup EXIT

rm -rf "$OUT"; mkdir -p "$OUT" "$REPO_ROOT/build"

echo "== 1/6 compile the app's SSP client into a capture CLI =="
swiftc -O -parse-as-library -o "$BIN" \
  "$REPO_ROOT/scripts/capture_mosh_switch_bytes.swift" \
  "$REPO_ROOT/scripts/spike_mosh_shims.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshTransport.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshWire.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshCrypto.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshCompression.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/OCB3.swift" \
  -framework Network

echo "== 2/6 generate the two panes' content (${COLS} cols) =="
python3 "$REPO_ROOT/scripts/gen_mosh_switch_content.py" \
  --cols "$COLS" --rows "$ROWS" --pane1 "$OUT/pane1.ansi" --pane2 "$OUT/pane2.ansi"

echo "== 3/6 isolated tmux: one window, two panes, pane 1 zoomed =="
kill_server
# `cat FILE; exec cat` — paint once, then park on a reader that never
# repaints, so tmux's model of the pane is static and capture-pane is a
# stable ground truth. `-f /dev/null`: a user config's base-index/status-line
# would make window addressing and the grid geometry nondeterministic.
P1="$($TMUX new-session -d -s base -x "$COLS" -y "$ROWS" -P -F '#{pane_id}' \
      "sh -c 'cat $OUT/pane1.ansi; exec cat'")"
# Status bar off: the app hides it too (mosh already renders tmux's TUI), and
# it makes the client grid map 1:1 onto the zoomed pane — which is what lets
# the replay test compare the mosh screen against capture-pane cell for cell.
$TMUX set -g status off
$TMUX set -g aggressive-resize on
P2="$($TMUX split-window -t "$P1" -P -F '#{pane_id}' \
      "sh -c 'cat $OUT/pane2.ansi; exec cat'")"
# `resize-pane -Z` is what ZOOMS (the app's ensureImmersiveZoom); the later
# `select-pane -Z` only KEEPS an existing zoom while changing the active pane
# — with no zoom to keep it is a no-op, and the first run of this rig captured
# a split screen with a 300-byte pane-border recolour instead of a repaint.
$TMUX resize-pane -Z -t "$P1"
GEOM="$($TMUX display-message -p -t "$P1" '#{pane_width}x#{pane_height} zoomed=#{window_zoomed_flag}')"
echo "   panes: 1=$P1  2=$P2   active pane: $GEOM"
[ "$GEOM" = "${COLS}x${ROWS} zoomed=1" ] || {
  echo "FATAL: zoomed pane is not the whole client grid ($GEOM != ${COLS}x${ROWS} zoomed=1);"
  echo "       the mosh screen would not map 1:1 onto capture-pane."
  exit 1
}

echo "== 4/6 mosh-server bootstrapped over ssh (the MoshBootstrap shape) =="
read -r PORT KEY <<<"$(ssh 127.0.0.1 \
  "$MOSH_SERVER new -i 127.0.0.1 -c 256 -l LANG=en_US.UTF-8 -- $TMUX_BIN -f /dev/null -L $SOCK attach -t base" \
  2>/dev/null | awk '/^MOSH CONNECT/{print $3, $4}')"
[ -n "${PORT:-}" ] || { echo "FATAL: no MOSH CONNECT line"; exit 1; }
echo "   udp:$PORT"

echo "== 5/6 switch scripts (what the app's -CC sidecar does) =="
# Script N runs at the exact moment phase N-1 has gone quiet, so it is also
# the right moment to snapshot the ground truth for that phase.
mkswitch() { # $1 = index, $2 = pane visible during the phase that just ended, $3 = pane to switch to
  cat >"$OUT/switch$1.sh" <<EOSW
#!/bin/sh
$TMUX capture-pane -p -e -t $2 > "$OUT/truth$(( $1 - 1 )).txt"
$TMUX display-message -p -t $2 '#{pane_width}x#{pane_height} zoomed=#{window_zoomed_flag}' > "$OUT/geom$(( $1 - 1 )).txt"
$TMUX select-pane -Z -t $3
sleep 0.4
EOSW
}
mkswitch 1 "$P1" "$P2"
mkswitch 2 "$P2" "$P1"
mkswitch 3 "$P1" "$P2"

echo "== 6/6 capture =="
"$BIN" 127.0.0.1 "$PORT" "$KEY" "$COLS" "$ROWS" "$OUT" \
  "$OUT/switch1.sh" "$OUT/switch2.sh" "$OUT/switch3.sh" | tee "$OUT/capture.log"

# Nothing runs in the panes (they are parked on `cat`), so tmux's model is
# unchanged by the client going away — the final truth is safe to take now.
$TMUX capture-pane -p -e -t "$P2" > "$OUT/truth3.txt"
$TMUX display-message -p -t "$P2" '#{pane_width}x#{pane_height} zoomed=#{window_zoomed_flag}' > "$OUT/geom3.txt"

echo "== 7/7 reference paint: a SECOND, FRESH mosh-server on the same session =="
# The attribute-accurate reference the other two can't be:
#   * `capture-pane` trims trailing SPACES from each line regardless of their
#     attributes, so an inverse-video space parked at a line end — the white
#     block itself — simply is not in truthN.txt;
#   * MoshTransport.requestFullRedraw() (ack display state 0) does NOT make a
#     stock mosh-server repaint: transportsender.cc culls sent states older
#     than the last ack, and an ack naming a culled state is IGNORED. See
#     redraw.bin, which comes back empty.
# A brand-new mosh-server attaching to the same (static) tmux session paints
# its whole framebuffer from state 0 because that state genuinely is fresh.
# Fed into a blank terminal, that is mosh's model of this exact screen.
read -r RPORT RKEY <<<"$(ssh 127.0.0.1 \
  "$MOSH_SERVER new -i 127.0.0.1 -c 256 -l LANG=en_US.UTF-8 -- $TMUX_BIN -f /dev/null -L $SOCK attach -t base" \
  2>/dev/null | awk '/^MOSH CONNECT/{print $3, $4}')"
[ -n "${RPORT:-}" ] || { echo "FATAL: no MOSH CONNECT line for the reference paint"; exit 1; }
mkdir -p "$OUT/ref"
"$BIN" 127.0.0.1 "$RPORT" "$RKEY" "$COLS" "$ROWS" "$OUT/ref" | sed 's/^/   ref /'
cp "$OUT/ref/phase0.bin" "$OUT/reference.bin"

cat >"$OUT/meta.json" <<EOM
{
  "cols": $COLS,
  "rows": $ROWS,
  "redraw": "redraw.bin",
  "reference": "reference.bin",
  "phases": [
    { "index": 0, "bytes": "phase0.bin", "truth": "truth0.txt", "pane": "$P1", "what": "initial sync (report says CLEAN)" },
    { "index": 1, "bytes": "phase1.bin", "truth": "truth1.txt", "pane": "$P2", "what": "select-pane -Z to pane 2" },
    { "index": 2, "bytes": "phase2.bin", "truth": "truth2.txt", "pane": "$P1", "what": "select-pane -Z back to pane 1" },
    { "index": 3, "bytes": "phase3.bin", "truth": "truth3.txt", "pane": "$P2", "what": "select-pane -Z to pane 2 again" }
  ]
}
EOM

echo
echo "artifacts in $OUT:"
ls -l "$OUT" | sed 's/^/   /'
