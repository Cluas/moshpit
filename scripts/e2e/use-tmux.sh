#!/bin/sh
# Puts a built tmux where the live tests' host finds it.
#   path      /usr/local/bin/tmux: on sshd's own non-interactive PATH, the
#             usual Linux host.
#   homebrew  /opt/homebrew/bin/tmux only, as on a Mac with Homebrew: not
#             on a non-interactive sshd's PATH, so only the app's probed
#             PATH finds it (B788, B850 went wrong exactly there).
#
#   scripts/e2e/use-tmux.sh /opt/e2e/tmux-3.8 homebrew
set -eu
built=$1
variant=$2
sudo=""
[ "$(id -u)" -eq 0 ] || sudo=sudo
$sudo rm -f /usr/local/bin/tmux /opt/homebrew/bin/tmux
if command -v tmux >/dev/null 2>&1; then
    echo "use-tmux: a tmux is still on PATH: $(command -v tmux)" >&2
    exit 1
fi
case "$variant" in
    path) $sudo ln -s "$built/bin/tmux" /usr/local/bin/tmux ;;
    homebrew) $sudo mkdir -p /opt/homebrew/bin && $sudo ln -s "$built/bin/tmux" /opt/homebrew/bin/tmux ;;
    *) echo "use-tmux: unknown variant $variant" >&2; exit 2 ;;
esac
# The tmux crate's live tests drive this one directly.
echo "MOSHPIT_TEST_TMUX=$built/bin/tmux"
