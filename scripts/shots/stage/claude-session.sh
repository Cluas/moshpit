#!/bin/sh
# A Claude Code turn, replayed for capture.
#
# Why replayed and not live: a real `claude` in this pane renders the operator's
# own account state — "Welcome back <name>", login-expiry warnings, effort
# state. That is personal data in a marketing screenshot and changes shape
# between runs, making captures irreproducible.
#
# What is real: everything OUTSIDE the pane, plus keyboard input and terminal
# resize (`SIGWINCH`) driving the staged binary (`claude-stage.c`), so the
# inline permission panel, draft-hidden cue, and persistent bottom composer
# stay docked cleanly above Moshpit's key bar at every viewport height.
dir="$(cd "$(dirname "$0")" && pwd)"
if [ -x "$dir/claude" ]; then
    exec "$dir/claude"
fi

bin="${TMPDIR:-/tmp}/moshpit-claude-stage-$(id -u)"
mkdir -p "$bin"
if [ ! -x "$bin/claude" ]; then
    cc -O2 "$dir/claude-stage.c" -o "$bin/.claude.$$" && mv -f "$bin/.claude.$$" "$bin/claude"
fi
exec "$bin/claude"
