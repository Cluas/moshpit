#!/usr/bin/env bash
# Installs this repository's pre-push gate (scripts/git-hooks/pre-push) for
# every worktree of the checkout: a launcher in the shared hooks folder runs
# the gate of the worktree being pushed from, and refuses the push when
# that worktree has none (a branch from before the gate existed: rebase it
# onto main first). Run once per clone:
#
#   scripts/setup-hooks.sh
#
# The gate's private lists come from a checkout of the ops repository
# (MOSHPIT_OPS, or ../moshpit-ops); this says whether it finds them.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
hooks="$(git rev-parse --git-common-dir)/hooks"
mkdir -p "$hooks"
if [ -e "$hooks/pre-push" ] && ! grep -q 'scripts/git-hooks/pre-push' "$hooks/pre-push"; then
    mv "$hooks/pre-push" "$hooks/pre-push.before-setup-hooks"
    echo "setup-hooks: the pre-push hook that was there is kept as pre-push.before-setup-hooks"
fi
cat > "$hooks/pre-push" <<'EOF'
#!/bin/sh
# Installed by scripts/setup-hooks.sh: the pushing worktree's own gate.
gate="$(git rev-parse --show-toplevel)/scripts/git-hooks/pre-push"
if [ ! -x "$gate" ]; then
    echo "pre-push: $gate isn't in this worktree; rebase it onto main, then push" >&2
    exit 1
fi
exec "$gate" "$@"
EOF
chmod +x "$hooks/pre-push"
echo "setup-hooks: pre-push gate installed in $hooks"

ops="${MOSHPIT_OPS:-$(dirname "$root")/moshpit-ops}"
if [ -f "$ops/app/oss/denylist.txt" ] && [ -f "$ops/app/oss/agent-names.txt" ]; then
    echo "setup-hooks: private lists found in $ops"
else
    echo "setup-hooks: no private lists in $ops: pushes are refused until the ops repository is" \
        "checked out there (or MOSHPIT_OPS names it); outside contributors set MOSHPIT_SCAN_PUBLIC_ONLY=1" >&2
fi
