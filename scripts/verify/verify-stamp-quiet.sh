#!/bin/sh
# The stamp script's quiet-notification behavior, against a real tmux pane and a
# FAKE sender that just logs its argv. Six claims:
#   1. an idle reminder on a parked (done) pane stamps nothing and pushes nothing
#   2. an attention answered within the grace window never reaches the sender
#   3. one that stands the window out reaches it exactly once
#   4. a done carries how long the closing episode ran, and the prompt it answered
#   5. an idle reminder heals a pane left stuck in attention
#   6. a turn Claude Code injected itself never becomes the finish card's prompt
set -eu
T=$(command -v tmux); S=stampquiet
SCRATCH=$(mktemp -d)
cleanup() { "$T" -L $S kill-server 2>/dev/null || true; rm -rf "$SCRATCH"; }
trap cleanup EXIT INT TERM
fail() { printf 'FAIL: %s\n' "$1" >&2; exit 1; }

mkdir -p "$SCRATCH/.moshpit"
cat > "$SCRATCH/.moshpit/moshpit-push.sh" <<'FAKE'
#!/bin/sh
echo "PUSH $*" >> "$HOME/push.log"
FAKE
chmod +x "$SCRATCH/.moshpit/moshpit-push.sh"

"$T" -L $S kill-server 2>/dev/null || true
"$T" -L $S new-session -d -s probe -x 100 -y 30
PANE=$("$T" -L $S list-panes -t probe -F '#{pane_id}' | head -1)

# Drive the REAL stamp script exactly as a hook would: inside the pane's env.
stamp() {  # $1=state $2=json-stdin
  printf '%s' "${2:-}" | HOME="$SCRATCH" TMUX=/tmp/x TMUX_PANE="$PANE" \
    MOSHPIT_NOTIFY_GRACE=2 \
    sh -c 'exec sh "'"$(pwd)"'/scripts/host/moshpit-stamp.sh" "$1" claude' _ "$1" \
    2>/dev/null
  # the script talks to tmux via `tmux` on PATH — point it at our socket
}
# The script calls bare `tmux`; interpose a shim on PATH.
mkdir -p "$SCRATCH/bin"
cat > "$SCRATCH/bin/tmux" <<SHIM
#!/bin/sh
exec "$T" -L $S "\$@"
SHIM
chmod +x "$SCRATCH/bin/tmux"
stamp() {
  printf '%s' "${2:-}" | HOME="$SCRATCH" PATH="$SCRATCH/bin:$PATH" TMUX=/tmp/x TMUX_PANE="$PANE" \
    MOSHPIT_NOTIFY_GRACE=3 sh "$(pwd)/scripts/host/moshpit-stamp.sh" "$1" claude 2>/dev/null
}
state() { "$T" -L $S display-message -p -t "$PANE" '#{@moshpit_state}'; }
pushes() { grep -c '^PUSH' "$SCRATCH/push.log" 2>/dev/null || true; }

echo "== 1: idle reminder on a parked pane =="
stamp done ""
[ "$(state)" = "done" ] || fail "setup: done stamp did not land"
sleep 1
: > "$SCRATCH/push.log"   # the setup's own done push is legitimate; not under test
stamp attention '{"hook_event_name":"Notification","message":"Claude is waiting for your input"}'
[ "$(state)" = "done" ] || fail "an idle nag flipped a parked pane to attention"
sleep 3
[ "$(pushes)" = "0" ] || fail "an idle nag pushed"
echo "   ok  stayed done, no push"

echo "== 2: a question answered within the grace never pushes =="
stamp working '{"hook_event_name":"UserPromptSubmit","prompt":"go"}'
stamp attention '{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"make"}}'
[ "$(state)" = "attention" ] || fail "a real mid-turn question did not stamp"
sleep 1
stamp working '{"hook_event_name":"UserPromptSubmit","prompt":"yes"}'   # answered at the desk
sleep 3
[ "$(pushes)" = "0" ] || fail "an answered question still pushed: $(cat "$SCRATCH/push.log"; echo; "$T" -L $S display-message -p -t "$PANE" "st=#{@moshpit_state} since=#{@moshpit_since}")"
echo "   ok  answered inside the window, phone never knew"

