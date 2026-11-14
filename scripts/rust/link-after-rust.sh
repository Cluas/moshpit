#!/bin/bash
# The Moshpit target's linker (LD and LDPLUSPLUS, project.yml): clang, once
# the Rust library a background "Build Rust" phase is building
# (scripts/rust/build-ios-background.sh) is done. With no background build
# (MOSHPIT_RUST_ASYNC not YES, the default) it is clang and nothing else.
#
# The link step runs without build settings in its environment, so the
# state directory comes from the -filelist argument:
# $TARGET_TEMP_DIR/Objects-normal/<arch>/Moshpit.LinkFileList.
filelist=""
prev=""
for arg in "$@"; do
    [ "$prev" = "-filelist" ] && filelist="$arg"
    prev="$arg"
done
state=""
[ -n "$filelist" ] && state="$(dirname "$(dirname "$(dirname "$filelist")")")/rust-background"

if [ -n "$state" ] && [ -f "$state/pid" ]; then
    pid="$(cat "$state/pid")"
    waited=0
    while [ ! -e "$state/status" ]; do
        if ! ps -o args= -p "$pid" 2>/dev/null | grep -q build-ios-background; then
            sleep 1 # its last write may be landing
            [ -e "$state/status" ] && break
            cat "$state/log" 2>/dev/null
            echo "error: the background Rust build (pid $pid) ended without a status: build again" >&2
            exit 1
        fi
        if [ "$waited" = 0 ]; then echo "note: waiting for the Rust library (crates/ffi, pid $pid)"; fi
        waited=$((waited + 1))
        sleep 1
    done
    # cargo's output, in this link's log, where a failed Rust build's errors show.
    cat "$state/log"
    status="$(cat "$state/status")"
    if [ "$status" != 0 ]; then
        echo "error: the Rust library (crates/ffi) failed to build (exit $status): cargo's output is above" >&2
        exit 1
    fi
    echo "note: the Rust library was ready after ${waited} s of waiting"
fi
exec "$(xcrun --find clang)" "$@"
