# The headless emulator the Android runs use, sourced by android/test.sh and
# android/live.sh: the room check, the boot, the app's install and launch, and
# the test hooks (DebugReceiver.java, crates/android/src/app/debug.rs).
#
# The emulator boots only if this machine has room (memory free >= 30 %, swap
# free >= 1 GB; FORCE=1 skips the check: exit 3 means "not now"), runs headless
# on 2 cores and 2 GB (`-no-window -no-audio -no-boot-anim -no-snapshot`), and
# is killed when the script ends, however it ends (`emu_stop` from the
# caller's EXIT trap).

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
EMU_AVD=${EMU_AVD:-moshpit-g3}
EMU_PORT=${EMU_PORT:-5584}
SERIAL=emulator-$EMU_PORT
PACKAGE=com.cluas.moshpit
ADB_BIN=$(command -v adb || echo "$ANDROID_HOME/platform-tools/adb")
EMULATOR="$ANDROID_HOME/emulator/emulator"
EMU_PID=
LOGCAT_PID=

adb() { "$ADB_BIN" -s "$SERIAL" "$@"; }

emu_room() {
    local free_pct swap_free_mb
    free_pct=$(memory_pressure | awk -F': ' '/free percentage/ { gsub("%", "", $2); print $2 }')
    swap_free_mb=$(sysctl -n vm.swapusage | awk '{ for (i = 1; i <= NF; i++) if ($i == "free") { v = $(i + 2); sub("M", "", v); print int(v) } }')
    echo "memory free ${free_pct}%, swap free ${swap_free_mb} MB"
    if [ "${FORCE:-0}" != 1 ] && { [ "${free_pct:-0}" -lt 30 ] || [ "${swap_free_mb:-0}" -lt 1024 ]; }; then
        echo "not enough room to boot the emulator now (need 30 % free and 1 GB swap free)" >&2
        exit 3
    fi
}

emu_stop() {
    [ -n "$LOGCAT_PID" ] && kill "$LOGCAT_PID" 2>/dev/null || true
    if [ -n "$EMU_PID" ]; then
        "$ADB_BIN" -s "$SERIAL" emu kill >/dev/null 2>&1 || true
        for _ in $(seq 1 30); do kill -0 "$EMU_PID" 2>/dev/null || break; sleep 1; done
        kill "$EMU_PID" 2>/dev/null || true
        echo "emulator stopped"
        EMU_PID=
    fi
}

