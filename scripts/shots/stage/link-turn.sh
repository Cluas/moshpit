#!/bin/sh
# B841's pane (Tests/MoshpitUITests/ReportsUITest.swift): an address too
# long for Claude Code's width, as Claude Code writes it
# (crates/term/tests/b841.rs, claude_code_output): "⏺ " and the address's
# first 58 characters to the edge, the rest on the next row behind a
# 2-column indent the cursor jumps (never writes), each row inside an OSC 8
# link to the whole address (one id, BEL-terminated), re-opened on the
# second. A copy that reaches past its end must give the address whole.
url='https://example.com/docx/ZqLmOFywpjhAb3dEfGh/edit?usp=sharing&ouid=112233445566778899&rtpof=true&sd=true'
first=$(printf '%s' "$url" | cut -c1-58)
rest=$(printf '%s' "$url" | cut -c59-)
open=$(printf '\033]8;id=zts17q;%s\007' "$url")
printf '\033[H\033[2J\033[3J'
printf '\033[38;5;231m\033[49m\342\217\272\033[39m %s\033[94m%s\033]8;;\007\r\033[2C\033[1B%s%s\033[39m\033]8;;\007\r\n\r\n' \
    "$open" "$first" "$open" "$rest"
printf '\342\234\273 Baked for 3s\r\n'
exec sh "$(dirname "$0")/hold.sh" claude
