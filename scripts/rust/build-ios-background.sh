#!/bin/bash
# The Moshpit target's "Build Rust" phase when MOSHPIT_RUST_ASYNC is YES
# (project.yml): starts scripts/rust/build-ios.sh in the background and
# returns at once, so the app's Swift compiles while cargo builds the
# library, instead of after it. Xcode runs a target's script phases strictly
# before or after its Compile Sources (declared outputs and
# FUSE_BUILD_SCRIPT_PHASES don't change that, nor does moving the build into
# a target of its own), and the only step that needs the library is the
# link: scripts/rust/link-after-rust.sh, the target's linker, waits for this
# build, prints its output, and fails the link if it failed.
#
# The state lives in $TARGET_TEMP_DIR/rust-background: pid, log, and status
# (cargo's exit code, written last). The linker finds the same directory from
# its -filelist argument, as the link step gets no build settings.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
state="$TARGET_TEMP_DIR/rust-background"
mkdir -p "$state"

# A build cancelled before its link can leave its cargo running. Let it
# finish first: it would write its status over this build's.
if [ -f "$state/pid" ]; then
    old="$(cat "$state/pid")"
    # Ours only (the subshell below runs as this script): a pid can be reused.
    ours() { ps -o args= -p "$old" 2>/dev/null | grep -q build-ios-background; }
    if ours; then
        echo "note: waiting for the previous build's Rust library (pid $old)"
        while ours; do sleep 1; done
    fi
fi
rm -f "$state/status" "$state/status.tmp" "$state/log" "$state/pid"

# The link must not be judged up to date before cargo is done: with no
# Swift changed, Xcode would check the old archive now, find it unchanged
# and skip the link. Without the archive the link always runs (and waits).
# Cargo puts it back from deps/ without rebuilding when nothing changed.
rm -f "$root"/target/*/"${MOSHPIT_RUST_PROFILE:-app}"/libmoshpit_ffi.a

(
    rc=0
    "$root/scripts/rust/build-ios.sh" "$@" || rc=$?
    echo "$rc" >"$state/status.tmp"
    mv "$state/status.tmp" "$state/status"
) </dev/null >"$state/log" 2>&1 &
echo $! >"$state/pid"
disown
echo "note: building crates/ffi in the background (pid $(cat "$state/pid"), log $state/log); the link waits for it"