# Boots the AVD headless; $1 is where its log goes.
emu_boot() {
    [ -e "$HOME/.android/avd/$EMU_AVD.avd/config.ini" ] || "$HERE/avd.sh"
    echo "booting $EMU_AVD on $SERIAL"
    "$EMULATOR" -avd "$EMU_AVD" -port "$EMU_PORT" -no-window -no-audio -no-boot-anim -no-snapshot \
        -memory 2048 -cores 2 -gpu swiftshader_indirect >"$1" 2>&1 &
    EMU_PID=$!
    "$ADB_BIN" -s "$SERIAL" wait-for-device
    local booted=0
    for _ in $(seq 1 180); do
        if [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; then booted=1; break; fi
        kill -0 "$EMU_PID" 2>/dev/null || { echo "the emulator exited; see $1" >&2; exit 1; }
        sleep 5
    done
    [ "$booted" = 1 ] || { echo "the emulator did not boot in 15 minutes" >&2; exit 1; }
    for scale in window_animation_scale transition_animation_scale animator_duration_scale; do
        adb shell settings put global "$scale" 0
    done
    adb shell svc power stayon true
    adb shell input keyevent KEYCODE_WAKEUP
    adb shell wm dismiss-keyguard || true
    echo "booted: Android $(adb shell getprop ro.build.version.release | tr -d '\r')"
}

# Installs $1, clears the app's data, starts logcat into $2, launches it.
app_launch() {
    adb install -r -t "$1" >/dev/null
    adb shell pm clear "$PACKAGE" >/dev/null || true
    adb shell pm grant "$PACKAGE" android.permission.POST_NOTIFICATIONS 2>/dev/null || true
    adb logcat -c
    adb logcat -v time MoshpitCore:I MoshpitView:I MoshpitTest:I nsh:I AndroidRuntime:E DEBUG:I '*:S' >"$2" 2>&1 &
    LOGCAT_PID=$!
    LOG="$2"
    adb shell am start -W -n "$PACKAGE/.MainActivity" >/dev/null
    wait_log 'test hooks ready' 120
}

# Waits until the log has a line matching $1 (extended regex) after line
# ${3:-0}, $2 seconds at most.
wait_log() {
    local from=${3:-0}
    for _ in $(seq 1 "$2"); do
        tail -n +"$((from + 1))" "$LOG" | grep -Eq "$1" && return 0
        if grep -Eq 'FATAL EXCEPTION|Fatal signal' "$LOG"; then
            echo "the app crashed: see $LOG" >&2
            exit 1
        fi
        sleep 1
    done
    echo "timed out waiting for /$1/" >&2
    return 1
}

lines_now() { wc -l <"$LOG" | tr -d ' '; }

# Runs the controller's hook $1 (JSON); prints its answer.
hook() {
    local mark json
    mark=$(lines_now)
    json=$1
    printf '%s' "$json" >"$RESULTS/.cmd.json"
    adb push "$RESULTS/.cmd.json" /data/local/tmp/moshpit-cmd.json >/dev/null
    adb shell 'am broadcast -a com.cluas.moshpit.TEST --es json "$(cat /data/local/tmp/moshpit-cmd.json)"' >/dev/null
    wait_log 'MoshpitTest.*answer ' 30 "$mark" >/dev/null || true
    tail -n +"$((mark + 1))" "$LOG" | sed -nE 's/.*MoshpitTest[^:]*: answer (.*)/\1/p' | head -1
}

# Runs the view's hook $1 with extra am arguments; waits for a log line
# matching $2.
view_hook() {
    local cmd=$1 want=$2 mark
    shift 2
    mark=$(lines_now)
    adb shell am broadcast -a com.cluas.moshpit.TEST --es cmd "$cmd" "$@" >/dev/null
    wait_log "MoshpitTest.*$want" 60 "$mark" >/dev/null || true
    tail -n +"$((mark + 1))" "$LOG" | grep -E "MoshpitTest.*$want" | head -1 | sed -E 's/.*MoshpitTest[^:]*: //'
}

# The terminal's rows now.
screen_text() {
    local mark
    mark=$(lines_now)
    adb shell am broadcast -a com.cluas.moshpit.TEST --es cmd screen >/dev/null
    wait_log 'MoshpitTest.*screen (end|none)' 20 "$mark" >/dev/null || true
    tail -n +"$((mark + 1))" "$LOG" | sed -nE 's/.*MoshpitTest[^:]*: screen\| ?(.*)/\1/p'
}

# Waits until the terminal shows $1 (extended regex), $2 seconds at most.
wait_screen() {
    for _ in $(seq 1 "$2"); do
        screen_text | grep -Eq "$1" && return 0
        sleep 1
    done
    return 1
}

shot() { adb exec-out screencap -p >"$RESULTS/$1.png"; echo "screenshot $1.png"; }

# The center of the view whose resource id (an NSH test id) is $1, from the
# window hierarchy; empty when it is not on screen.
center_of() {
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
    adb shell cat /sdcard/ui.xml 2>/dev/null | python3 -I -c '
import re, sys
want = sys.argv[1]
xml = sys.stdin.read()
for node in re.findall(r"<node [^>]*>", xml):
    rid = re.search(r"resource-id=\"([^\"]*)\"", node)
    if rid and (rid.group(1) == want or rid.group(1).endswith(":id/" + want)):
        x1, y1, x2, y2 = map(int, re.search(r"bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"", node).groups())
        print((x1 + x2) // 2, (y1 + y2) // 2)
        break
' "$1"
}

SUMMARY=
verdict() { echo "$1" | tee -a "$SUMMARY"; }
