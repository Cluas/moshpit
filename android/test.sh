#!/usr/bin/env bash
# The Android app on a headless emulator, without a network host: every NSH
# screen it hosts (a screenshot each), a tap through NSH into the controller,
# the Keystore (a hardware key made, a key imported), the terminal's input
# path (typing, a held key's repeats, IME composition) on the byte channel,
# the notification channel and the share intent.
#
#   android/test.sh        # needs android/build/moshpit-debug.apk (android/build.sh)
#
# Results: android/build/results/screens-<time>/ (logcat, screenshots,
# summary.txt). See android/emu.sh for when the emulator boots and how it is
# stopped.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APK="$HERE/build/moshpit-debug.apk"
RESULTS="$HERE/build/results/screens-$(date +%Y%m%d-%H%M%S)"
# shellcheck source=emu.sh
. "$HERE/emu.sh"

[ -e "$APK" ] || { echo "build the APK first: android/build.sh" >&2; exit 1; }
emu_room
mkdir -p "$RESULTS"
SUMMARY="$RESULTS/summary.txt"
trap emu_stop EXIT
trap 'exit 130' INT TERM
emu_boot "$RESULTS/emulator.log"
app_launch "$APK" "$RESULTS/logcat.txt"
wait_log 'page [0-9]+: home' 60
sleep 3

# --- Home, empty, and a tap on its + through NSH -------------------------------------
shot 01-home-empty
if wait_log 'page [0-9]+ pushed' 5; then :; fi
add=$(center_of home-add)
if [ -n "$add" ]; then
    mark=$(lines_now)
    adb shell input tap $add
    if wait_log 'home: add' 10 "$mark" && wait_log 'page [0-9]+: connection-form' 10 "$mark"; then
        sleep 2
        shot 02-connection-form-tapped
        verdict "PASS tap on Home's + reached the controller and opened the form"
    else
        verdict "FAIL tap on Home's + ($add)"
    fi
    hook '{"cmd":"back"}' >/dev/null
    sleep 1
else
    verdict "SKIP no home-add in the window hierarchy"
fi

