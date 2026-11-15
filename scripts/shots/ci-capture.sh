#!/usr/bin/env bash
# The App Store and README screenshots, shot on a CI runner (macOS, Xcode 27):
# a staged host on the runner itself, the app seeded to each screen on a
# simulator, one PNG per shot. Run by .github/workflows/screenshots.yml; the
# shot list and why it is what it is: the release notes' SCREENSHOTS doc.
#
#   scripts/shots/ci-capture.sh <app> <out> <device> [locale…]
#
#   <app>     the Debug simulator build of Moshpit.app (DEBUG seeds in it)
#   <out>     where the PNGs go, one folder per locale
#   <device>  iphone | ipad
#   locale    en-US (default) zh-Hans zh-Hant ja de-DE es-ES fr-FR ko pt-BR
#
# Everything the app shows outside the panes is live: it connects over SSH
# to the runner, as devbox.example.com (an /etc/hosts line for 127.0.0.1),
# port 2222, to a tmux on a private socket. Only the panes' contents are
# staged (scripts/shots/stage/), and they say so. Every name a shot shows
# is example.com's or a made-up repo's: nothing of a person's.
#
# Nothing here is the person's: the key is made for the run and thrown
# away, sshd runs from a config of its own on loopback, tmux on its own
# socket, the repos under a temp folder.
set -euo pipefail

app="$1"; out="$2"; device="$3"; shift 3
locales=("$@"); [ "${#locales[@]}" -gt 0 ] || locales=(en-US)
root="$(cd "$(dirname "$0")/../.." && pwd)"
bundle=com.cluas.moshpit
work="$(mktemp -d)"
mkdir -p "$out"

log() { echo "▶ $*"; }
fail() { echo "✘ $*" >&2; exit 1; }

# ── The staged host ─────────────────────────────────────────────────────

log "the staged host (scripts/shots/stage-host.sh)"
trap '"$root/scripts/shots/stage-host.sh" stop "$work/host"; rm -rf "$work"' EXIT
"$root/scripts/shots/stage-host.sh" start "$work/host" > "$work/seed.env"
seed() { sed -n "s/^MOSHPIT_SEED_$1=//p" "$work/seed.env"; }
user="$(seed USER)"; key_b64="$(seed KEY_B64)"; host="$(seed HOST)"; port="$(seed PORT)"
fingerprint="$(seed TRUST_FP)"; stage_tmux="$(seed TMUX_BIN)"

# ── The simulator ───────────────────────────────────────────────────────

case "$device" in
    iphone) want="iPhone 17 Pro Max|iPhone 18 Pro Max"; px="1320x2868" ;;
    ipad) want="iPad Pro 13-inch \\(M5\\)|iPad Pro 13-inch \\(M4\\)"; px="2064x2752" ;;
    *) fail "device: iphone or ipad, not $device" ;;
