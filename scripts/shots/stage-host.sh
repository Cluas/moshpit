#!/usr/bin/env bash
# A host staged on a CI runner (macOS) for the app on its simulator to reach:
# the screenshot run's (scripts/shots/ci-capture.sh) and the UI flows' that
# need a live session (Tests/MoshpitUITests/NSHSessionFlowUITest.swift, run
# by .github/workflows/ci.yml).
#
#   scripts/shots/stage-host.sh start <work dir>    stage it; print the seeds
#   scripts/shots/stage-host.sh restamp <work dir>  date the agents' states
#                                                   from now again
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
# Two more hosts for Home and the island, the same sshd under other names
# (gpu-01.example.com, build.example.com), each with a tmux of its own
# (-L moshpit-shots-gpu, -L moshpit-shots-build): their wrappers are
# MOSHPIT_SEED_TMUX_BIN_GPU and _BUILD.
# On a CI runner the session signs in as `dev`, a user made for the run, as
# every fixture names the person: Home's card says dev@devbox.example.com,
# not the runner's own login, and the repos sit in its home
# (~/src/payments-api). Anywhere else it is the person running this.
# Nothing of the person's: the key is thrown away with <work dir>, sshd runs
# from a config of its own on loopback, tmux on its own socket.
set -euo pipefail

cmd="${1:-}"; work="${2:-}"
[ -n "$cmd" ] && [ -n "$work" ] || { echo "usage: $0 start|stop <work dir>" >&2; exit 64; }
root="$(cd "$(dirname "$0")/../.." && pwd)"
port=2222
host=devbox.example.com
user="$(id -un)"
# What the session needs (wrappers, scripts, texts, the authorized key) is
# made in $kit and read from $box: on a runner, dev's home, as the work dir
# isn't dev's to read; elsewhere the same folder.
kit="$work/kit"
if [ -n "${GITHUB_ACTIONS:-}" ]; then
    user=dev
    box=/Users/dev/.moshpit-shots
    stage=/Users/dev/src
else
    box="$kit"
    stage="$work/stage"
fi
built="$work/stage"
# A command as the session's user (tmux runs as it, so its sockets are its).
as_user() { if [ "$user" = "$(id -un)" ]; then "$@"; else sudo -u "$user" -H "$@"; fi; }
t_dev() { as_user "$box/stage-tmux" "$@"; }
t_gpu() { as_user "$box/stage-tmux-gpu" "$@"; }
t_build() { as_user "$box/stage-tmux-build" "$@"; }
# $kit into $box, for the session's user.
install_kit() {
    [ "$box" = "$kit" ] && return 0
    sudo mkdir -p "$box"
    sudo cp -R "$kit/." "$box/"
    sudo chown -R "$user:staff" "$box"
}

log() { echo "▶ $*" >&2; }
fail() { echo "✘ $*" >&2; exit 1; }

# The staged sessions and the agents' stamps (stage/layout.sh): shared with
# stage-vm-host.sh.
. "$root/scripts/shots/stage/layout.sh"

if [ "$cmd" = restamp ]; then
    stamp_all
    exit 0
fi

if [ "$cmd" = stop ]; then
    for wrapper in stage-tmux stage-tmux-gpu stage-tmux-build; do
        [ -x "$box/$wrapper" ] && as_user "$box/$wrapper" kill-server 2>/dev/null || true
    done
    [ -f "$work/sshd.pid" ] && sudo kill "$(cat "$work/sshd.pid")" 2>/dev/null || true
    exit 0
fi
[ "$cmd" = start ] || fail "start, restamp or stop, not $cmd"
mkdir -p "$work" "$kit"