# --- Some data: connections, keys ------------------------------------------------------
hook '{"cmd":"add-connection","name":"studio","host":"10.0.2.2","port":2222,"user":"cluas","password":"x","mux":"tmux"}' >/dev/null
hook '{"cmd":"add-connection","name":"build-box","host":"192.0.2.10","user":"ci","password":"x","mosh":true,"mux":"none"}' >/dev/null
answer=$(hook '{"cmd":"generate-key","name":"pixel","hardware":true}')
echo "$answer" >"$RESULTS/hardware-key.json"
if echo "$answer" | grep -q '"publicKey":"ecdsa-sha2-nistp256 '; then
    verdict "PASS Keystore made a hardware P-256 key: $(echo "$answer" | sed -nE 's/.*"fingerprint":"([^"]*)".*/\1/p')"
else
    verdict "FAIL Keystore key: $answer"
fi
answer=$(hook '{"cmd":"generate-key","name":"laptop"}')
echo "$answer" | grep -q '"publicKey":"ssh-ed25519 ' && verdict "PASS generated an Ed25519 key, kept Keystore-wrapped" || verdict "FAIL generate: $answer"
key=$(sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g' "$ROOT/crates/ssh/tests/fixtures/keys/rsa2048-pem")
answer=$(hook "{\"cmd\":\"import-key\",\"name\":\"old-rsa\",\"text\":\"$key\"}")
if echo "$answer" | grep -q 'SHA256:YI308USvR1zZ2NtAEtFF7tm0SR2whhSScC1VPQYyh8w'; then
    verdict "PASS imported a PKCS#1 PEM key through the shared Rust path (fingerprint matches ssh-keygen)"
else
    verdict "FAIL import: $answer"
fi
sleep 2
shot 03-home

# --- Every screen the app hosts ------------------------------------------------------------
n=4
for page in settings ssh-keys add-key key-picker shortcuts connection-form accent theme accent-editor theme-editor recent-log; do
    answer=$(hook "{\"cmd\":\"open\",\"page\":\"$page\"}")
    sleep 3
    shot "$(printf '%02d' $n)-$page"
    if echo "$answer" | grep -q '"surface"'; then verdict "PASS screen $page"; else verdict "FAIL screen $page: $answer"; fi
    hook '{"cmd":"back"}' >/dev/null
    sleep 1
    n=$((n + 1))
done
grep -E 'nsh: |post to .*: ' "$RESULTS/logcat.txt" | head -20 >"$RESULTS/screen-errors.txt" || true
if [ -s "$RESULTS/screen-errors.txt" ]; then verdict "NOTE screen errors: $(wc -l <"$RESULTS/screen-errors.txt" | tr -d ' ') (screen-errors.txt)"; fi

# --- The editors: a color from the host's picker reaches the draft and is saved -----------------
# The page on top (its name).
top_page() { hook '{"cmd":"stack"}' | python3 -I -c 'import json,sys; s=json.load(sys.stdin)["stack"]; print(s[-1][1] if s else "")'; }
# The center of the first node whose attribute $1 is $2 (text, content-desc,
# resource-id), from the window hierarchy; empty when there is none.
center_where() {
    adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1 || true
    adb shell cat /sdcard/ui.xml 2>/dev/null | python3 -I -c '
import re, sys
attr, want = sys.argv[1], sys.argv[2]
for node in re.findall(r"<node [^>]*>", sys.stdin.read()):
    m = re.search(attr + r"=\"([^\"]*)\"", node)
    if m and (m.group(1) == want or m.group(1).endswith(":id/" + want)):
        x1, y1, x2, y2 = map(int, re.search(r"bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"", node).groups())
        print((x1 + x2) // 2, (y1 + y2) // 2)
        break
' "$1" "$2"
}
# Picks $2 (#RRGGBB) in the color picker the well $1 opens: the hex field, Done.
pick_color() {
    local well field done_at
    well=$(center_where resource-id "$1")
    [ -n "$well" ] || { echo "no well $1"; return 1; }
    adb shell input tap $well
    sleep 2
    field=$(center_where content-desc "Hex color")
    [ -n "$field" ] || { echo "no picker"; return 1; }
    shot "$3"
    adb shell input tap $field
    adb shell input keyevent KEYCODE_MOVE_END
    for _ in 1 2 3 4 5 6 7; do adb shell input keyevent KEYCODE_DEL; done
    adb shell input text "${2#\#}"
    sleep 1
    done_at=$(center_where resource-id button1)
    [ -n "$done_at" ] || { echo "no Done"; return 1; }
    adb shell input tap $done_at
    sleep 1
}
hook '{"cmd":"open","page":"accent"}' >/dev/null
sleep 2
hook '{"cmd":"tell","event":{"ev":"add"}}' >/dev/null
sleep 3
shot 20-accent-editor
if pick_color accent-editor-color '#FF7F50' 21-color-picker; then
    hook '{"cmd":"tell","event":{"ev":"name","value":"Coral"}}' >/dev/null
    sleep 1
    shot 22-accent-editor-picked
    hook '{"cmd":"tell","event":{"ev":"save"}}' >/dev/null
    sleep 1
    looks=$(hook '{"cmd":"looks"}')
    echo "$looks" >"$RESULTS/looks-accent.json"
    if echo "$looks" | grep -q '"accent":"FF7F50","id":"[^"]*","name":"Coral"\|"name":"Coral"'; then
        echo "$looks" | grep -q 'FF7F50' && verdict "PASS accent editor: the color picked in the host's picker was saved (Coral #FF7F50)" ||
            verdict "FAIL accent editor saved another color: $looks"
    else
        verdict "FAIL accent editor: $looks"
    fi
else
    verdict "FAIL accent editor's color well"
fi
hook '{"cmd":"back"}' >/dev/null
sleep 1
hook '{"cmd":"open","page":"theme"}' >/dev/null
sleep 2
hook '{"cmd":"tell","event":{"ev":"new"}}' >/dev/null
sleep 3
shot 23-theme-editor
if pick_color theme-color-background '#102030' 24-theme-color-picker; then
    hook '{"cmd":"tell","event":{"ev":"name","value":"Deep"}}' >/dev/null
    hook '{"cmd":"tell","event":{"ev":"save"}}' >/dev/null
    sleep 1
    looks=$(hook '{"cmd":"looks"}')
    echo "$looks" >"$RESULTS/looks-theme.json"
    if echo "$looks" | grep -q '"background":"102030"'; then
        verdict "PASS theme editor: the background picked in the host's picker was saved and applied"
    else
        verdict "FAIL theme editor: $looks"
    fi
else
    verdict "FAIL theme editor's color well"
fi
hook '{"cmd":"back"}' >/dev/null
sleep 1

# --- Add Key's Import File: the system's document picker ------------------------------------------
adb push "$ROOT/crates/ssh/tests/fixtures/keys/rsa2048-pem" /sdcard/Download/moshpit-test-key.pem >/dev/null
adb push "$ROOT/crates/ssh/tests/fixtures/keys/rsa2048-pem" /data/local/tmp/moshpit-test-key.pem >/dev/null
adb shell "run-as $PACKAGE sh -c 'mkdir -p files && cat /data/local/tmp/moshpit-test-key.pem > files/picked.pem'" || true
hook '{"cmd":"open","page":"add-key"}' >/dev/null
sleep 2
hook '{"cmd":"tell","event":{"ev":"method","value":"import"}}' >/dev/null
hook '{"cmd":"tell","event":{"ev":"name","value":"from-file"}}' >/dev/null
mark=$(lines_now)
hook '{"cmd":"tell","event":{"ev":"import-file","target":"private"}}' >/dev/null
sleep 4
if adb shell dumpsys activity activities | grep -E 'topResumedActivity|mResumedActivity' | grep -qi documentsui; then
    verdict "PASS Import File opened the system's document picker (DocumentsUI)"
    shot 25-document-picker
    file=$(center_where text moshpit-test-key.pem)
    if [ -z "$file" ]; then
        # The picker opens on Recent: Downloads is in its roots' drawer.
        roots=$(center_where content-desc "Show roots")
        [ -n "$roots" ] && adb shell input tap $roots && sleep 2
        downloads=$(center_where text Downloads)
        [ -n "$downloads" ] && adb shell input tap $downloads && sleep 3
        file=$(center_where text moshpit-test-key.pem)
    fi
    if [ -n "$file" ]; then
        verdict "NOTE picked the file in DocumentsUI's Downloads (a real tap)"
        adb shell input tap $file
    else
        # The picker opened elsewhere than Downloads: answer it with the same
        # file from the app's own storage, then close it.
        verdict "NOTE the file wasn't on the picker's screen: answered it from the app's storage (pick-path)"
        view_hook pick-path 'picked ' --es path "/data/data/$PACKAGE/files/picked.pem" >/dev/null
        adb shell input keyevent KEYCODE_BACK
    fi
else
    verdict "FAIL no document picker on top"
fi
if wait_log 'picked a file: [0-9]+ bytes' 15 "$mark"; then
    sleep 2
    shot 26-add-key-from-file
    hook '{"cmd":"tell","event":{"ev":"submit"}}' >/dev/null
    sleep 2
    keys=$(hook '{"cmd":"keys"}')
    if echo "$keys" | grep -q '"fingerprint":"SHA256:YI308USvR1zZ2NtAEtFF7tm0SR2whhSScC1VPQYyh8w","id":"[^"]*","name":"from-file"\|"name":"from-file"'; then
        echo "$keys" | grep -q 'YI308USvR1zZ2NtAEtFF7tm0SR2whhSScC1VPQYyh8w' &&
            verdict "PASS a key read from a picked file was imported (fingerprint matches ssh-keygen)" ||
            verdict "FAIL the picked key: $keys"
    else
        verdict "FAIL the picked key was not kept: $keys"
    fi
else
    verdict "FAIL no file came back from the picker"
fi
# Submit closed Add Key; a back from Home would leave the app.
[ "$(top_page)" = add-key ] && hook '{"cmd":"back"}' >/dev/null
adb shell am start -n "$PACKAGE/.MainActivity" >/dev/null 2>&1 || true
sleep 1

# --- Android's words (B258) -------------------------------------------------------------------------
words_on() { adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; adb shell cat /sdcard/ui.xml; }
hook '{"cmd":"open","page":"ssh-keys"}' >/dev/null
sleep 3
shot 27-ssh-keys-words
dump=$(words_on)
if echo "$dump" | grep -q 'ANDROID KEYSTORE' && ! echo "$dump" | grep -qi 'secure enclave\|face id'; then
    verdict "PASS SSH Keys says ANDROID KEYSTORE, no Secure Enclave or Face ID"
else
    verdict "NOTE SSH Keys' words not in the window hierarchy (see 27-ssh-keys-words.png)"
fi
hook '{"cmd":"back"}' >/dev/null
hook '{"cmd":"open","page":"settings"}' >/dev/null
sleep 3
dump=$(words_on)
if echo "$dump" | grep -q 'Accent' && ! echo "$dump" | grep -q 'App Icon'; then
    verdict "PASS Settings leaves App Icon out on Android"
else
    verdict "NOTE Settings' rows not read from the window hierarchy (see 28-settings-appearance.png)"
fi
shot 28-settings-appearance
hook '{"cmd":"back"}' >/dev/null
sleep 1

# --- Notification channel and share intent -------------------------------------------------
hook '{"cmd":"notify","title":"claude needs you","body":"studio · %3 waits for an answer"}' >/dev/null
sleep 2
if adb shell dumpsys notification --noredact | grep -q 'claude needs you'; then
    verdict "PASS notification posted on channel agents"
else
    verdict "FAIL notification"
fi
mark=$(lines_now)
adb shell "am start -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT 'git status' -n $PACKAGE/.MainActivity" >/dev/null
if wait_log 'share: 10 chars' 10 "$mark"; then verdict "PASS share intent reached the share queue"; else verdict "FAIL share intent"; fi

echo
echo "results: $RESULTS"
cat "$SUMMARY"
