#!/bin/bash
# W14b on a real device: the features beside a terminal on a Rust session —
# path probe, preview, a stream's first MiB, a folder listing, an image
# upload, the maintenance pass's read of the host — on the session's own
# connection (borrowed) against a Swift SSH connection of their own (the
# app before W14b, `-MOSHPIT_SIDE_BORROW 0` on the same Debug build).
# Counts the sign-ins each run costs in a private sshd's log and prints the
# app's own timings (`RustSideProbe`, DEBUG launch arguments only).
#
# Private everything: an sshd started here as the current user on
# <address>:<port>, key auth only, its own host key, client key and config
# in a temp directory; its sessions get a scratch HOME (so the uploads and
# SFTP land there, `Subsystem sftp internal-sftp -d`) and TMUX_TMPDIR (a
# tmux server of their own — proven with `tmux ls` over the same sshd before
# anything runs). ~/.ssh, the machine's sshd and the user's tmux and
# mosh-servers are never addressed; mosh-servers this run started are found
# by the scratch HOME in their environment. Everything is stopped and
# removed on exit.
#
# Usage: scripts/verify/verify-device-side.sh <device-udid> <address of this Mac the device reaches> [<port>] [<scenario> ...]
#   scenarios: ssh ssh+tmux mosh mosh+tmux (default: all four)
# Needs the Debug app on the device (xcodebuild -scheme Moshpit, devicectl install).
set -uo pipefail
DEV=${1:?device udid (xcrun devicectl list devices)}
ADDR=${2:?an address of this Mac the device can reach}
PORT=${3:-$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("0.0.0.0", 0)); print(s.getsockname()[1])')}
shift $(( $# >= 3 ? 3 : $# ))
scenarios=("$@")
[ ${#scenarios[@]} -eq 0 ] && scenarios=(ssh ssh+tmux mosh mosh+tmux)
BUNDLE=com.cluas.moshpit
SSHD=${SSHD:-/usr/sbin/sshd}
TMUX=$(command -v tmux)
MOSH_SERVER=$(command -v mosh-server)
USER_NAME=$(id -un)
ROUNDS=${ROUNDS:-5}
# Never touched, whatever its environment says (the user's own).
PROTECTED_PIDS=" ${PROTECTED_PIDS:-} "

dir=$(mktemp -d /tmp/w14b-side.XXXXXX)
home=$dir/home
tmuxdir=$dir/tmux
pid=""
console=""

ours_mosh_servers() {
    # mosh-server daemonizes away from sshd, and its own environment as ps
    # shows it doesn't carry the session's: ours are the parents of the
    # shells whose environment names this run's TMUX_TMPDIR.
    ps -Eww -o pid=,ppid=,command= 2>/dev/null | awk -v mark="TMUX_TMPDIR=$tmuxdir" \
        'index($0, mark) && !index($0, "awk") { print $2 }' | sort -u | while read -r parent; do
        ps -o command= -p "$parent" 2>/dev/null | grep -q "mosh-server" && echo "$parent"
    done
}

cleanup() {
    [ -n "$console" ] && kill "$console" 2>/dev/null
    if [ -n "$pid" ]; then
        kill -TERM -- "-$pid" 2>/dev/null
        kill "$pid" 2>/dev/null
        wait "$pid" 2>/dev/null
    fi
    TMUX_TMPDIR="$tmuxdir" "$TMUX" kill-server 2>/dev/null
    for p in $(ours_mosh_servers); do
        case "$PROTECTED_PIDS" in *" $p "*) continue ;; esac
        kill "$p" 2>/dev/null
    done
    rm -rf "$dir"
}
trap cleanup EXIT INT TERM

mkdir -p "$home/notes" "$tmuxdir"
chmod 700 "$tmuxdir"
head -c 1048576 /dev/urandom > "$home/chart.png"
head -c 4194304 /dev/urandom > "$home/clip.mp4"
printf '# plan\n' > "$home/notes/plan.md"
ssh-keygen -q -t ed25519 -N '' -C w14b-host -f "$dir/host_ed25519"
ssh-keygen -q -t ed25519 -N '' -C w14b-client -f "$dir/client"
cp "$dir/client.pub" "$dir/authorized_keys"
cat > "$dir/sshd_config" <<EOF
Port $PORT
ListenAddress $ADDR
HostKey $dir/host_ed25519
PidFile $dir/sshd.pid
AuthorizedKeysFile $dir/authorized_keys
StrictModes no
UsePAM no
UseDNS no
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
SetEnv HOME=$home ZDOTDIR=$home TMUX_TMPDIR=$tmuxdir MOSH_SERVER_NETWORK_TMOUT=300
Subsystem sftp internal-sftp -d $home
LogLevel VERBOSE
EOF
# Its own process group, so every connection it forks goes with it.
perl -e 'setpgrp(0, 0); exec @ARGV' "$SSHD" -D -f "$dir/sshd_config" -E "$dir/sshd.log" < /dev/null &
pid=$!
for _ in $(seq 100); do nc -z "$ADDR" "$PORT" > /dev/null 2>&1 && break; sleep 0.05; done
nc -z "$ADDR" "$PORT" > /dev/null 2>&1 || { echo "sshd didn't start:"; cat "$dir/sshd.log"; exit 2; }

TMUX_TMPDIR="$tmuxdir" "$TMUX" -f /dev/null new-session -d -s w14b -x 120 -y 40
ssh_private() {
    ssh -F /dev/null -i "$dir/client" -o IdentitiesOnly=yes -o BatchMode=yes -o LogLevel=ERROR \
        -o UserKnownHostsFile="$dir/known_hosts" -o StrictHostKeyChecking=accept-new \
        -p "$PORT" "$USER_NAME@$ADDR" "$@"
}
listed=$(ssh_private "$TMUX ls -F '#{session_name}'" 2>&1)
if [ "$listed" != "w14b" ]; then
    echo "FAIL: the private sshd's sessions don't see only the private tmux server: $listed"
    exit 2
fi
echo "private sshd on $ADDR:$PORT; its tmux (TMUX_TMPDIR=$tmuxdir) lists: $listed"

count() { grep -c "Accepted publickey for $USER_NAME" "$dir/sshd.log"; }
FP=$(ssh-keygen -l -E sha256 -f "$dir/host_ed25519.pub" | awk '{print $2}')
KEY=$(base64 < "$dir/client" | tr -d '\n')

run() {
    local scenario=$1 mode=$2 id=$3
    local args=(-MOSHPIT_RUST_SESSION 1 -MOSHPIT_AUTOCARE_OFF
                -MOSHPIT_RUST_SEED_HOST "$ADDR" -MOSHPIT_RUST_SEED_PORT "$PORT"
                -MOSHPIT_RUST_SEED_USER "$USER_NAME" -MOSHPIT_RUST_SEED_KEY_B64 "$KEY"
                -MOSHPIT_RUST_SEED_TRUST_FP "$FP" -MOSHPIT_RUST_SEED_ID "$id"
                -MOSHPIT_RUST_SEED_TMUX_BIN "$TMUX" -MOSHPIT_RUST_SEED_MOSH_BIN "$MOSH_SERVER"
                -MOSHPIT_SIDE_PROBE "$home" -MOSHPIT_SIDE_ROUNDS "$ROUNDS")
    case $scenario in
        ssh+tmux) args+=(-MOSHPIT_RUST_SEED_MUX tmux) ;;
        mosh) args+=(-MOSHPIT_RUST_SEED_MOSH 1) ;;
        mosh+tmux) args+=(-MOSHPIT_RUST_SEED_MOSH 1 -MOSHPIT_RUST_SEED_MUX tmux) ;;
    esac
    [ "$mode" = dial ] && args+=(-MOSHPIT_SIDE_BORROW 0)
    local before log
    before=$(count)
    log="$dir/$scenario-$mode.log"
    xcrun devicectl device process launch --device "$DEV" --terminate-existing --console \
        "$BUNDLE" -- "${args[@]}" > "$log" 2>&1 &
    console=$!
    for _ in $(seq 240); do
        grep -q "side probe done" "$log" && break
        kill -0 "$console" 2>/dev/null || break
        sleep 0.5
    done
    sleep 2
    kill "$console" 2>/dev/null; wait "$console" 2>/dev/null; console=""
    echo "== $scenario, $mode: $(( $(count) - before )) sign-in(s)"
    grep -E "^\[w14b\] (side|round)|^\[w14\] seeded" "$log" | sed 's/^/   /'
    grep -q "side probe done" "$log" || { echo "   (no probe result)"; tail -5 "$log" | sed 's/^/   | /'; }
}

n=0
for scenario in "${scenarios[@]}"; do
    for mode in dial borrow; do
        n=$((n + 1))
        run "$scenario" "$mode" "$(printf '0B14B000-0000-4000-8000-%012d' "$n")"
        for p in $(ours_mosh_servers); do
            case "$PROTECTED_PIDS" in *" $p "*) continue ;; esac
            kill "$p" 2>/dev/null
        done
    done
done
# The app off the private host: its process ended.
xcrun devicectl device info processes --device "$DEV" --json-output "$dir/processes.json" > /dev/null 2>&1
app_pid=$(python3 -I -c 'import json, sys
for p in json.load(open(sys.argv[1]))["result"]["runningProcesses"]:
    if "/Moshpit.app/" in p.get("executable", ""):
        print(p["processIdentifier"]); break' "$dir/processes.json" 2>/dev/null)
[ -n "$app_pid" ] && xcrun devicectl device process terminate --device "$DEV" --pid "$app_pid" > /dev/null 2>&1
echo "done: the app's process ${app_pid:-(not found)} ended, the private sshd and its tmux are stopped next"