esac
udid="$(xcrun simctl list devices available | grep -E "^ +($want) \(" | head -1 \
    | sed -nE 's/.*\(([0-9A-F-]{36})\).*/\1/p')"
if [ -z "$udid" ]; then
    # The image may carry the device type without a device made of it.
    type="$(xcrun simctl list devicetypes | grep -E "^($want) \(" | head -1 \
        | sed -nE 's/.*\((com\.apple\.[^)]+)\).*/\1/p')"
    runtime="$(xcrun simctl list runtimes available | grep -E '^iOS' | tail -1 \
        | sed -nE 's/.* - (com\.apple\.[^ ]+)$/\1/p')"
    [ -n "$type" ] && [ -n "$runtime" ] || fail "no $device simulator ($want) on this image"
    udid="$(xcrun simctl create "shots-$device" "$type" "$runtime")"
fi
log "simulator $udid ($device, want $px)"
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null
xcrun simctl ui "$udid" appearance dark
xcrun simctl status_bar "$udid" override --time 9:41 --dataNetwork wifi --wifiMode active \
    --wifiBars 3 --cellularMode active --cellularBars 4 --batteryState charged --batteryLevel 100
xcrun simctl install "$udid" "$app"

# ── The shots ───────────────────────────────────────────────────────────

# The seeded session: the runner as a host, tmux on the staged socket.
session=(-MOSHPIT_SEED_USER "$user" -MOSHPIT_SEED_KEY_B64 "$key_b64"
    -MOSHPIT_SEED_HOST "$host" -MOSHPIT_SEED_PORT "$port"
    -MOSHPIT_SEED_TRUST_FP "$fingerprint" -MOSHPIT_SEED_NAME devbox
    -MOSHPIT_SEED_MUX tmux -MOSHPIT_SEED_TMUX_BIN "$stage_tmux")
# The companion, hatched and grown: the golden "tun" (crates/pet/tests/
# golden.json), as its record is stored (crates/store pet.rs, moshpit.pet.v1,
# JSON bytes), passed as plist data in the launch's argument domain, which
# the app reads and never writes.
day=86400; t=$(date +%s)
pet_json="{\"version\":1,\"seed\":\"ca9e919a9b819ab94bb856496e95c9eb\",\"hatched_at\":$((t - 20 * day)),\"hatch_hour\":14,\"name\":\"\",\"name_at\":0,\"finishes\":{\"shots\":40},\"stage\":2,\"stage_at\":[$((t - 20 * day)),$((t - 12 * day)),$((t - 3 * day))],\"flags\":0}"
pet="<$(printf '%s' "$pet_json" | xxd -p | tr -d '\n')>"

# No banners, no prompts, no first-run hints (the pane swipe's toast); the
# companion hatched.
quiet=(-MOSHPIT_SEED_QUIET 1 -MOSHPIT_AUTOCARE_OFF -MoshpitPaneSwipeHintSeen YES
    -moshpit.settings.notifications '<false/>' -moshpit.settings.liveActivity '<false/>'
    -moshpit.pet.v1 "$pet")

# Home: the runner, connected, beside two hosts that aren't.
home_fixture="$(python3 -I - "$key_b64" "$fingerprint" "$port" "$user" "$stage_tmux" "$host" <<'EOF'
import base64, json, sys
key, fingerprint, port, user, tmux, host = sys.argv[1:7]
fixture = {
    "key": key,
    "trust": [{"host": host, "port": int(port), "fingerprint": fingerprint}],
    "connections": [
        {"id": "5EED0000-0000-4000-8000-000000000001", "name": "devbox", "host": host,
         "port": int(port), "user": user, "mux": "tmux", "tmuxPath": tmux, "connect": True},
        {"id": "5EED0000-0000-4000-8000-000000000002", "name": "gpu-01", "host": "gpu-01.example.com",
         "user": "dev", "mosh": True, "mux": "tmux"},
        {"id": "5EED0000-0000-4000-8000-000000000003", "name": "build-box", "host": "build.example.com",
         "user": "ci", "mux": "herdr"},
    ],
}
print(base64.b64encode(json.dumps(fixture).encode()).decode())
EOF
)"

# A store locale's app language (Localizable.xcstrings') and its region.
language() {
    case "$1" in
        zh-Hans|zh-Hant|pt-BR) echo "$1" ;;
        *) echo "${1%%-*}" ;;
    esac
}
region() {
    case "$1" in
        zh-Hans) echo zh_CN ;; zh-Hant) echo zh_TW ;; ja) echo ja_JP ;; ko) echo ko_KR ;;
        *) echo "${1/-/_}" ;;
    esac
}

shoot() {  # <locale> <name> <seconds> <launch args…>
    local locale="$1" name="$2" wait="$3"; shift 3
    mkdir -p "$out/$locale"
    xcrun simctl terminate "$udid" "$bundle" 2>/dev/null || true
    xcrun simctl launch "$udid" "$bundle" -AppleLanguages "($(language "$locale"))" \
        -AppleLocale "$(region "$locale")" "$@" >/dev/null
    sleep "$wait"
    xcrun simctl io "$udid" screenshot --type=png "$out/$locale/$name.png" >/dev/null
    local size
    size="$(python3 -I -c 'import struct,sys; d=open(sys.argv[1],"rb").read(24); print("%dx%d" % struct.unpack(">II", d[16:24]))' "$out/$locale/$name.png")"
    if [ "$size" != "$px" ]; then
        echo "✘ $locale/$name.png is $size, not $px" >&2
        sizes_wrong=1
    else
        echo "  ✓ $locale/$name.png $size"
    fi
}

sizes_wrong=0
for locale in "${locales[@]}"; do
    log "$device · $locale"
    if [ -z "${SHOTS_XCTESTRUN:-}" ]; then
        # Home: long enough to connect to the staged host and poll its
        # agents. With a test bundle, ShotsUITest takes these two, waiting
        # on the page instead of the clock.
        shoot "$locale" 01-home 24 "${quiet[@]}" -MOSHPIT_SEED_HOME_FIXTURE "$home_fixture"
        shoot "$locale" 02-agent-pane 14 "${quiet[@]}" "${session[@]}"
    fi
    if [ "$device" = iphone ] && [ -z "${SHOTS_XCTESTRUN:-}" ]; then
        shoot "$locale" 06-keyboard 18 "${quiet[@]}" "${session[@]}" -MOSHPIT_TERMINAL_DIRECTOR shots
    fi
    shoot "$locale" 09-themes 9 "${quiet[@]}" -MOSHPIT_SEED_SCREEN theme
    shoot "$locale" 12-icons 9 "${quiet[@]}" -MOSHPIT_SEED_SCREEN app-icon
