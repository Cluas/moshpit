#!/bin/sh
# Does a reconnect re-announce prompts the user was already told about — on a
# REAL device?
#
# The fix (announce once per EPISODE, keyed on @moshpit_since, persisted) was
# proven on the simulator; this run proves it on hardware, where the app, the
# network stack, and the reconnect path are all the shipping ones. The evidence
# is the app's own decision log — `postAttention` logs "announcing attention"
# BEFORE the notification-authorization gate, precisely so "we decided to
# announce" is observable apart from "iOS delivered something". idevicesyslog
# needs USB.
#
# Ends with a POSITIVE control: a genuinely new question after the reconnect
# must announce exactly once. Without it, "0 announcements" is indistinguishable
# from a dead probe — every vacuous zero this project has produced came from
# skipping that step.
#
# Usage: scripts/verify-device-reconnect.sh <devicectl-udid> <usb-udid> <host-ip>
set -eu
DEV=${1:?devicectl udid}
USB=${2:?usbmux udid (idevice_id -l)}
HOST=${3:?address of this mac the device can reach}
BUNDLE=com.cluas.moshpit
T=$(command -v tmux)
S=devicereconnect
# Stable path, kept after the run: the first version used mktemp and the one
# time a phase failed, the evidence needed for the diagnosis had no findable
# address.
OUT="/tmp/device-reconnect-evidence"
rm -rf "$OUT"; mkdir -p "$OUT"
WRAP="$(pwd)/build/devicereconnect-tmux"
LOG="$OUT/syslog.txt"
CAP=""

