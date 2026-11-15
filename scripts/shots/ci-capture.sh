#!/usr/bin/env bash
# The App Store screenshots, shot on a CI runner (macOS, Xcode 27): staged
# hosts on the runner itself, the app driven to each screen on a simulator
# by ShotsUITest, one PNG per shot, then each composed under its headline
# for the store (scripts/shots/compose.swift, scripts/shots/captions/).
# Run by .github/workflows/screenshots.yml.
#
#   scripts/shots/ci-capture.sh <app> <out> <device> [locale…]
#
#   <app>     the Debug simulator build of Moshpit.app (DEBUG seeds in it)
#   <out>     where the PNGs go, one folder per locale; the store images in
#             <out>/store/<locale>/
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
tmux_gpu="$(seed TMUX_BIN_GPU)"; tmux_build="$(seed TMUX_BIN_BUILD)"

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

# Every shot: no host care, no first-run hints (the pane swipe's toast), the
# companion hatched. The shots inside the app add quiet: no banners (a
# simulator's never goes by itself), no Live Activity. The lock screen and
# the island leave both on (ShotsUITest's loud arguments).
common=(-MOSHPIT_AUTOCARE_OFF -MoshpitPaneSwipeHintSeen YES -moshpit.pet.v1 "$pet")
quiet=(-MOSHPIT_SEED_QUIET 1 -moshpit.settings.notifications '<false/>'
    -moshpit.settings.liveActivity '<false/>')

# Home: the three staged hosts (one sshd under three names, a tmux each),
# all connecting.
home_fixture="$(python3 -I - "$key_b64" "$fingerprint" "$port" "$user" "$host" \
    "$stage_tmux" "$tmux_gpu" "$tmux_build" <<'EOF'
import base64, json, sys
key, fingerprint, port, user, host, tmux, gpu, build = sys.argv[1:9]
hosts = [("devbox", host, tmux), ("gpu-01", "gpu-01.example.com", gpu),
         ("build-box", "build.example.com", build)]
fixture = {
    "key": key,
    "trust": [{"host": h, "port": int(port), "fingerprint": fingerprint} for _, h, _ in hosts],
    "connections": [
        {"id": "5EED0000-0000-4000-8000-00000000000%d" % (i + 1), "name": name, "host": h,
         "port": int(port), "user": user, "mux": "tmux", "tmuxPath": bin, "connect": True}
        for i, (name, h, bin) in enumerate(hosts)
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

[ -n "${SHOTS_XCTESTRUN:-}" ] || fail "the store set is ShotsUITest's: SHOTS_XCTESTRUN (the workflow's build-for-testing)"
sizes_wrong=0

# ── The shots (ShotsUITest) ─────────────────────────────────────────────

# SHOTS_XCTESTRUN: the .xctestrun of the workflow's build-for-testing. The
# test waits on each page's id, keeps the screen as an attachment named
# shot~<locale>~<shot>, and the attachments come out of the result bundle
# as <locale>/<shot>.png.
if [ -n "${SHOTS_XCTESTRUN:-}" ]; then
    log "$device · ShotsUITest (the store set)"
    specs=""
    for locale in "${locales[@]}"; do specs="$specs $locale=$(language "$locale")=$(region "$locale")"; done
    args_json="$(python3 -I -c 'import json,sys; print(json.dumps(sys.argv[1:]))' "${common[@]}")"
    quiet_json="$(python3 -I -c 'import json,sys; print(json.dumps(sys.argv[1:]))' "${quiet[@]}")"
    result="$work/shots-$device.xcresult"
    env TEST_RUNNER_SHOTS_LOCALES="${specs# }" TEST_RUNNER_SHOTS_ARGS_JSON="$args_json" \
        TEST_RUNNER_SHOTS_QUIET_JSON="$quiet_json" \
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
        turned = "x".join(reversed(px.split("x")))
        if size not in (px, turned):
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

# ── The store images ───────────────────────────────────────────────────

# Each shot under its headline (scripts/shots/captions/<locale>.json), at the
# store's size. SHOTS_EXTERNAL: shots taken elsewhere (the lock screen and
# the island, on a phone), in place of the simulator's.
log "$device · the store images (scripts/shots/compose.swift)"
composed=0
xcrun swift "$root/scripts/shots/compose.swift" "$device" "$out" "$out/store" \
    ${SHOTS_EXTERNAL:+--external "$SHOTS_EXTERNAL"} "${locales[@]}" || composed=1

[ "$sizes_wrong" -eq 0 ] || fail "some shots are not $px"
[ "$composed" -eq 0 ] || fail "some store images are missing (above)"
log "done: $out"
