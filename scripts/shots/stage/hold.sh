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
bin="${TMPDIR:-/tmp}/moshpit-shots-bin-$(id -u)"
mkdir -p "$bin"
[ -x "$bin/$1" ] || cp "$(command -v sleep)" "$bin/$1"
exec "$bin/$1" 2147483647
