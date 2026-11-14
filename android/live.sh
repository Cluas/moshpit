#!/usr/bin/env bash
# Live sessions from the headless emulator to a PRIVATE sshd on this Mac: the
# app's whole path — the Rust core's SSH, tmux and mosh over the emulator's
# network, the byte channel through JNI, Termux's view, the Keystore's keys.
#
# The bench (all under one temporary directory, all stopped and removed when
# the script ends, however it ends):
#   - sshd: /usr/sbin/sshd -f <dir>/sshd_config on 127.0.0.1:$PORT (a high
#     port), its own host key, key auth only, `SetEnv TMUX_TMPDIR=<dir>/tmux`
#     so every tmux the sessions start is a private server in the temp dir —
#     never ~/.ssh, never the system sshd, never the user's tmux;
#   - the emulator reaches it at 10.0.2.2:$PORT (the host's loopback);
#   - mosh-server is the user's binary, started by the sessions through that
#     sshd; only the pids the sessions report are killed.
#
#   - the login shell's HOME is a fresh directory in the bench (sshd's
#     SetEnv, with a plain .zshrc): the installer writes there, never in the
#     user's ~/.moshpit or ~/.claude;
#   - herdr: a private server, `herdr --session mw12b-<pid>` under that HOME
#     (its socket is the bench's; the user's herdr is never addressed),
#     stopped and removed with the rest.
#
# Checks: a Keystore hardware key and an imported key sign in; echo of typed
# keys (and their latency), a burst's throughput, reconnect after the TCP leg
# is cut, the pill's switch to mosh and back, SSH + tmux on the private server
# and the breadcrumb's pickers, Set up this host's hooks on the bench's HOME,
# herdr on the Rust session, mosh, mosh's UDP silence.
#
#   android/live.sh        # needs android/build/moshpit-debug.apk
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APK="$HERE/build/moshpit-debug.apk"
RESULTS="$HERE/build/results/live-$(date +%Y%m%d-%H%M%S)"
PORT=${PORT:-42022}
# shellcheck source=emu.sh
. "$HERE/emu.sh"

[ -e "$APK" ] || { echo "build the APK first: android/build.sh" >&2; exit 1; }
emu_room
mkdir -p "$RESULTS"
SUMMARY="$RESULTS/summary.txt"

