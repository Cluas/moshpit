# shellcheck shell=bash disable=SC2154 # sourced: the caller sets stage, scripts, text and hold
# The staged sessions and the agents' stamps, shared by stage-host.sh (a CI
# runner's host) and stage-vm-host.sh (a host on this Mac a VM reaches).
# Sourced. Expects:
#
#   tmux, gpu, build    a command each that runs tmux for one staged host
#                       (devbox, gpu-01, build-box) on its own socket
#   t_dev, t_gpu, t_build  the same, for the stamps (`restamp` runs them
#                       without the rest of the staging)
#   stage               the folder the repos are in
#   scripts, text, hold the pane scripts, their texts, stage/hold.sh, where
#                       the session's user reads them

# The three hosts' sessions over the staged repos.
stage_layout() {
    # devbox: the payments work. Its claude asks to edit; codex runs the
    # tests; a window split three ways (claude writing tests, vitest watching,
    # the server's log); a shell where an earlier claude finished; then the
    # server, the database, the log and the docs.
    tmux kill-server 2>/dev/null || true
    tmux new-session -d -s work -x 120 -y 40 -c "$stage/payments-api" -n claude \
        "exec sh $scripts/claude-session.sh"
    tmux new-window -t work -n build -c "$stage/payments-api" "exec sh $scripts/build-session.sh"
    tmux new-window -t work -n review -c "$stage/payments-api" "exec $scripts/claude --working $text/review-claude.txt"
    tmux split-window -h -l 45% -t work:review -c "$stage/payments-api" "exec sh $hold vitest $text/review-vitest.txt"
    tmux split-window -v -t work:review.1 -c "$stage/payments-api" "exec sh $hold tail $text/review-tail.txt"
    tmux select-pane -t work:review.0
    tmux new-window -t work -n shell -c "$stage/infra" "exec sh $hold zsh $text/shell.txt"
    # The rest of a working session: the dev server, the database, the
    # worker's log, the docs being written. No agent in them, so Home's
    # counts and the Live Activity stay as they are; they fill the
    # overview's grid (the iPad's held one row, the rest of it empty).
    tmux new-window -t work -n server -c "$stage/payments-api" "exec sh $hold node $text/server.txt"
    tmux new-window -t work -n db -c "$stage/payments-api" "exec sh $hold psql $text/db.txt"
    tmux new-window -t work -n logs -c "$stage/payments-api" "exec sh $hold tail $text/logs.txt"
    tmux new-window -t work -n docs -c "$stage/payments-api" "exec sh $hold nvim $text/docs.txt"
    tmux select-window -t work:claude

    # gpu-01: a training run (claude at work) and an eval waiting on gemini's
    # question.
    gpu kill-server 2>/dev/null || true
    gpu new-session -d -s train -x 120 -y 40 -c "$stage/trainer" -n claude "exec $scripts/claude --working $text/train-claude.txt"
    gpu new-window -t train -n eval -c "$stage/trainer" "exec sh $hold gemini $text/train-gemini.txt"
    gpu select-window -t train:claude

    # build-box: an infra plan (claude at work) and a CI bump codex finished.
    build kill-server 2>/dev/null || true
    build new-session -d -s ci -x 120 -y 40 -c "$stage/infra" -n deploy "exec $scripts/claude --working $text/ci-claude.txt"
    build new-window -t ci -n codex -c "$stage/dashboard" "exec sh $hold codex $text/ci-codex.txt"
    build select-window -t ci:deploy
}

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
    stamp t_dev work:shell "done" claude $((now - 1500))
    stamp t_gpu train:claude working claude $((now - 1260)) "Tuning the learning-rate warmup"
    stamp t_gpu train:eval attention gemini $((now - 40)) \
        "Allow execution of: 'python eval.py'?"
    stamp t_build ci:deploy working claude $((now - 200)) "Planning the bucket move"
    stamp t_build ci:codex "done" codex $((now - 900))
}

