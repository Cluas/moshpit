#!/usr/bin/env bash
# A staged host on this Mac for a VM to reach: the store shots a simulator
# can't take (the lock screen's question, the Dynamic Island) are taken in a
# vphone VM, and they are the app's real notification and Live Activity, so
# the app needs a live host. The same three hosts' sessions and stamps as a
# CI runner's (stage/layout.sh), with nothing of the person's touched and no
# sudo:
#
# - sshd: a private one, run as you, from a config, host key and authorized
#   key of its own in <work dir>, listening on <bind ip>:<port> only (the
#   VM's bridge, e.g. vphone's 192.168.64.1, never the LAN); its sessions'
#   HOME, ZDOTDIR and TMUX_TMPDIR in <work dir>, so none of your dotfiles,
#   ~/.ssh or tmux sockets is read or written.
# - tmux: three servers under <work dir>/tmux (-L moshpit-vmshots, -gpu,
#   -build), never your own.
# - no /etc/hosts: each connection's host is <bind ip>; their names stay
#   devbox, gpu-01 and build-box.
#
#   scripts/shots/stage-vm-host.sh start <work dir> <bind ip> [port]
#   scripts/shots/stage-vm-host.sh restamp <work dir>
#   scripts/shots/stage-vm-host.sh stop <work dir>
#
# `start` prints what the app is launched with (MOSHPIT_SEED_HOME_FIXTURE,
# base64; MOSHPIT_PET_PLIST, plist data, for -moshpit.pet.v1) and keeps the
# agents' stamps dated from now, a restamp every 45 s, until `stop`, which
# takes all of it down (sshd, the tmux servers, the restamper) and removes
# <work dir>.
set -euo pipefail

cmd="${1:-}"; work="${2:-}"
[ -n "$cmd" ] && [ -n "$work" ] || {
    echo "usage: $0 start <work dir> <bind ip> [port] | restamp <work dir> | stop <work dir>" >&2; exit 64; }
root="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mkdir -p "$work" && cd "$work" && pwd)"
user="$(id -un)"
home="$work/home"
tmuxdir="$work/tmux"
kit="$work/kit"
stage="$home/src"
scripts="$kit/scripts"
text="$kit/text"
hold="$scripts/hold.sh"

log() { echo "▶ $*" >&2; }
fail() { echo "✘ $*" >&2; exit 1; }

t_dev() { "$kit/stage-tmux" "$@"; }
t_gpu() { "$kit/stage-tmux-gpu" "$@"; }
t_build() { "$kit/stage-tmux-build" "$@"; }
tmux() { t_dev "$@"; }
gpu() { t_gpu "$@"; }
build() { t_build "$@"; }
. "$root/scripts/shots/stage/layout.sh"

# A process group's leader pid, its group killed whole.
kill_group() {  # <pid file>
    [ -f "$1" ] || return 0
    kill -- -"$(cat "$1")" 2>/dev/null || kill "$(cat "$1")" 2>/dev/null || true
    rm -f "$1"
}

case "$cmd" in
restamp)
    stamp_all
    exit 0
    ;;
stop)
    kill_group "$work/restamper.pid"
    for wrapper in stage-tmux stage-tmux-gpu stage-tmux-build; do
        [ -x "$kit/$wrapper" ] && "$kit/$wrapper" kill-server 2>/dev/null || true
    done
    kill_group "$work/sshd.pid.group"
    rm -rf "$work"
    exit 0
    ;;
start) ;;
*) fail "start, restamp or stop, not $cmd" ;;
esac

bind="${3:-}"; port="${4:-22622}"
[ -n "$bind" ] || fail "start needs the address to listen on (the VM's bridge, e.g. 192.168.64.1)"
ifconfig | grep -q "inet $bind " || fail "no interface on this Mac has $bind"
[ ! -e "$work/sshd.pid.group" ] || fail "$work is staged already: stop it first"
mkdir -p "$home" "$tmuxdir" "$kit" "$scripts" "$text"
chmod 700 "$tmuxdir"
touch "$home/.zshrc"

log "a private sshd, as $user, on $bind:$port"
ssh-keygen -q -t ed25519 -N '' -C vmshots -f "$work/id"
cp "$work/id.pub" "$work/authorized_keys"
ssh-keygen -q -t ed25519 -N '' -C vmshots-host -f "$work/host_ed25519"
cat > "$work/sshd_config" <<EOF
Port $port
ListenAddress $bind
HostKey $work/host_ed25519
PidFile $work/sshd.pid
AuthorizedKeysFile $work/authorized_keys
AllowUsers $user
StrictModes no
UsePAM no
UseDNS no
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
SetEnv HOME=$home ZDOTDIR=$home TMUX_TMPDIR=$tmuxdir
Subsystem sftp internal-sftp
MaxStartups 100:30:200
LogLevel VERBOSE
EOF
# No throttling of a source that misbehaves: every connection the app
# makes comes from loopback (the VM), and OpenSSH 9.8+ penalises a
# source for each connection dropped before it signs in (an app killed
# between shots, a probe), until it stops answering it at all (dry run 6:
# "stopped answering during the SSH handshake", worse with every locale).
cp "$work/sshd_config" "$work/sshd_config.try"
echo "PerSourcePenalties no" >> "$work/sshd_config.try"
if /usr/sbin/sshd -t -f "$work/sshd_config.try" 2>/dev/null; then
    mv "$work/sshd_config.try" "$work/sshd_config"