# --- The private sshd ---------------------------------------------------------------
# Under /tmp, not the scratch directory: tmux's socket path is
# $TMUX_TMPDIR/tmux-<uid>/default and a Unix socket's path has 104 bytes at
# most on macOS. Its real path (/private/tmp/…), as tmux reports it.
BENCH=$(cd "$(mktemp -d /tmp/mw12.XXXXXX)" && pwd -P)
chmod 700 "$BENCH"
mkdir -p "$BENCH/tmux" "$BENCH/home"
chmod 700 "$BENCH/tmux" "$BENCH/home"
BENCH_PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
printf "PROMPT='%%%% '\nunsetopt BEEP\n" >"$BENCH/home/.zshrc"
HERDR_BIN=$(command -v herdr || true)
HSESSION="mw12b-$$"
HERDR_PID=
# One private herdr command: the bench's HOME, its session.
H() {
    env -i HOME="$BENCH/home" ZDOTDIR="$BENCH/home" SHELL=/bin/zsh TERM=xterm-256color LANG=en_US.UTF-8 \
        PATH="$BENCH_PATH" "$HERDR_BIN" --session "$HSESSION" "$@"
}
REAL_CLAUDE_SUM=$(shasum "$HOME/.claude/settings.json" 2>/dev/null | cut -d' ' -f1 || true)
MOSH_PIDS=()
cleanup() {
    emu_stop
    if [ -n "$HERDR_PID" ]; then
        H server stop >/dev/null 2>&1 || true
        kill "$HERDR_PID" 2>/dev/null || true
        pkill -f "herdr --session $HSESSION" 2>/dev/null || true
        echo "stopped the bench's herdr"
    fi
    if [ -e "$BENCH/sshd.pid" ]; then kill "$(cat "$BENCH/sshd.pid")" 2>/dev/null || true; fi
    TMUX_TMPDIR="$BENCH/tmux" tmux kill-server 2>/dev/null || true
    for pid in "${MOSH_PIDS[@]:-}"; do
        [ -n "$pid" ] && kill "$pid" 2>/dev/null && echo "stopped mosh-server $pid" || true
    done
    # The sshd's children (sessions' shells), by the bench's own config path.
    pkill -f "sshd -f $BENCH/sshd_config" 2>/dev/null || true
    rm -rf "$BENCH"
    echo "bench removed"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ssh-keygen -q -t ed25519 -N '' -f "$BENCH/host_key" -C moshpit-w12-host
ssh-keygen -q -t ed25519 -N '' -f "$BENCH/client_key" -C moshpit-w12-client
cp "$BENCH/client_key.pub" "$BENCH/authorized_keys"
chmod 600 "$BENCH/authorized_keys"
cat >"$BENCH/sshd_config" <<EOF
HostKey $BENCH/host_key
Port $PORT
ListenAddress 127.0.0.1
PidFile $BENCH/sshd.pid
AuthorizedKeysFile $BENCH/authorized_keys
StrictModes no
UsePAM no
UseDNS no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AcceptEnv LANG LC_*
SetEnv TMUX_TMPDIR=$BENCH/tmux MOSH_SERVER_NETWORK_TMOUT=300 HOME=$BENCH/home ZDOTDIR=$BENCH/home PATH=$BENCH_PATH
LogLevel VERBOSE
EOF
/usr/sbin/sshd -f "$BENCH/sshd_config" -E "$BENCH/sshd.log"
for _ in $(seq 1 20); do [ -s "$BENCH/sshd.pid" ] && break; sleep 0.2; done
[ -s "$BENCH/sshd.pid" ] || { cat "$BENCH/sshd.log"; echo "sshd did not start" >&2; exit 1; }
SSH=(ssh -p "$PORT" -i "$BENCH/client_key" -o IdentitiesOnly=yes -o UserKnownHostsFile="$BENCH/known_hosts"
    -o StrictHostKeyChecking=accept-new -o BatchMode=yes 127.0.0.1)
# The sessions' tmux is the bench's: `tmux ls` through this sshd talks to a
# server under $BENCH/tmux, and there is none yet.
tmux_ls=$("${SSH[@]}" 'echo TMUX_TMPDIR=$TMUX_TMPDIR; tmux ls 2>&1' || true)
echo "$tmux_ls" >"$RESULTS/tmux-ls.txt"
if echo "$tmux_ls" | grep -q "TMUX_TMPDIR=$BENCH/tmux" && echo "$tmux_ls" | grep -q "$BENCH/tmux"; then
    verdict "PASS private sshd on 127.0.0.1:$PORT; tmux ls through it: $(echo "$tmux_ls" | tail -1)"
else
    verdict "FAIL the bench's tmux is not private: $tmux_ls"
    exit 1
fi
home_seen=$("${SSH[@]}" 'echo $HOME' || true)
[ "$home_seen" = "$BENCH/home" ] && verdict "PASS the login shell's HOME is the bench's ($home_seen)" ||
    { verdict "FAIL the login shell's HOME is not the bench's: $home_seen"; exit 1; }
# The bench's herdr: a private session under the bench's HOME, with one
# workspace.
if [ -n "$HERDR_BIN" ]; then
    case "$(H status 2>&1)" in
    *"$BENCH/home"*)
        H server >"$BENCH/herdr-server.log" 2>&1 </dev/null &
        HERDR_PID=$!
        for _ in $(seq 1 100); do H api snapshot >/dev/null 2>&1 && break; sleep 0.1; done
        H workspace create --focus --label live >/dev/null
        verdict "PASS private herdr: --session $HSESSION, its socket under $BENCH/home"
        ;;
    *) verdict "FAIL the bench's herdr session isn't private: $(H status 2>&1 | head -2)"; HERDR_BIN= ;;
    esac
fi
head -c 3000000 /dev/urandom | base64 >"$BENCH/burst.txt"
BURST_BYTES=$(wc -c <"$BENCH/burst.txt" | tr -d ' ')

# --- The app ---------------------------------------------------------------------------
trap 'cleanup' EXIT
emu_boot "$RESULTS/emulator.log"
app_launch "$APK" "$RESULTS/logcat.txt"
wait_log 'page [0-9]+: home' 60
USER_NAME=$(id -un)

