#!/bin/bash
# Imports the wezterm crates the shared terminal core (crates/term) builds on
# into vendor/wezterm, unchanged, from an upstream checkout at one commit.
# Our changes to them are separate commits on top (docs/WEZTERM-PATCHES.md),
# so an upstream update is a merge, not a re-patch:
#
#   git switch -c wezterm-import <the last import commit>   # see WEZTERM-PATCHES.md
#   scripts/wezterm/import.sh /path/to/wezterm <new rev>
#   git commit -am "chore(wezterm): import upstream <new rev>"
#   git switch <our branch> && git merge wezterm-import       # our patches stay
#
#   import.sh <upstream checkout> <rev>
set -euo pipefail
src="$1"
rev="$2"
dst="$(cd "$(dirname "$0")/../.." && pwd)/vendor/wezterm"
# wezterm-term and every crate of wezterm's repository it builds on.
crates=(term termwiz vtparse bidi wezterm-blob-leases wezterm-cell wezterm-char-props
        color-types wezterm-dynamic wezterm-escape-parser wezterm-input-types
        wezterm-surface filedescriptor)
git -C "$src" checkout -q "$rev"
mkdir -p "$dst"
for c in "${crates[@]}"; do
    # Not the table generators of bidi and char-props or their Unicode data
    # (14 MB for bidi alone): the tables they generate are in src/.
    skip=()
    case "$c" in
        bidi|wezterm-char-props) skip=(--exclude /data/ --exclude /generate/ --exclude /codegen/) ;;
    esac
    rsync -a --delete ${skip[@]+"${skip[@]}"} "$src/$c/" "$dst/$c/"
done
cp "$src/LICENSE.md" "$dst/LICENSE.md"
echo "$(git -C "$src" rev-parse HEAD)" > "$dst/UPSTREAM"
echo "imported wezterm $(cat "$dst/UPSTREAM") into $dst"
