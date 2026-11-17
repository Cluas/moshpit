#!/bin/sh
# One leg of E2E-1: every `#[ignore]`d live test in the workspace (they drive a
# private sshd, tmux, mosh-server, herdr) against one tmux version, on a host
# set up by host-setup.sh. CI runs exactly this; so does the local Linux
# container.
#
#   scripts/e2e/run.sh <3.6a|3.8> <path|homebrew>
#
# `path` puts tmux on sshd's own PATH (/usr/local/bin); `homebrew` only in
# /opt/homebrew/bin, as on a Mac, where only the app's probed PATH finds it.
# Env: E2E_JOBS (tests at once, default 2), E2E_TMUX_ROOT (default /opt/e2e),
# E2E_FILTER (extra nextest filterset, to narrow a local run).
set -eu
version=$1
variant=$2
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
tmux_root=${E2E_TMUX_ROOT:-/opt/e2e}

scripts/e2e/build-tmux.sh "$version" "$tmux_root/tmux-$version"
eval "$(scripts/e2e/use-tmux.sh "$tmux_root/tmux-$version" "$variant" | sed -n 's/^MOSHPIT_TEST_TMUX=\(.*\)/export MOSHPIT_TEST_TMUX=\1/p')"
[ -n "${MOSHPIT_TEST_TMUX:-}" ] || { echo "run.sh: use-tmux gave no tmux" >&2; exit 1; }
"$MOSHPIT_TEST_TMUX" -V
# In `homebrew`, tmux lives in /opt/homebrew/bin: give test runners that PATH
# (as on a Mac) while sshd sessions still reset to system PATH (testing boot::exec).
[ "$variant" != homebrew ] || export PATH="$PATH:/opt/homebrew/bin"

export MOSHPIT_SSH_TEST_SSHD=${MOSHPIT_SSH_TEST_SSHD:-/usr/sbin/sshd}
export MOSHPIT_E2E_FAULTS=${MOSHPIT_E2E_FAULTS:-1}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$root/target}
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
# Debug info is most of a test binary's size and nothing here reads it.
export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-line-tables-only}

# Live test binaries: live.rs, live_*.rs, opensh/handshake_stall, and e2e2;
# exclude the local Mac-host throughput benchmark (live_window).
filter='binary(/^(live|live_.*|openssh|handshake_stall|e2e2)$/) & !binary(live_window)'
[ -z "${E2E_FILTER:-}" ] || filter="($filter) & ($E2E_FILTER)"

junit="$CARGO_TARGET_DIR/nextest/e2e/junit.xml"
rm -f "$junit"
set +e
cargo nextest run --target-dir "$CARGO_TARGET_DIR" \
    -p moshpit-session -p moshpit-ssh -p moshpit-tmux -p moshpit-app --tests \
    --profile e2e --run-ignored all --test-threads "${E2E_JOBS:-2}" -E "$filter" --no-tests=fail
status=$?
set -e
echo "nextest exit $status"
[ -f "$junit" ] || junit="$root/target/nextest/e2e/junit.xml"
python3 scripts/e2e/verdict.py "$junit" \
    scripts/e2e/quarantine.txt "$version" "$variant"