# A Keystore hardware key: its public half authorizes it on the bench.
answer=$(hook '{"cmd":"generate-key","name":"pixel","hardware":true}')
HW_KEY=$(echo "$answer" | sed -nE 's/.*"id":"([^"]*)".*/\1/p')
echo "$answer" | sed -nE 's/.*"publicKey":"([^"]*)".*/\1/p' >>"$BENCH/authorized_keys"
# The Mac's client key, imported as pasted.
key=$(sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g' "$BENCH/client_key")
answer=$(hook "{\"cmd\":\"import-key\",\"name\":\"bench\",\"text\":\"$key\"}")
IMPORTED_KEY=$(echo "$answer" | sed -nE 's/.*"id":"([^"]*)".*/\1/p')
[ -n "$HW_KEY" ] && [ -n "$IMPORTED_KEY" ] || { verdict "FAIL keys: $answer"; exit 1; }

add() { hook "$1" | sed -nE 's/.*"id":"([^"]*)".*/\1/p'; }
SSH_PLAIN=$(add "{\"cmd\":\"add-connection\",\"name\":\"bench-ssh\",\"host\":\"10.0.2.2\",\"port\":$PORT,\"user\":\"$USER_NAME\",\"key\":\"$HW_KEY\",\"mux\":\"none\"}")
SSH_TMUX=$(add "{\"cmd\":\"add-connection\",\"name\":\"bench-tmux\",\"host\":\"10.0.2.2\",\"port\":$PORT,\"user\":\"$USER_NAME\",\"key\":\"$IMPORTED_KEY\",\"mux\":\"tmux\",\"tmuxPath\":\"$(command -v tmux)\"}")
MOSH=$(add "{\"cmd\":\"add-connection\",\"name\":\"bench-mosh\",\"host\":\"10.0.2.2\",\"port\":$PORT,\"user\":\"$USER_NAME\",\"key\":\"$IMPORTED_KEY\",\"mosh\":true,\"mux\":\"none\",\"moshServer\":\"$(command -v mosh-server)\"}")
HERDR=
[ -n "$HERDR_BIN" ] && HERDR=$(add "{\"cmd\":\"add-connection\",\"name\":\"bench-herdr\",\"host\":\"10.0.2.2\",\"port\":$PORT,\"user\":\"$USER_NAME\",\"key\":\"$IMPORTED_KEY\",\"mux\":\"herdr\",\"herdrPath\":\"$HERDR_BIN --session $HSESSION\"}")
echo "connections: ssh=$SSH_PLAIN tmux=$SSH_TMUX mosh=$MOSH herdr=$HERDR"
# The page on top (its name).
top_page() { hook '{"cmd":"stack"}' | python3 -I -c 'import json,sys; s=json.load(sys.stdin)["stack"]; print(s[-1][1] if s else "")'; }

state_of() { hook "{\"cmd\":\"state\",\"connection\":\"$1\"}"; }
# Waits until connection $1's state matches $2, $3 seconds at most; trusts the
# host key when asked (the dialog's OK, through the hooks).
wait_state() {
    for _ in $(seq 1 "$3"); do
        hook '{"cmd":"answer-all","ok":true}' >/dev/null
        state_of "$1" | grep -Eq "$2" && return 0
        sleep 1
    done
    state_of "$1" >&2
    return 1
}
# Waits for the shell's prompt on screen (typing before it is up races the
# shell's start: zsh drops what came before its line editor).
wait_prompt() { wait_screen '➜|❯|[$%#>] *$' 30 || true; sleep 1; }
# Types $1 at the terminal on screen, as keys (adb's input), then Enter. The
# device's shell gets it single-quoted, so $… reaches the host's shell as typed
# ($1 must not hold a single quote).
type_line() {
    adb shell "input text '$(printf '%s' "$1" | sed 's/ /%s/g')'"
    adb shell input keyevent KEYCODE_ENTER
}

# --- 1. SSH, a Keystore key ------------------------------------------------------------
hook "{\"cmd\":\"enter\",\"connection\":\"$SSH_PLAIN\"}" >/dev/null
if wait_state "$SSH_PLAIN" '"state":"connected"' 60; then
    t0=$(date +%s)
    verdict "PASS SSH with the Keystore hardware key: connected"
else
    verdict "FAIL SSH with the Keystore key: $(state_of "$SSH_PLAIN")"
fi
grep -q 'Accepted publickey.*ECDSA' "$BENCH/sshd.log" && verdict "PASS sshd accepted the ECDSA (Keystore) key" || verdict "FAIL sshd did not log the ECDSA key"
view_hook focus 'focus' >/dev/null
wait_prompt
type_line "uname -s"
if wait_screen '^Darwin' 15; then verdict "PASS typed 'uname -s' through the byte channel; the Mac answered Darwin"; else verdict "FAIL echo over SSH"; fi
shot 20-terminal-ssh
# A held key, as Android sends it: down events with repeat counts 0…9, then up
# (adb's input can't hold a key; G3's open item).
adb shell "input text 'echo held-0123456789'"
sleep 1
view_hook keys 'keys ' --ei code 67 --ei repeat 9 >/dev/null
sleep 1
type_line 'ok'
if wait_screen '^held-ok$' 15; then verdict "PASS a held backspace (10 repeats) deleted 10 characters in order"; else verdict "FAIL held backspace"; fi
# An input method's composition on the view's own InputConnection: only the
# commits reach the host.
adb shell "input text 'echo%sime-'"
sleep 1
view_hook ime 'IME (done|none)' >/dev/null
sleep 1
adb shell input keyevent KEYCODE_ENTER
if wait_screen '^ime-你好ma是' 15; then verdict "PASS IME composition: the host got only the commits (你好 ma 是)"; else verdict "FAIL IME composition"; fi
echo_line=$(view_hook echo 'echo n=' --ei count 60 --ei gap 120)
verdict "ECHO ssh (keystroke written -> echo drained, 60 keys): $echo_line"
# A burst: a file of known size, its throughput into the terminal.
view_hook probe-reset 'probe reset' >/dev/null
type_line "cat $BENCH/burst.txt; echo BURST-DONE"
if wait_screen '^BURST-DONE' 600; then
    probe=$(view_hook probe 'probe ')
    verdict "BURST ssh $BURST_BYTES B file: $probe"
else
    verdict "FAIL burst did not finish in 10 minutes"
fi
grep -E 'Janky|Total frames' <(adb shell dumpsys gfxinfo "$PACKAGE") | head -2 | sed 's/^/  gfx /' | tee -a "$SUMMARY" || true
type_line "clear"
# Reconnect after the TCP leg is cut: from the core's own log lines (state
# changes), as logcat stamps them.
mark=$(lines_now)
hook "{\"cmd\":\"fault\",\"connection\":\"$SSH_PLAIN\",\"kind\":\"cut-tcp\"}" >/dev/null
if wait_log 'session [0-9]+: Reconnecting' 15 "$mark" && wait_log 'session [0-9]+: Connected' 60 "$mark"; then
    times=$(tail -n +"$((mark + 1))" "$LOG" | grep -E 'session [0-9]+: (Reconnecting|Connected)' | head -2 | awk '{ print $2 }')
    took=$(echo "$times" | python3 -I -c '
import sys
t = [l.strip() for l in sys.stdin if l.strip()]
s = lambda v: sum(float(x) * m for x, m in zip(v.split(":"), (3600, 60, 1)))
print(round((s(t[1]) - s(t[0])) * 1000) if len(t) == 2 else "?")')
    verdict "PASS reconnected after the TCP leg was cut: ${took} ms from Reconnecting to Connected"
    sleep 2
    type_line 'echo again-$((6*7))'
    wait_screen '^again-42' 15 && verdict "PASS typing after the reconnect reaches the host" || verdict "FAIL typing after the reconnect"
else
    verdict "FAIL no reconnect after the cut: $(state_of "$SSH_PLAIN")"
fi
# The pill's switch (the page confirms on its own sheet, then says
# switch-transport): to mosh, a new session over it, and back to SSH.
hook '{"cmd":"tell","event":{"ev":"switch-transport"}}' >/dev/null
if wait_state "$SSH_PLAIN" '"moshServerPid":[0-9]+.*"protocol":"mosh"' 60 && wait_state "$SSH_PLAIN" '"state":"connected"' 30; then
    pid=$(state_of "$SSH_PLAIN" | sed -nE 's/.*"moshServerPid":([0-9]+).*/\1/p')
    [ -n "$pid" ] && MOSH_PIDS+=("$pid")
    wait_prompt
    type_line 'echo switched-$((9*9))'
    wait_screen '^switched-81' 20 && verdict "PASS the pill switched SSH to mosh (mosh-server $pid) and typing reaches the host" ||
        verdict "FAIL typing after the switch to mosh"
    shot 20b-terminal-switched-to-mosh
    hook '{"cmd":"tell","event":{"ev":"switch-transport"}}' >/dev/null
    if wait_state "$SSH_PLAIN" '"protocol":"ssh"' 30 && wait_state "$SSH_PLAIN" '"moshServerPid":null.*"protocol":"ssh".*"state":"connected"' 60; then
        sleep 2
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            verdict "FAIL back on SSH, but mosh-server $pid still runs"
        else
            verdict "PASS the pill switched back to SSH; the mosh session's server went with it"
        fi
    else
        verdict "FAIL switching back to SSH: $(state_of "$SSH_PLAIN")"
    fi
else
    verdict "FAIL the pill's switch to mosh: $(state_of "$SSH_PLAIN")"
fi
hook '{"cmd":"back"}' >/dev/null

# --- 2. SSH + tmux on the private server -----------------------------------------------
# The bench's tmux has no session yet: the screen's empty state, and its
# Create (the one way a session gets made), then the attach.
hook "{\"cmd\":\"enter\",\"connection\":\"$SSH_TMUX\"}" >/dev/null
if wait_state "$SSH_TMUX" '"tmuxAttached":false' 60; then
    sleep 3
    shot 21-terminal-tmux-empty
    hook '{"cmd":"tell","event":{"ev":"create"}}' >/dev/null
fi
if wait_state "$SSH_TMUX" '"tmuxAttached":true' 60; then
    verdict "PASS SSH + tmux: the empty state's Create made a session and tmux -CC attached"
else
    verdict "FAIL SSH + tmux: $(state_of "$SSH_TMUX")"
fi
view_hook focus 'focus' >/dev/null
wait_prompt
type_line 'echo $TMUX | grep -o "mw12[^/]*"'
if wait_screen "$(basename "$BENCH" | sed 's/\./\\./')" 20; then verdict "PASS the pane's tmux is the bench's private server (\$TMUX under $BENCH/tmux)"; else verdict "FAIL tmux pane echo"; fi
shot 21-terminal-tmux-attached
echo_line=$(view_hook echo 'echo n=' --ei count 40 --ei gap 150)
verdict "ECHO ssh+tmux (40 keys): $echo_line"
TMUX_TMPDIR="$BENCH/tmux" tmux ls >"$RESULTS/tmux-sessions.txt" 2>&1 || true
verdict "NOTE bench tmux sessions: $(tr '\n' ';' <"$RESULTS/tmux-sessions.txt")"
# The breadcrumb's pickers (W11h's screen): the windows, a new one named.
hook '{"cmd":"tell","event":{"ev":"crumb","crumb":"window"}}' >/dev/null
sleep 3
if [ "$(top_page)" = mux-picker ]; then
    shot 21b-tmux-picker-windows
    hook '{"cmd":"tell","event":{"ev":"new","name":"second"}}' >/dev/null
    windows=0
    for _ in $(seq 1 20); do
        windows=$(TMUX_TMPDIR="$BENCH/tmux" tmux list-windows -a -F '#W' 2>/dev/null | grep -c . || true)
        [ "$windows" -ge 2 ] && break
        sleep 1
    done
    if TMUX_TMPDIR="$BENCH/tmux" tmux list-windows -a -F '#W' 2>/dev/null | grep -qx second; then
        verdict "PASS the windows picker made a window named 'second' on the bench's tmux ($windows windows)"
    else
        verdict "FAIL the picker's new window: $(TMUX_TMPDIR="$BENCH/tmux" tmux list-windows -a 2>&1 | tr '\n' ';')"
    fi
    sleep 2
    hook '{"cmd":"tell","event":{"ev":"crumb","crumb":"session"}}' >/dev/null
    sleep 3
    [ "$(top_page)" = mux-picker ] && shot 21c-tmux-picker-sessions
    hook '{"cmd":"back"}' >/dev/null
    sleep 1
else
    verdict "FAIL the breadcrumb opened no picker (top: $(top_page))"
fi
# Set up this host, on the bench's HOME: look, install Claude Code's hooks,
# prove them in the pane, take them out.
hs_wait() { # $1: a regex the page's state must match, $2 seconds
    for _ in $(seq 1 "$2"); do
        hook '{"cmd":"host-setup"}' | grep -Eq "$1" && return 0
        sleep 1
    done
    hook '{"cmd":"host-setup"}' >&2
    return 1
}
hook '{"cmd":"open","page":"settings"}' >/dev/null
sleep 2
hook '{"cmd":"tell","event":{"ev":"open","page":"host-setup"}}' >/dev/null
if [ "$(top_page)" = host-setup ] && hs_wait '"phase":"idle"' 60; then
    hook '{"cmd":"host-setup"}' >"$RESULTS/host-setup-inspected.json"
    shot 21d-host-setup
    grep -q '"uname"' "$RESULTS/host-setup-inspected.json" && verdict "PASS Set up this host looked at the host: $(sed -nE 's/.*"uname":"([^"]*)".*/\1/p' "$RESULTS/host-setup-inspected.json" | cut -c1-40)" ||
        verdict "FAIL Set up this host: $(cat "$RESULTS/host-setup-inspected.json")"
    hook '{"cmd":"tell","event":{"ev":"install-hooks"}}' >/dev/null
    if hs_wait '"hooks":"current".*"phase":"idle"|"phase":"idle".*"hooks":"current"' 90; then
        grep -q moshpit "$BENCH/home/.claude/settings.json" 2>/dev/null &&
            verdict "PASS installed Claude Code's hooks in the bench's ~/.claude/settings.json" ||
            verdict "FAIL hooks current, but the bench's settings.json has no Moshpit hook"
        shot 21e-host-setup-installed
        hook '{"cmd":"tell","event":{"ev":"prove-hooks"}}' >/dev/null
        if hs_wait '"hooksProof":\{[^}]*"state":"(proven|failed|unavailable)"' 60; then
            verdict "NOTE hooks proof: $(hook '{"cmd":"host-setup"}' | sed -nE 's/.*"hooksProof":(\{[^}]*\}).*/\1/p')"
        fi
        hook '{"cmd":"tell","event":{"ev":"remove-hooks"}}' >/dev/null
        hs_wait '"hooks":"absent".*"phase":"idle"|"phase":"idle".*"hooks":"absent"' 90 &&
            verdict "PASS removed the hooks again" || verdict "FAIL removing the hooks"
    else
        verdict "FAIL installing the hooks: $(hook '{"cmd":"host-setup"}' | sed -nE 's/.*"error":"([^"]*)".*/\1/p')"
    fi
    hook '{"cmd":"back"}' >/dev/null
else
    verdict "FAIL Set up this host did not open or inspect (top: $(top_page))"
fi
hook '{"cmd":"back"}' >/dev/null
now_sum=$(shasum "$HOME/.claude/settings.json" 2>/dev/null | cut -d' ' -f1 || true)
[ "$now_sum" = "$REAL_CLAUDE_SUM" ] && verdict "PASS the user's own ~/.claude/settings.json is untouched" ||
    verdict "FAIL the user's ~/.claude/settings.json changed"
sleep 1
hook '{"cmd":"back"}' >/dev/null
sleep 2
shot 22-home-tmux-tree

# --- 2b. herdr on the Rust session ------------------------------------------------------
if [ -n "$HERDR" ]; then
    hook "{\"cmd\":\"enter\",\"connection\":\"$HERDR\"}" >/dev/null
    if wait_state "$HERDR" '"herdr":\{"activePane":"[^"]+","attached":true' 90; then
        verdict "PASS herdr: the Rust session's client attached to the bench's herdr ($(state_of "$HERDR" | sed -nE 's/.*"workspaces":(\[[^]]*\]).*/\1/p'))"
    else
        verdict "FAIL herdr: $(state_of "$HERDR")"
    fi
    sleep 3
    view_hook focus 'focus' >/dev/null
    sleep 2
    type_line 'echo herdr-$((6*7))'
    if wait_screen 'herdr-42' 30; then verdict "PASS typing in herdr's pane reaches the host"; else verdict "FAIL herdr pane echo"; fi
    shot 22b-terminal-herdr
    panes_before=$(H api snapshot 2>/dev/null | python3 -I -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["snapshot"]["panes"]))' || echo 0)
    hook '{"cmd":"tell","event":{"ev":"crumb","crumb":"pane"}}' >/dev/null
    sleep 3
    if [ "$(top_page)" = mux-picker ]; then
        shot 22c-herdr-picker-panes
        hook '{"cmd":"tell","event":{"ev":"new"}}' >/dev/null
        panes_after=$panes_before
        for _ in $(seq 1 20); do
            panes_after=$(H api snapshot 2>/dev/null | python3 -I -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["snapshot"]["panes"]))' || echo 0)
            [ "$panes_after" -gt "$panes_before" ] && break
            sleep 1
        done
        [ "$panes_after" -gt "$panes_before" ] && verdict "PASS herdr's panes picker split a pane ($panes_before -> $panes_after)" ||
            verdict "FAIL herdr's picker split nothing ($panes_before -> $panes_after)"
        sleep 2
        shot 22d-herdr-picker-split
        hook '{"cmd":"back"}' >/dev/null
    else
        verdict "FAIL herdr's breadcrumb opened no picker (top: $(top_page))"
    fi
    hook '{"cmd":"back"}' >/dev/null
    sleep 2
    shot 22e-home-herdr-tree
    hook "{\"cmd\":\"disconnect\",\"connection\":\"$HERDR\"}" >/dev/null
fi

# --- 3. mosh, and its UDP silence -------------------------------------------------------
hook "{\"cmd\":\"enter\",\"connection\":\"$MOSH\"}" >/dev/null
if wait_state "$MOSH" '"state":"connected"' 60; then
    state=$(state_of "$MOSH")
    pid=$(echo "$state" | sed -nE 's/.*"moshServerPid":([0-9]+).*/\1/p')
    [ -n "$pid" ] && MOSH_PIDS+=("$pid")
    verdict "PASS mosh connected (mosh-server pid ${pid:-?}, degrade $(echo "$state" | sed -nE 's/.*"degrade":("[^"]*"|null).*/\1/p'))"
else
    verdict "FAIL mosh: $(state_of "$MOSH")"
fi
view_hook focus 'focus' >/dev/null
wait_prompt
type_line 'echo mosh-ok-$((7*8))'
if wait_screen '^mosh-ok-56' 20; then verdict "PASS typing over mosh reaches the host"; else verdict "FAIL echo over mosh"; fi
shot 23-terminal-mosh
echo_line=$(view_hook echo 'echo n=' --ei count 40 --ei gap 150)
verdict "ECHO mosh (40 keys): $echo_line"
hook "{\"cmd\":\"fault\",\"connection\":\"$MOSH\",\"kind\":\"mosh-blackhole-on\"}" >/dev/null
silent_at=$(date +%s)
banner=0
for _ in $(seq 1 40); do
    if state_of "$MOSH" | grep -q '"returnPathDead":true'; then banner=1; break; fi
    sleep 1
done
if [ "$banner" = 1 ]; then
    verdict "PASS mosh UDP silence: the return-path banner after $(( $(date +%s) - silent_at )) s"
    shot 24-terminal-mosh-silence
else
    verdict "FAIL mosh UDP silence: no banner in 40 s ($(state_of "$MOSH"))"
fi
hook "{\"cmd\":\"fault\",\"connection\":\"$MOSH\",\"kind\":\"mosh-blackhole-off\"}" >/dev/null
sleep 3
type_line 'echo back-$((5*5))'
if wait_screen '^back-25' 30; then verdict "PASS mosh recovered after the silence (the server was not killed)"; else verdict "FAIL mosh after the silence"; fi
if [ -n "${pid:-}" ] && kill -0 "$pid" 2>/dev/null; then verdict "PASS mosh-server $pid still running after the silence"; fi
hook "{\"cmd\":\"disconnect\",\"connection\":\"$MOSH\"}" >/dev/null
hook "{\"cmd\":\"disconnect\",\"connection\":\"$SSH_TMUX\"}" >/dev/null
hook "{\"cmd\":\"disconnect\",\"connection\":\"$SSH_PLAIN\"}" >/dev/null
sleep 3
cp "$BENCH/sshd.log" "$RESULTS/sshd.log"
logins=$(grep -c 'Accepted publickey' "$BENCH/sshd.log" || true)
verdict "NOTE sshd logins: $logins"

echo
echo "results: $RESULTS"
cat "$SUMMARY"