if [ "$user" != "$(id -un)" ] && ! id "$user" >/dev/null 2>&1; then
    log "a user for the session, $user"
    uid=$(( $(dscl . -list /Users UniqueID | awk '{print $2}' | sort -n | tail -1) + 1 ))
    sudo dscl . -create "/Users/$user"
    sudo dscl . -create "/Users/$user" UserShell /bin/zsh
    sudo dscl . -create "/Users/$user" RealName Dev
    sudo dscl . -create "/Users/$user" UniqueID "$uid"
    sudo dscl . -create "/Users/$user" PrimaryGroupID 20
    sudo dscl . -create "/Users/$user" NFSHomeDirectory "/Users/$user"
    sudo dscl . -passwd "/Users/$user" "$(openssl rand -hex 16)"
    sudo mkdir -p "/Users/$user"
    sudo chown "$user:staff" "/Users/$user"
    # Let in by Remote Login if it only lets in its own list.
    sudo dseditgroup -o edit -a "$user" -t user com.apple.access_ssh 2>/dev/null || true
    # No first-run wizard from zsh.
    as_user touch "/Users/$user/.zshrc"
fi

# What each SSH exec runs on the host, and when (B817: whether the app's
# path probe reached the host before a tap): the session user's zsh reads
# ~/.zshenv for every `zsh -c` sshd starts. Only that user's, made for the
# shots, never the runner's or a person's own.
if [ "$user" != "$(id -un)" ]; then
    as_user sh -c 'cat > "$HOME/.zshenv"' <<'EOF'
