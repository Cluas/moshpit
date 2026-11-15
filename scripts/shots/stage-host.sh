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

# The agents' states, stamped as the host's hooks stamp them
# (scripts/host/moshpit-stamp.sh: the pane's @moshpit_state, _agent, _since,
# _title): two waiting on the person, four at work, two done. Every agent
# pane's foreground is its agent's name (stage/hold.sh), never a shell, so
# the app takes the stamps as they are (crates/agents' reclassify); the
# done claude's pane is back at its shell, as a finished one is. Dated from
# now: `restamp` dates them again, so a timer a shot shows an hour into the
# run still reads minutes (the capture restamps while it shoots).
stamp_all() {
    local now
    now=$(date +%s)
    stamp() {  # <tmux> <pane> <state> <agent> <since> [title]
        "$1" set -p -t "$2" @moshpit_state "$3"
        "$1" set -p -t "$2" @moshpit_agent "$4"
        "$1" set -p -t "$2" @moshpit_since "$5"
        [ -z "${6:-}" ] || "$1" set -p -t "$2" @moshpit_title "$6"
    }
    stamp t_dev work:claude attention claude $((now - 95)) \
        "Claude needs your permission to use Edit"
    stamp t_dev work:build working codex $((now - 380)) "Running the test suite"
    stamp t_dev work:review.0 working claude $((now - 610)) "Writing tests for backoff.js"
    stamp t_dev work:shell done claude $((now - 1500))
    stamp t_gpu train:claude working claude $((now - 1260)) "Tuning the learning-rate warmup"
    stamp t_gpu train:eval attention gemini $((now - 40)) \
        "Allow execution of: 'python eval.py'?"
    stamp t_build ci:deploy working claude $((now - 200)) "Planning the bucket move"
    stamp t_build ci:codex done codex $((now - 900))
}

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
EOF
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

tmux_bin="$(command -v tmux)" || fail "no tmux (brew install tmux)"
wrap() {  # <wrapper> <socket>
    cat > "$kit/$1" <<EOF
#!/bin/sh
exec "$tmux_bin" -L $2 "\$@"
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
text="$kit/text"
mkdir -p "$text"
cat > "$text/review-claude.txt" <<'TEXT'
> add tests for backoff.js: the cap, the jitter, the retry count

● I'll cover the cap, the jitter bounds and the retry count.

● Write(test/backoff.test.js)
  ⎿  58 lines

● Bash(npx vitest run test/backoff.test.js)
  ⎿  ✓ caps the wait at 30s
     ✓ keeps jitter inside ±20%
     ✓ gives up after the last try

● Now the queue: a dropped event should be retried once more…
TEXT
cat > "$text/review-vitest.txt" <<'TEXT'
 DEV  v2.1.4 /srv/payments-api

 ✓ test/backoff.test.js (12 tests) 96ms
 ✓ src/retry.test.js (9 tests) 412ms
 ✓ src/webhook.test.js (14 tests) 806ms

 Test Files  3 passed (3)
      Tests  35 passed (35)

 PASS  Waiting for file changes...
TEXT
cat > "$text/review-tail.txt" <<'TEXT'
09:41:02 POST /webhooks/stripe 200 41ms
09:41:03 retry evt_1Q2 attempt=2 wait=1.6s
09:41:05 POST /webhooks/stripe 200 38ms
09:41:05 retry evt_1Q7 attempt=1 wait=0.8s
09:41:06 POST /webhooks/stripe 200 44ms
09:41:08 delivered evt_1Q2 after 2 tries
TEXT
cat > "$text/train-claude.txt" <<'TEXT'
> the loss spikes at step 2k, fix the warmup

● The warmup ends at step 2,000 while the batch is still growing.
  I'll stretch it to 4,000 and decay with a cosine.

● Update(schedule.py)
  ⎿  Updated 2 additions and 1 removal

● Bash(python train.py --steps 6000 --log-every 500)
  ⎿  step 500   loss 3.912
     step 1000  loss 3.104
     step 1500  loss 2.788
TEXT
cat > "$text/train-gemini.txt" <<'TEXT'
> run the eval on the held-out set

✦ I'll run the evaluation on the 12k held-out examples.

  Shell python eval.py --split heldout --batch 64

 Allow execution of: 'python eval.py'?

 ● 1. Yes, allow once
   2. Yes, allow always
   3. No, suggest changes (esc)
TEXT
cat > "$text/ci-claude.txt" <<'TEXT'
> plan the bucket move to the new region

● Reading the state first: 14 resources, 2 of them buckets.

● Bash(terraform plan -out move.plan)
  ⎿  Refreshing state... [id=artifacts]
     Plan: 2 to add, 0 to change, 2 to destroy.
TEXT
cat > "$text/shell.txt" <<'TEXT'
➜  infra git:(main) terraform fmt -check
➜  infra git:(main)
TEXT
cat > "$text/ci-codex.txt" <<'TEXT'
• Bumped the Node image to 22 in ci/Dockerfile and the workflow matrix.
• Ran the build: 3m12s, cache hit on the dependency layer.

✔ Done. 2 files changed, +4 -4.
TEXT
# The pane scripts beside the texts, where the session's user reads them.
mkdir -p "$kit/scripts"
cp "$root"/scripts/shots/stage/{hold,claude-session,build-session}.sh "$kit/scripts/"
install_kit
text="$box/text"
scripts="$box/scripts"
hold="$scripts/hold.sh"

# devbox: the payments work. Its claude asks to edit; codex runs the
# tests; a window split three ways (claude writing tests, vitest watching,
# the server's log); a shell where an earlier claude finished.
tmux kill-server 2>/dev/null || true
tmux new-session -d -s work -x 120 -y 40 -c "$stage/payments-api" -n claude \
    "exec sh $scripts/claude-session.sh"
tmux new-window -t work -n build -c "$stage/payments-api" "exec sh $scripts/build-session.sh"
tmux new-window -t work -n review -c "$stage/payments-api" "exec sh $hold claude $text/review-claude.txt"
tmux split-window -h -l 45% -t work:review -c "$stage/payments-api" "exec sh $hold vitest $text/review-vitest.txt"
tmux split-window -v -t work:review.1 -c "$stage/payments-api" "exec sh $hold tail $text/review-tail.txt"
tmux select-pane -t work:review.0
tmux new-window -t work -n shell -c "$stage/infra" "exec sh $hold zsh $text/shell.txt"
tmux select-window -t work:claude

# gpu-01: a training run (claude at work) and an eval waiting on gemini's
# question.
gpu kill-server 2>/dev/null || true
gpu new-session -d -s train -x 120 -y 40 -c "$stage/trainer" -n claude "exec sh $hold claude $text/train-claude.txt"
gpu new-window -t train -n eval -c "$stage/trainer" "exec sh $hold gemini $text/train-gemini.txt"
gpu select-window -t train:claude

# build-box: an infra plan (claude at work) and a CI bump codex finished.
build kill-server 2>/dev/null || true
build new-session -d -s ci -x 120 -y 40 -c "$stage/infra" -n deploy "exec sh $hold claude $text/ci-claude.txt"
build new-window -t ci -n codex -c "$stage/dashboard" "exec sh $hold codex $text/ci-codex.txt"
build select-window -t ci:deploy

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
