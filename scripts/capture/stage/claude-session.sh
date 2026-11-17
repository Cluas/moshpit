#!/bin/sh
# A Claude Code turn, replayed for capture.
# Synced with scripts/shots/stage/claude-session.sh via claude-stage.c.
dir="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$dir/../../.." && pwd)"
stage_c="$root/scripts/shots/stage/claude-stage.c"
if [ -x "$dir/claude" ]; then
    exec "$dir/claude"
fi
if [ -f "$stage_c" ]; then
    bin="${TMPDIR:-/tmp}/moshpit-claude-stage-$(id -u)"
    mkdir -p "$bin"
    if [ ! -x "$bin/claude" ]; then
        cc -O2 "$stage_c" -o "$bin/.claude.$$" && mv -f "$bin/.claude.$$" "$bin/claude"
    fi
    exec "$bin/claude"
fi
exec sh "$root/scripts/shots/stage/hold.sh" claude