# Written by scripts/shots/stage-host.sh: what each SSH exec ran, and when.
if [[ -n $ZSH_EXECUTION_STRING ]]; then
    zmodload -F zsh/datetime p:EPOCHREALTIME b:strftime 2>/dev/null
    strftime -s _moshpit_t '%H:%M:%S' ${EPOCHREALTIME%.*}
    # An exec a line (a newline as \n): the path probe whole (B817: its
    # directory and patterns come last), any other long one's start and end.
    _moshpit_s=${ZSH_EXECUTION_STRING//$'\n'/\\n}
    if [[ $_moshpit_s != *moshpit-probe* ]] && (( ${#_moshpit_s} > 600 )); then
        _moshpit_s="${_moshpit_s[1,300]} [...] ${_moshpit_s[-300,-1]}"
    fi
    print -r -- "$_moshpit_t.${${EPOCHREALTIME#*.}[1,3]} $_moshpit_s" >> ~/.moshpit-exec.log
    unset _moshpit_t _moshpit_s
fi
EOF
    as_user sh -c ': > "$HOME/.moshpit-exec.log"'
fi

log "a throwaway key and an sshd of our own on 127.0.0.1:$port"
ssh-keygen -q -t ed25519 -N '' -C shots -f "$work/id"
cp "$work/id.pub" "$kit/authorized_keys"
chmod 644 "$kit/authorized_keys"
install_kit
ssh-keygen -q -t ed25519 -N '' -f "$work/host_ed25519"
cat > "$work/sshd_config" <<EOF
Port $port
ListenAddress 127.0.0.1
HostKey $work/host_ed25519
AuthorizedKeysFile $box/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM no
StrictModes no
PidFile $work/sshd.pid
Subsystem sftp /usr/libexec/sftp-server
AllowUsers $user
MaxStartups 100:30:200
LogLevel VERBOSE
EOF
# No throttling of a source that misbehaves: every connection the app
# makes comes from loopback (or the VM), and OpenSSH 9.8+ penalises a
# source for each connection dropped before it signs in (an app killed
# between shots, a probe), until it stops answering it at all (dry run 6:
# "stopped answering during the SSH handshake", worse with every locale).
cp "$work/sshd_config" "$work/sshd_config.try"
echo "PerSourcePenalties no" >> "$work/sshd_config.try"
if sudo /usr/sbin/sshd -t -f "$work/sshd_config.try" 2>/dev/null; then
    mv "$work/sshd_config.try" "$work/sshd_config"
else
    rm -f "$work/sshd_config.try"
fi
sudo /usr/sbin/sshd -f "$work/sshd_config" -E "$work/sshd.log"
for name in "$host" gpu-01.example.com build.example.com; do
    grep -q " $name\$" /etc/hosts || echo "127.0.0.1 $name" | sudo tee -a /etc/hosts >/dev/null
done
ssh_in() {
    ssh -i "$work/id" -p "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o BatchMode=yes "$user@$host" true
}
for _ in $(seq 1 20); do ssh_in 2>/dev/null && break; sleep 0.5; done
ssh_in || { cat "$work/sshd.log" >&2; fail "sshd never answered as $host"; }
fingerprint="$(ssh-keygen -l -E sha256 -f "$work/host_ed25519.pub" | awk '{print $2}')"
log "host key $fingerprint"

# The binary itself (`tmux` is a function here).
tmux_bin="$(type -P tmux)" || fail "no tmux (brew install tmux)"
wrap() {  # <wrapper> <socket>
    cat > "$kit/$1" <<EOF
#!/bin/sh
exec "$tmux_bin" -f /dev/null -L $2 "\$@"
EOF
    chmod +x "$kit/$1"
}
wrap stage-tmux moshpit-shots
wrap stage-tmux-gpu moshpit-shots-gpu
wrap stage-tmux-build moshpit-shots-build
tmux() { t_dev "$@"; }
gpu() { t_gpu "$@"; }
build() { t_build "$@"; }

log "staged repos and a tmux session on its own socket"
stage_repo() {  # <name> <branch> <file> <content>
    local dir="$built/$1"
    # Staged again (the iPad's run after the iPhone's, in ~/src): afresh.
    rm -rf "$dir"
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
stage_repo trainer main train.py 'from schedule import warmup_cosine'
# The chart the Claude turn says it wrote: the file-preview shot opens it.
mkdir -p "$built/payments-api/bench"
xcrun swift "$root/scripts/shots/stage/latency-chart.swift" "$built/payments-api/bench/latency.png" \
    || fail "the staged chart wasn't drawn"
# Into the session user's home (staged again afresh: the iPad's run after
# the iPhone's).
if [ "$stage" != "$built" ]; then
    for repo in payments-api dashboard infra trainer; do sudo rm -rf "${stage:?}/$repo"; done
    sudo mkdir -p "$stage"
    sudo cp -R "$built/." "$stage/"
    sudo chown -R "$user:staff" "$stage"
fi

# The small panes' text (the big ones replay a turn: stage/*-session.sh).
# The small panes' text (the big ones replay a turn: stage/*-session.sh).
mkdir -p "$kit/text"
cp "$root"/scripts/shots/stage/text/*.txt "$kit/text/"
# The pane scripts beside the texts, where the session's user reads them.
mkdir -p "$kit/scripts"
cp "$root"/scripts/shots/stage/{hold,claude-session,build-session}.sh "$kit/scripts/"
install_kit
text="$box/text"
scripts="$box/scripts"
hold="$scripts/hold.sh"

stage_layout
stamp_all
tmux list-panes -a -F 'devbox #{session_name}:#{window_name} #{pane_id} #{pane_current_command}' >&2
gpu list-panes -a -F 'gpu-01 #{session_name}:#{window_name} #{pane_id} #{pane_current_command}' >&2
build list-panes -a -F 'build-box #{session_name}:#{window_name} #{pane_id} #{pane_current_command}' >&2

cat <<EOF
MOSHPIT_SEED_USER=$user
MOSHPIT_SEED_KEY_B64=$(base64 < "$work/id" | tr -d '\n')
MOSHPIT_SEED_HOST=$host
MOSHPIT_SEED_PORT=$port
MOSHPIT_SEED_TRUST_FP=$fingerprint
MOSHPIT_SEED_NAME=devbox
MOSHPIT_SEED_MUX=tmux
MOSHPIT_SEED_TMUX_BIN=$box/stage-tmux
MOSHPIT_SEED_TMUX_BIN_GPU=$box/stage-tmux-gpu
MOSHPIT_SEED_TMUX_BIN_BUILD=$box/stage-tmux-build
EOF