cleanup() {
  [ -n "$CAP" ] && kill "$CAP" 2>/dev/null || true
  "$T" -L $S kill-server 2>/dev/null || true
  rm -f "$WRAP"
}
trap cleanup EXIT INT TERM
say() { printf '%s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }
announced() {
  # A count read off a dead capture is frozen, and a frozen count makes phase 2
  # pass vacuously ("no new announcements") while measuring nothing. Refuse to
  # read one: idevicesyslog has quit spontaneously on this machine before.
  kill -0 "$CAP" 2>/dev/null || fail "the syslog capture died — counts from here would be frozen, not zero"
  grep -c "announcing attention" "$LOG" 2>/dev/null || true
}
cc() { "$T" -L $S list-clients -F '#{client_name} #{client_control_mode}' 2>/dev/null \
  | awk '$2==1 {print $1; exit}'; }
# The client's IDENTITY, not its name. pty names are reused and a LAN reconnect
# completes in under a second, so polling for "the name went away" misses the
# gap entirely and then reads the survivor as proof the kill failed (it did,
# in this script's second run). A new attach is a new tmux client process.
ccpid() { "$T" -L $S list-clients -F '#{client_pid} #{client_control_mode}' 2>/dev/null \
  | awk '$2==1 {print $1; exit}'; }

mkdir -p build
printf '#!/bin/sh\nexec "%s" -L %s "$@"\n' "$T" "$S" > "$WRAP"
chmod +x "$WRAP"

# Four panes waiting on a question. Fresh `since` stamps every run: the app
# PERSISTS which episodes it has announced (that persistence is half the fix),
# so re-running with yesterday's timestamps would measure the record, not the
# logic.
"$T" -L $S kill-server 2>/dev/null || true
# Panes run `cat`, not a shell: the monitor now treats a hook-stamped pane whose
# FOREGROUND is a shell as a dead agent and heals its state to done — which is
# exactly right in production and would silently hollow out this fixture.
"$T" -L $S new-session -d -s agents -x 300 -y 90 cat
for _ in 1 2 3; do "$T" -L $S split-window -t agents -d cat; done
"$T" -L $S select-layout -t agents tiled >/dev/null
NOW=$(date +%s); i=0
for P in $("$T" -L $S list-panes -t agents -F '#{pane_id}'); do
  i=$((i+1))
  "$T" -L $S set -p -t "$P" @moshpit_agent claude
  "$T" -L $S set -p -t "$P" @moshpit_since "$((NOW - i))"
  "$T" -L $S set -p -t "$P" @moshpit_title "device-reconnect q$i"
  "$T" -L $S set -p -t "$P" @moshpit_state attention
done
STANDING=4

# Two DECOYS the phone must reclassify and never announce — the fossils a real
# phone showed frozen for a day. One is an idle reminder recorded as attention
# by an older stamp (agent alive, `cat` foreground, the telltale title); the
# other is a dead agent (a SHELL in the foreground under an attention stamp).
# Both are absence-based checks, which is why they share a run with four panes
# that MUST announce: the positives prove the channel the absences rely on.
"$T" -L $S new-window -t agents -d cat
FOSSIL=$("$T" -L $S list-panes -s -t agents -F '#{pane_id}' | tail -1)
"$T" -L $S set -p -t "$FOSSIL" @moshpit_agent claude
"$T" -L $S set -p -t "$FOSSIL" @moshpit_since "$((NOW - 3600))"
"$T" -L $S set -p -t "$FOSSIL" @moshpit_title "Claude is waiting for your input"
"$T" -L $S set -p -t "$FOSSIL" @moshpit_state attention
"$T" -L $S new-window -t agents -d
DEAD=$("$T" -L $S list-panes -s -t agents -F '#{pane_id}' | tail -1)
"$T" -L $S set -p -t "$DEAD" @moshpit_agent claude
"$T" -L $S set -p -t "$DEAD" @moshpit_since "$((NOW - 60))"
"$T" -L $S set -p -t "$DEAD" @moshpit_title "Bash: rm -rf build"
"$T" -L $S set -p -t "$DEAD" @moshpit_state attention
say "probe: $STANDING real prompts, plus decoys fossil=$FOSSIL (idle title) dead=$DEAD (shell foreground)"

# One continuous capture; counts are snapshotted at phase boundaries.
idevicesyslog -u "$USB" > "$LOG" 2>&1 &
CAP=$!
sleep 3
# Alive is not enough — idevicesyslog can sit running while producing NOTHING
# (device off the USB bus, another instance holding usbmux). kill -0 passes in
# that state and every count downstream is a vacuous zero, which is exactly how
# one run "found" a regression that was a loose cable. Demand actual output.
[ "$(wc -c < "$LOG" | tr -d ' ')" -gt 200 ] || fail "syslog capture produced no data — is the device actually on USB? (idevice_id -l must list $USB)"

KEY=$(base64 -i "$HOME/.ssh/id_ed25519" | tr -d '\n')
# Grace shortened to 1s: this harness studies the announce-once logic, not the
# 30-second answered-at-the-desk window (scripts/verify-stamp-quiet.sh covers
# the window itself, host-side).
xcrun devicectl device process launch --device "$DEV" --terminate-existing "$BUNDLE" \
  -MOSHPIT_ANNOUNCE_GRACE 1 \
  -MOSHPIT_AUTOCARE_OFF 1 -MOSHPIT_SEED_ID 3F2504E0-4F89-11D3-9A0C-0305E82C3301 \
  -MOSHPIT_SEED_USER "$(whoami)" -MOSHPIT_SEED_KEY_B64 "$KEY" \
  -MOSHPIT_SEED_HOST "$HOST" -MOSHPIT_SEED_PORT 22 \
  -MOSHPIT_SEED_TMUX 1 -MOSHPIT_SEED_TMUX_BIN "$WRAP" >/dev/null 2>&1

printf 'waiting for the app to attach'
C=""
for _ in $(seq 1 25); do C=$(cc); [ -n "$C" ] && break; printf '.'; sleep 2; done
printf '\n'
[ -z "$C" ] && fail "never attached"
say "attached on $C"

say ""
say "== phase 1: first sighting (hook poll runs every 2s) =="
sleep 18
P1=$(announced)
say "   announcements: $P1   (expected: $STANDING, one per standing prompt)"
# idevicesyslog is a LOSSY channel: one observed run scheduled all four
# announcements (4× "announce scheduled", 0× cancelled) and delivered three
# "announcing" lines; the clean rerun delivered all four. An under-count here
# with a full set of schedule lines is the channel, not the app — rerun before
# believing it.
[ "$P1" -ge "$STANDING" ] || fail "first sighting under-announced ($P1 < $STANDING) — probe dead, hooks not polled, or syslog dropped lines (rerun)"
[ "$P1" -le "$STANDING" ] || fail "first sighting OVER-announced ($P1 > $STANDING) — a decoy rang: fossil or dead-agent reclassification failed"
grep -q "announcing attention: pane $FOSSIL" "$LOG" && fail "the idle-title fossil announced"
grep -q "announcing attention: pane $DEAD" "$LOG" && fail "the dead-agent pane announced"
say "   ok  both decoys stayed silent"

say ""
say "== phase 2: kill the transport (the real reconnect) =="
TTY=$(basename "$C")
PID=$(ps -ef | awk -v t="$TTY" '$0 ~ ("sshd-session: .*@" t) {print $2; exit}')
[ -z "$PID" ] && fail "no sshd session found for $TTY"
say "   killing sshd session $PID (serving $TTY)"
kill "$PID"
OLDPID=$(ccpid)
[ -z "$OLDPID" ] && fail "no client pid before the kill"
say "   old client pid: $OLDPID"
printf '   waiting for a NEW client process'
NEWPID=""
for _ in $(seq 1 30); do
  NEWPID=$(ccpid)
  [ -n "$NEWPID" ] && [ "$NEWPID" != "$OLDPID" ] && break
  NEWPID=""
  printf '.'; sleep 2
done
printf '\n'
[ -z "$NEWPID" ] && fail "no new client appeared — either the kill did not drop the transport or the app never reconnected"
say "   reconnected: client pid $OLDPID -> $NEWPID"
sleep 18
P2=$(announced)
say "   announcements now: $P2   (delta: $((P2 - P1)) — nothing on the host changed)"
[ "$P2" -eq "$P1" ] || fail "reconnect re-announced $((P2 - P1)) unchanged prompt(s)"
say "   ok  the reconnect announced nothing"

say ""
say "== phase 3: positive control — a genuinely NEW question must ring =="
FIRST=$("$T" -L $S list-panes -t agents -F '#{pane_id}' | head -1)
"$T" -L $S set -p -t "$FIRST" @moshpit_state working
sleep 5
"$T" -L $S set -p -t "$FIRST" @moshpit_since "$(date +%s)"
"$T" -L $S set -p -t "$FIRST" @moshpit_title "post-reconnect NEW question"
"$T" -L $S set -p -t "$FIRST" @moshpit_state attention
sleep 14
P3=$(announced)
say "   announcements now: $P3   (delta: $((P3 - P2)))"
[ "$P3" -eq "$((P2 + 1))" ] || fail "a new question after the reconnect announced $((P3 - P2)) times, want exactly 1"
say "   ok  new question rang once — the probe was live the whole time"

say ""
say "== phase 4: looking at the prompt acknowledges it =="
"$T" -L $S set -p -t "$FIRST" @moshpit_title "ack me"
xcrun devicectl device process launch --device "$DEV" \
  --payload-url "moshpit://connection/3F2504E0-4F89-11D3-9A0C-0305E82C3301?pane=$FIRST" "$BUNDLE" >/dev/null 2>&1
sleep 12
grep -q "acknowledged: pane $FIRST" "$LOG" \
  || fail "viewing the waiting pane did not acknowledge it"
say "   ok  viewed -> standing cleared"

say ""
say "PASS — on hardware: decoys silent, first sighting $P1, reconnect +0, new question +1, viewed prompt acknowledged."
