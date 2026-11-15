#!/usr/bin/env bash
# Installs this repository's pre-push gate (scripts/git-hooks/pre-push) for
# every worktree of the checkout: a launcher in the shared hooks folder runs
# the gate of the worktree being pushed from, and refuses the push when
# that worktree has none (a branch from before the gate existed: rebase it
# onto main first). Run once per clone:
#
#   scripts/setup-hooks.sh
#
# The gate's private lists come from the ops repository: a read-only clone
# next to this one (moshpit-ops-gate, or MOSHPIT_OPS_GATE), kept for the
# gate alone. The hook fast-forwards it before each public push, and
# `git config moshpit.opsDir` points the scan at it.
set -euo pipefail

common="$(git rev-parse --path-format=absolute --git-common-dir)"
hooks="$common/hooks"
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

gate="${MOSHPIT_OPS_GATE:-$(dirname "$(dirname "$common")")/moshpit-ops-gate}"
url="${MOSHPIT_OPS_URL:-git@github.com:Cluas/moshpit-ops.git}"
if [ -d "$gate/.git" ]; then
    git -C "$gate" fetch -q origin main && git -C "$gate" merge -q --ff-only origin/main && fetched=1 || fetched=""
else
    git clone -q --single-branch --branch main "$url" "$gate" && fetched=1 || fetched=""
fi
if [ -n "$fetched" ] && [ -f "$gate/app/oss/denylist.txt" ] && [ -f "$gate/app/oss/agent-names.txt" ]; then
    git -C "$gate" config moshpit.gateClone true
    git -C "$gate" remote set-url --push origin "read-only: the pre-push gate's copy"
    git config moshpit.opsDir "$gate"
    echo "setup-hooks: private lists from $gate (read-only, fast-forwarded before each public push)"
else
    echo "setup-hooks: no private lists ($url isn't reachable from here): pushes to the public" \
        "repository are refused; outside contributors set MOSHPIT_SCAN_PUBLIC_ONLY=1" >&2
fi