echo "== 3: one that stands the window out pushes exactly once =="
stamp attention '{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rm -rf build"}}'
sleep 5
[ "$(pushes)" = "1" ] || fail "expected exactly 1 push, got $(pushes)"
grep -q '^PUSH attention claude Bash: rm -rf build' "$SCRATCH/push.log" || fail "push argv wrong: $(cat "$SCRATCH/push.log")"
echo "   ok  pushed once, with the question"

echo "== 4: done carries the closing episode's length =="
sleep 2
stamp done ""
sleep 1
# The title slot carries the LAST PROMPT ("yes", claim 2's answer at the desk)
# — a finish card says what finished — then the duration.
grep -Eq '^PUSH done claude yes [0-9]+$' "$SCRATCH/push.log" || fail "done push should carry the prompt and the duration: $(tail -1 "$SCRATCH/push.log")"
DUR=$(tail -1 "$SCRATCH/push.log" | awk '{print $NF}')
[ "$DUR" -ge 5 ] && [ "$DUR" -le 20 ] || fail "duration $DUR implausible for a ~7s episode"
# And the pane's own title now reads the same, for the app's local card.
TITLE_DONE=$("$T" -L $S display-message -p -t "$PANE" '#{@moshpit_title}')
[ "$TITLE_DONE" = "yes" ] || fail "a done's title should be the prompt it answered, got: $TITLE_DONE"
echo "   ok  prompt=yes dur=$DUR s"

echo "== 5: an idle nag HEALS a fossil attention =="
# The frozen-for-a-day case: a pane stuck in attention (old stamp script, or a
# question that stopped existing) must be healed to done by the next idle
# reminder — silently.
"$T" -L $S set -p -t "$PANE" @moshpit_state attention
"$T" -L $S set -p -t "$PANE" @moshpit_title "Bash: some old question"
: > "$SCRATCH/push.log"
stamp attention '{"hook_event_name":"Notification","message":"Claude is waiting for your input"}'
[ "$(state)" = "done" ] || fail "a fossil attention was not healed (state=$(state))"
sleep 3
[ "$(pushes)" = "0" ] || fail "healing a fossil pushed"
TITLE_NOW=$("$T" -L $S display-message -p -t "$PANE" '#{@moshpit_title}')
[ -z "$TITLE_NOW" ] || fail "the fossil's title survived the heal: $TITLE_NOW"
echo "   ok  healed to done, silently, title cleared"

echo "== 6: an injected turn never becomes the finish card's prompt =="
# Claude Code fires UserPromptSubmit for its own synthetic turns too — a
# background task completing arrives as "<task-notification>…". The remembered
# prompt must stay the one the person typed, and the done must carry that.
stamp working '{"hook_event_name":"UserPromptSubmit","prompt":"ship it"}'
stamp working '{"hook_event_name":"UserPromptSubmit","prompt":"<task-notification>\n<task-id>bgh3exmc1</task-id>\n<tool-use-id>toolu_01X</tool-use-id>\n<status>completed</status>\n</task-notification>"}'
PROMPT_NOW=$("$T" -L $S display-message -p -t "$PANE" '#{@moshpit_prompt}')
[ "$PROMPT_NOW" = "ship it" ] || fail "an injected turn overwrote the remembered prompt: $PROMPT_NOW"
TITLE_NOW=$("$T" -L $S display-message -p -t "$PANE" '#{@moshpit_title}')
case "$TITLE_NOW" in *task-notification*) fail "an injected turn became the pane title: $TITLE_NOW" ;; esac
: > "$SCRATCH/push.log"
stamp done ""
sleep 1
grep -Eq '^PUSH done claude ship it [0-9]+$' "$SCRATCH/push.log" || fail "done should carry the typed prompt: $(tail -1 "$SCRATCH/push.log")"
echo "   ok  prompt stayed 'ship it', done carried it"

echo
echo "PASS — parked panes stay quiet, answered questions never ring, real ones ring once."
