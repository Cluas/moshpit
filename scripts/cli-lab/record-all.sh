#!/bin/bash
# Records every CLI the lab can run, path by path (one path's CLIs at a
# time, in parallel), into crates/term/tests/fixtures/cli-lab.
set -u
here=$(cd "$(dirname "$0")" && pwd)
out=${OUT:-$here/../../crates/term/tests/fixtures/cli-lab}
mkdir -p "$out" /tmp/L
# What each CLI shows once its input box is up (one word: see drive.py).
ready() {
    case $1 in
        claude|codex) echo shortcuts ;;
        gemini|qwen) echo '@path' ;;
        copilot) echo 'trust' ;;
        qoder) echo 'shortcuts' ;;
        shell) echo '\$' ;;
    esac
}
opts() {
    local cli=$1
    echo --settle 4 --boot 60 --ready "$(ready "$cli")"
    # Copilot asks to trust the folder first (Enter: yes).
    [ "$cli" = copilot ] && echo --pre Enter
    # Qoder exits ~70 ms after its input box shows (no account): record its
    # first screens only (tmux_ref.py cuts before the exit below).
    [ "$cli" = qoder ] && echo --text - --reply 3
}
rec() { # cli path [extra...]
    local cli=$1 path=$2; shift 2
    # shellcheck disable=SC2046
    python3 "$here/drive.py" "$cli" "$path" --out "$out" $(opts "$cli") "$@" >"/tmp/L/$cli-$path$*.log" 2>&1
}
clis=${CLIS:-"shell qoder claude codex gemini qwen copilot"}
for path in ${PATHS:-tmux ssh mosh}; do
    for cli in $clis; do rec "$cli" "$path" & done
    wait
done
# mosh with tmux inside, and mosh-client's own prediction display on.
for cli in ${MOSH_EXTRA:-claude codex}; do
    rec "$cli" mosh-tmux &
    rec "$cli" mosh --predict always &
done
wait
# tmux's reading of the same bytes: Qoder's screen before its exit, and
# every recording that has no live tmux capture.
for f in "$out"/qoder-*.jsonl; do
    # The live capture shows tmux's "Pane is dead" after Qoder's exit.
    python3 -c 'import sys; f = sys.argv[1]; L = [l for l in open(f) if not ("\"ref\"" in l and "\"at\"" not in l)]; open(f, "w").writelines(L)' "$f"
    python3 "$here/tmux_ref.py" "$f" --before 'Custom.?Provider'
done
for f in "$out"/*-ssh.jsonl "$out"/*-mosh*.jsonl; do
    case $f in */qoder-*) ;; *) python3 "$here/tmux_ref.py" "$f" ;; esac
done
ls -la "$out"
