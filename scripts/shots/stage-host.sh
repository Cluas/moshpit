#!/usr/bin/env bash
# A host staged on a CI runner (macOS) for the app on its simulator to reach:
# the screenshot run's (scripts/shots/ci-capture.sh) and the UI flows' that
# need a live session (Tests/MoshpitUITests/NSHSessionFlowUITest.swift, run
# by .github/workflows/ci.yml).
#
#   scripts/shots/stage-host.sh start <work dir>    stage it; print the seeds
#   scripts/shots/stage-host.sh stop <work dir>     take it all down again
#
# `start` prints the app's seeds as KEY=value lines, one per line:
# MOSHPIT_SEED_USER, _KEY_B64, _HOST, _PORT, _TRUST_FP, _NAME, _MUX and
# _TMUX_BIN. The screenshot run passes them as launch arguments; CI hands
# them to the UI test runner as TEST_RUNNER_MOSHPIT_SEED_*.
#
# The host: sshd on 127.0.0.1:2222, answering as devbox.example.com (an
# /etc/hosts line for 127.0.0.1), signing in with a key made for this run;
# tmux on a private socket (-L moshpit-shots) with one session, "work", over
# staged repos whose panes are filled by scripts/shots/stage/ (and say so).
# Nothing of the person's: the key is thrown away with <work dir>, sshd runs
# from a config of its own on loopback, tmux on its own socket.
set -euo pipefail

cmd="${1:-}"; work="${2:-}"
[ -n "$cmd" ] && [ -n "$work" ] || { echo "usage: $0 start|stop <work dir>" >&2; exit 64; }
root="$(cd "$(dirname "$0")/../.." && pwd)"
port=2222
host=devbox.example.com
user="$(id -un)"

log() { echo "▶ $*" >&2; }
fail() { echo "✘ $*" >&2; exit 1; }

if [ "$cmd" = stop ]; then
    [ -x "$work/stage-tmux" ] && "$work/stage-tmux" kill-server 2>/dev/null || true
    [ -f "$work/sshd.pid" ] && sudo kill "$(cat "$work/sshd.pid")" 2>/dev/null || true
    exit 0
fi
[ "$cmd" = start ] || fail "start or stop, not $cmd"
mkdir -p "$work"

log "a throwaway key and an sshd of our own on 127.0.0.1:$port"
ssh-keygen -q -t ed25519 -N '' -C shots -f "$work/id"
cp "$work/id.pub" "$work/authorized_keys"
chmod 600 "$work/authorized_keys"
ssh-keygen -q -t ed25519 -N '' -f "$work/host_ed25519"
cat > "$work/sshd_config" <<EOF
Port $port
ListenAddress 127.0.0.1
HostKey $work/host_ed25519
AuthorizedKeysFile $work/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM no
StrictModes no
PidFile $work/sshd.pid
AllowUsers $user
EOF
sudo /usr/sbin/sshd -f "$work/sshd_config" -E "$work/sshd.log"
grep -q " $host\$" /etc/hosts || echo "127.0.0.1 $host" | sudo tee -a /etc/hosts >/dev/null
ssh_in() {
    ssh -i "$work/id" -p "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o BatchMode=yes "$user@$host" true
}
for _ in $(seq 1 20); do ssh_in 2>/dev/null && break; sleep 0.5; done
ssh_in || { cat "$work/sshd.log" >&2; fail "sshd never answered as $host"; }
fingerprint="$(ssh-keygen -l -E sha256 -f "$work/host_ed25519.pub" | awk '{print $2}')"
log "host key $fingerprint"

tmux_bin="$(command -v tmux)" || fail "no tmux (brew install tmux)"
cat > "$work/stage-tmux" <<EOF
#!/bin/sh
exec "$tmux_bin" -L moshpit-shots "\$@"
EOF
chmod +x "$work/stage-tmux"
tmux() { "$work/stage-tmux" "$@"; }

log "staged repos and a tmux session on its own socket"
stage="$work/stage"
stage_repo() {  # <name> <branch> <file> <content>
    local dir="$stage/$1"
    mkdir -p "$dir"
    git -C "$dir" init -q
    printf '%s\n' "$4" > "$dir/$3"
    git -C "$dir" add -A
    git -C "$dir" -c user.email=dev@example.com -c user.name=Dev commit -qm "Initial commit"
    [ "$2" = main ] || git -C "$dir" checkout -q -b "$2"
}
stage_repo payments-api fix-webhook-retry retry.js 'export function retry(fn, times = 3) {
  return fn();
}'
stage_repo dashboard main app.tsx 'export default function App() {
  return <Shell />;
}'
stage_repo infra main main.tf 'resource "aws_s3_bucket" "artifacts" {}'
tmux kill-server 2>/dev/null || true
tmux new-session -d -s work -x 120 -y 40 -c "$stage/payments-api" -n claude \
    "sh $root/scripts/shots/stage/claude-session.sh"
tmux new-window -t work -n build -c "$stage/payments-api" "sh $root/scripts/shots/stage/build-session.sh"
tmux new-window -t work -n shell -c "$stage/infra"
tmux select-window -t work:claude

# The agents' states, stamped as the host's hooks stamp them
# (scripts/host/moshpit-stamp.sh: the pane's @moshpit_state, _agent, _since,
# _title): one waiting on the person, one working, one done. The panes'
# foreground is the staged script's `sleep`, never a shell, so the app takes
# the stamps as they are (crates/agents' reclassify).
now=$(date +%s)
stamp() {  # <pane> <state> <agent> <since> [title]
    tmux set -p -t "$1" @moshpit_state "$2"
    tmux set -p -t "$1" @moshpit_agent "$3"
    tmux set -p -t "$1" @moshpit_since "$4"
    [ -z "${5:-}" ] || tmux set -p -t "$1" @moshpit_title "$5"
}
stamp work:claude attention claude $((now - 95)) "Claude needs your permission to use Edit"
stamp work:build working codex $((now - 380)) "Running the test suite"
stamp work:shell done claude $((now - 1500))
tmux list-windows -t work >&2

cat <<EOF
MOSHPIT_SEED_USER=$user
MOSHPIT_SEED_KEY_B64=$(base64 < "$work/id" | tr -d '\n')
MOSHPIT_SEED_HOST=$host
MOSHPIT_SEED_PORT=$port
MOSHPIT_SEED_TRUST_FP=$fingerprint
MOSHPIT_SEED_NAME=devbox
MOSHPIT_SEED_MUX=tmux
MOSHPIT_SEED_TMUX_BIN=$work/stage-tmux
EOF
