#!/bin/sh
# Fills a staged pane and holds it under a program's name:
#
#   hold.sh <name> [<text file>]
#
# Clears the pane, prints the text file (if given), then becomes a copy of
# sleep called <name>. tmux reports a pane's command as its foreground
# process group's leader (osdep-darwin: tcgetpgrp, then that pid's name),
# so the pane says <name>, as it would for the real program. A script that
# merely sleeps in a loop leaves `sh` in front, and a shell in front is an
# agent gone (crates/agents' reclassify): its stamp would read as done on
# Home, in the island and in the overview.
[ -z "${2:-}" ] || { printf '\033[H\033[2J\033[3J'; cat "$2"; }
#
# Several panes start at once under one name (the staged claudes): the copy
# lands under its name in one rename, so a pane never runs one another pane
# is still writing (the split run's iPad leg: a half-copied claude died at
# once, its window renumbered, and the layout's next split found no pane 1).
bin="${TMPDIR:-/tmp}/moshpit-shots-bin-$(id -u)"
mkdir -p "$bin"
if [ ! -x "$bin/$1" ]; then
    cp "$(command -v sleep)" "$bin/.$1.$$" && mv -f "$bin/.$1.$$" "$bin/$1"
fi
exec "$bin/$1" 2147483647
