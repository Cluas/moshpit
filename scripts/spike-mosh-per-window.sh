#!/usr/bin/env bash
# Route-B spike — one REAL mosh connection per tmux window (grouped sessions).
#
# Question under test: can "a native tab per tmux window over mosh" stand on
#   mosh-server new -i 127.0.0.1 -- tmux new-session -t base \; select-window -t :N
# i.e. one mosh-server + one grouped tmux session per window, driven by the
# app's own SSP client (MoshTransport) — no GPL mosh-client anywhere?
#
# What it proves when it passes:
#   * two MoshTransport actors run concurrently in one process (no shared
#     state fights — the app could open one per visible window);
#   * each transport's screen is ITS window: typed input round-trips into the
#     right window, and the sibling's marker never bleeds across (grouped
#     sessions keep independent current windows — the load-bearing tmux fact);
#   * bootstrap is N cheap execs over the existing SSH lane, exactly the
#     MoshBootstrap shape.
#
# Isolation: a private tmux socket (-L), same trick as verify-e2e-mosh-tmux.sh
# — this never touches real tmux sessions. Cleanup kills the whole socket.
#
# Prereqs: Remote Login on (ssh 127.0.0.1 must work with keys); tmux and
# mosh-server on PATH (Homebrew); Xcode command-line tools.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCK="moshspikeb"
TMUX_BIN="$(command -v tmux)"
MOSH_SERVER="$(command -v mosh-server)"
BIN="$REPO_ROOT/build/spike-mosh-per-window"
mkdir -p "$REPO_ROOT/build"

cleanup() {
  "$TMUX_BIN" -L "$SOCK" kill-server 2>/dev/null || true
  pkill -f "mosh-server new.*$SOCK" 2>/dev/null || true
}
trap cleanup EXIT

echo "== 1/4 compile the app's SSP client into a CLI =="
swiftc -O -parse-as-library -o "$BIN" \
  "$REPO_ROOT/scripts/spike_mosh_per_window.swift" \
  "$REPO_ROOT/scripts/spike_mosh_shims.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshTransport.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshWire.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshCrypto.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/MoshCompression.swift" \
  "$REPO_ROOT/Moshpit/Services/Mosh/OCB3.swift" \
  -framework Network

echo "== 2/4 base session: two interactive-shell windows on a private socket =="
cleanup
# -f /dev/null: the private socket would still read ~/.tmux.conf, and a
# user config's base-index/status-line makes window addressing and screen
# assertions nondeterministic (first run failed exactly that way: base-index 1
# made `select-window -t :0` a no-op and both clients landed on one window).
# Window IDs (@N) are captured at creation and used for targeting — they are
# config-independent.
TMUX="$TMUX_BIN -f /dev/null -L $SOCK"
WID_A="$($TMUX new-session -d -s base -x 100 -y 30 -P -F '#{window_id}' sh)"
$TMUX set -g aggressive-resize on
WID_B="$($TMUX new-window -t base -P -F '#{window_id}' sh)"
echo "   window ids: A=$WID_A B=$WID_B"

echo "== 3/4 one mosh-server per window, bootstrapped over ssh (the MoshBootstrap shape) =="
boot() { # $1 = window id
  ssh 127.0.0.1 "$MOSH_SERVER new -i 127.0.0.1 -c 256 -l LANG=en_US.UTF-8 -- $TMUX_BIN -f /dev/null -L $SOCK new-session -t base \\; select-window -t '$1'" \
    2>/dev/null | awk '/^MOSH CONNECT/{print $3, $4}'
}
read -r PORT_A KEY_A <<<"$(boot "$WID_A")"
read -r PORT_B KEY_B <<<"$(boot "$WID_B")"
[ -n "${PORT_A:-}" ] && [ -n "${PORT_B:-}" ] || { echo "FATAL: no MOSH CONNECT line"; exit 1; }
echo "   window 0 → udp:$PORT_A    window 1 → udp:$PORT_B"

echo "== 4/4 two concurrent MoshTransports, marker round-trip each =="
"$BIN" 127.0.0.1 "$PORT_A" "$KEY_A" "$PORT_B" "$KEY_B"