else
    rm -f "$work/sshd_config.try"
fi
# Its own process group, so every connection it forks goes with it.
perl -e 'setpgrp(0, 0); exec @ARGV' /usr/sbin/sshd -D -f "$work/sshd_config" -E "$work/sshd.log" \
    < /dev/null > /dev/null 2>&1 &
echo $! > "$work/sshd.pid.group"
for _ in $(seq 100); do nc -z "$bind" "$port" > /dev/null 2>&1 && break; sleep 0.05; done
nc -z "$bind" "$port" > /dev/null 2>&1 || { cat "$work/sshd.log" >&2; fail "sshd didn't start"; }
ssh -F /dev/null -i "$work/id" -p "$port" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    -o BatchMode=yes "$user@$bind" true || { cat "$work/sshd.log" >&2; fail "sshd didn't let the key in"; }
fingerprint="$(ssh-keygen -l -E sha256 -f "$work/host_ed25519.pub" | awk '{print $2}')"
log "host key $fingerprint"

log "three private tmux servers under $tmuxdir"
# The binary itself (`tmux` is a function here).
tmux_bin="$(type -P tmux)" || fail "no tmux (brew install tmux)"
wrap() {  # <wrapper> <socket>
    cat > "$kit/$1" <<EOF
#!/bin/sh
unset TMUX
TMUX_TMPDIR="$tmuxdir" exec "$tmux_bin" -f /dev/null -L $2 "\$@"
EOF
    chmod +x "$kit/$1"
}
wrap stage-tmux moshpit-vmshots
wrap stage-tmux-gpu moshpit-vmshots-gpu
wrap stage-tmux-build moshpit-vmshots-build

log "staged repos in $stage"
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
stage_repo trainer main train.py 'from schedule import warmup_cosine'
mkdir -p "$stage/payments-api/bench"
xcrun swift "$root/scripts/shots/stage/latency-chart.swift" "$stage/payments-api/bench/latency.png" \
    || fail "the staged chart wasn't drawn"
cp "$root"/scripts/shots/stage/text/*.txt "$text/"
cp "$root"/scripts/shots/stage/{hold,claude-session,build-session}.sh "$scripts/"

stage_layout
stamp_all
t_dev list-panes -a -F 'devbox #{session_name}:#{window_name} #{pane_id} #{pane_current_command}' >&2
t_gpu list-panes -a -F 'gpu-01 #{session_name}:#{window_name} #{pane_id} #{pane_current_command}' >&2
t_build list-panes -a -F 'build-box #{session_name}:#{window_name} #{pane_id} #{pane_current_command}' >&2

# The stamps dated from now again every 45 s, in a group of its own.
perl -e 'setpgrp(0, 0); exec @ARGV' bash -c 'while sleep 45; do "$0" restamp "$1" 2>/dev/null; done' \
    "$root/scripts/shots/stage-vm-host.sh" "$work" < /dev/null > /dev/null 2>&1 &
echo $! > "$work/restamper.pid"

# Home: the three hosts, all at the bind address, all connecting.
fixture="$(python3 -I - "$(base64 < "$work/id" | tr -d '\n')" "$fingerprint" "$bind" "$port" "$user" \
    "$kit/stage-tmux" "$kit/stage-tmux-gpu" "$kit/stage-tmux-build" <<'PY'
import base64, json, sys
key, fingerprint, host, port, user, tmux, gpu, build = sys.argv[1:9]
hosts = [("devbox", tmux), ("gpu-01", gpu), ("build-box", build)]
fixture = {
    "key": key,
    "trust": [{"host": host, "port": int(port), "fingerprint": fingerprint}],
    "connections": [
        {"id": "5EED0000-0000-4000-8000-00000000000%d" % (i + 1), "name": name, "host": host,
         "port": int(port), "user": user, "mux": "tmux", "tmuxPath": path, "connect": True}
        for i, (name, path) in enumerate(hosts)
    ],
}
print(base64.b64encode(json.dumps(fixture).encode()).decode())
PY
)"
# The companion, hatched and grown, as ci-capture.sh seeds it.
day=86400; t=$(date +%s)
pet_json="{\"version\":1,\"seed\":\"ca9e919a9b819ab94bb856496e95c9eb\",\"hatched_at\":$((t - 20 * day)),\"hatch_hour\":14,\"name\":\"\",\"name_at\":0,\"finishes\":{\"shots\":40},\"stage\":2,\"stage_at\":[$((t - 20 * day)),$((t - 12 * day)),$((t - 3 * day))],\"flags\":0}"

cat <<EOF
MOSHPIT_SEED_HOME_FIXTURE=$fixture
MOSHPIT_PET_PLIST=<$(printf '%s' "$pet_json" | xxd -p | tr -d '\n')>
EOF
log "staged: launch with -MOSHPIT_SEED_HOME_FIXTURE and -moshpit.pet.v1 (above); stop with: $0 stop $work"