done
xcrun simctl terminate "$udid" "$bundle" 2>/dev/null || true

# ── The shots a page has to be waited for (ShotsUITest) ─────────────────

# SHOTS_XCTESTRUN: the .xctestrun of the workflow's build-for-testing. The
# test waits on each page's id, keeps the screen as an attachment named
# shot~<locale>~<shot>, and the attachments come out of the result bundle
# as <locale>/<shot>.png beside the others.
if [ -n "${SHOTS_XCTESTRUN:-}" ]; then
    log "$device · ShotsUITest (Home's agents, the agent's pane, tmux's windows, the keyboard)"
    specs=""
    for locale in "${locales[@]}"; do specs="$specs $locale=$(language "$locale")=$(region "$locale")"; done
    args_json="$(python3 -I -c 'import json,sys; print(json.dumps(sys.argv[1:]))' "${quiet[@]}")"
    result="$work/shots-$device.xcresult"
    env TEST_RUNNER_SHOTS_LOCALES="${specs# }" TEST_RUNNER_SHOTS_ARGS_JSON="$args_json" \
        TEST_RUNNER_SHOTS_HOME_FIXTURE="$home_fixture" \
        TEST_RUNNER_MOSHPIT_SEED_USER="$user" TEST_RUNNER_MOSHPIT_SEED_KEY_B64="$key_b64" \
        TEST_RUNNER_MOSHPIT_SEED_HOST="$host" TEST_RUNNER_MOSHPIT_SEED_PORT="$port" \
        TEST_RUNNER_MOSHPIT_SEED_TRUST_FP="$fingerprint" TEST_RUNNER_MOSHPIT_SEED_NAME=devbox \
        TEST_RUNNER_MOSHPIT_SEED_MUX=tmux TEST_RUNNER_MOSHPIT_SEED_TMUX_BIN="$stage_tmux" \
        xcodebuild test-without-building -xctestrun "$SHOTS_XCTESTRUN" -destination "id=$udid" \
        -only-testing:MoshpitUITests/ShotsUITest -resultBundlePath "$result" \
        > "$out/shots-ui-$device.log" 2>&1 || echo "✘ ShotsUITest failed: $out/shots-ui-$device.log" >&2
    exported="$work/attachments-$device"
    xcrun xcresulttool export attachments --path "$result" --output-path "$exported" >/dev/null 2>&1 || true
    python3 -I - "$exported" "$out" "$px" <<'PY' || sizes_wrong=1
import json, os, shutil, struct, sys
src, out, px = sys.argv[1:4]
try:
    manifest = json.load(open(os.path.join(src, "manifest.json")))
except OSError:
    sys.exit("✘ ShotsUITest left no attachments")
bad = 0
for test in manifest:
    for attachment in test["attachments"]:
        name = attachment["suggestedHumanReadableName"]
        if not name.startswith("shot~"):
            continue
        _, locale, shot = name.split("~", 2)
        shot = shot.rsplit(".", 1)[0].split("_", 1)[0] if shot.endswith(".png") else shot
        os.makedirs(os.path.join(out, locale), exist_ok=True)
        target = os.path.join(out, locale, shot + ".png")
        shutil.copyfile(os.path.join(src, attachment["exportedFileName"]), target)
        size = "%dx%d" % struct.unpack(">II", open(target, "rb").read(24)[16:24])
        if size != px:
            print(f"✘ {locale}/{shot}.png is {size}, not {px}", file=sys.stderr)
            bad = 1
        else:
            print(f"  ✓ {locale}/{shot}.png {size}")
sys.exit(bad)
PY
fi

# What the app said, for a shot that came out wrong.
xcrun simctl spawn "$udid" log show --last 15m --style compact \
    --predicate 'subsystem BEGINSWITH "com.cluas.moshpit"' > "$out/app-$device.log" 2>/dev/null || true
cp "$work/host/sshd.log" "$out/sshd-$device.log" 2>/dev/null || true

[ "$sizes_wrong" -eq 0 ] || fail "some shots are not $px"
log "done: $out"
