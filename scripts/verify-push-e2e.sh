#!/bin/sh
# verify-push-e2e.sh — exercise the whole remote-notification chain on a laptop,
# with no Apple credentials and no phone.
#
#   hook args -> seal (openssl) -> HTTP -> relay -> APNs payload
#
# It covers the three seams the unit tests cannot:
#
#   1. The sender talking to a real relay process over real HTTP, including the
#      relay's JSON validation and its bearer-token lookup.
#   2. That the envelope survives the relay byte-for-byte and still decrypts.
#   3. That the script as INSTALLED is valid `sh` and works. Nothing is minified
#      or pasted any more — HostInstaller delivers the file verbatim over an SSH
#      exec channel — so what runs here is byte-for-byte what runs on a host. The
#      iOS unit tests cannot run a shell at all.
#
# Usage:  scripts/verify-push-e2e.sh
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
PORT=${PORT:-18777}
RELAY_PID=""

cleanup() {
  [ -n "$RELAY_PID" ] && kill "$RELAY_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

say() { printf "\n== %s\n" "$1"; }
fail() { printf "FAIL: %s\n" "$1" >&2; exit 1; }

SECRET=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
SEND_TOKEN=aaaabbbbccccddddeeeeffff00001111aaaabbbbccccddddeeeeffff00001111
CONN=3F2504E0-4F89-11D3-9A0C-0305E82C3301
# sha256 of SEND_TOKEN, the only form the relay ever holds.
HASH=$(printf "%s" "$SEND_TOKEN" | openssl dgst -sha256 | awk '{print $NF}')

say "go vet + go test"
(cd "$ROOT/push-relay" && go vet ./... && go test ./...) || fail "relay tests"

say "build + start the relay in dry-run mode"
(cd "$ROOT/push-relay" && go build -o "$TMP/relay" .) || fail "build"
MOSHPIT_RELAY_DRY_RUN=1 \
MOSHPIT_RELAY_ADDR=":$PORT" \
MOSHPIT_RELAY_STATE="$TMP/devices.json" \
  "$TMP/relay" > "$TMP/relay.log" 2>&1 &
RELAY_PID=$!

# Wait for the listener rather than sleeping a guessed interval.
i=0
while [ $i -lt 50 ]; do
  if curl -fsS "http://127.0.0.1:$PORT/healthz" > /dev/null 2>&1; then break; fi
  i=$((i + 1))
  sleep 0.1
done
curl -fsS "http://127.0.0.1:$PORT/healthz" > /dev/null 2>&1 || fail "relay never came up: $(cat "$TMP/relay.log")"

say "register a device"
curl -fsS -X POST "http://127.0.0.1:$PORT/v1/register" \
  -H "content-type: application/json" \
  -d "{\"apnsToken\":\"$(printf 'ab%.0s' $(seq 1 32))\",\"sendTokenHash\":\"$HASH\",\"env\":\"sandbox\"}" \
  > /dev/null || fail "register rejected"

say "send one attention through the real sender script"
cat > "$TMP/push.conf" <<CONF
RELAY_URL=http://127.0.0.1:$PORT
SEND_TOKEN=$SEND_TOKEN
SECRET=$SECRET
CONN=$CONN
CONF
TITLE='Bash: rm -rf "build" \ 构建'
MOSHPIT_PUSH_CONF="$TMP/push.conf" MOSHPIT_PUSH_HOST=m1-pro TMUX_PANE=%3 TMUX= \
  sh "$ROOT/scripts/moshpit-push.sh" attention claude "$TITLE" \
  || fail "sender exited non-zero"

# The dry-run sender logs the exact payload it would have handed to APNs.
grep -q "DRY RUN push" "$TMP/relay.log" || fail "relay logged no push: $(cat "$TMP/relay.log")"
PAYLOAD=$(grep "DRY RUN push" "$TMP/relay.log" | tail -1 | sed -e 's/^.*payload=//')

say "check the payload the relay built"
echo "$PAYLOAD" | grep -q '"mutable-content":1' \
  || fail "no mutable-content: the decrypting extension would never run"
# No category on the wire: only a push the extension has opened may carry the
# Allow/Deny actions, because only then are the pane ids there to act on.
echo "$PAYLOAD" | grep -q '"category"' \
  && fail "the payload carries a category — an unopened push would offer dead buttons"
echo "$PAYLOAD" | grep -q '"interruption-level":"time-sensitive"' \
  || fail "interruption level not set"
echo "$PAYLOAD" | grep -q 'An agent needs you' \
  || fail "fallback loc-key missing"
# The relay must not have leaked any plaintext into the visible alert.
echo "$PAYLOAD" | grep -q 'rm -rf' \
  && fail "the relay payload contains plaintext from the agent"
grep "DRY RUN push" "$TMP/relay.log" | tail -1 | grep -q "collapse=moshpit.attention.$CONN.%3" \
  || fail "collapse id is not the app's local notification identifier"

say "open the envelope that came out the other side"
IV=$(printf "%s" "$PAYLOAD" | sed -e 's/.*"iv":"//' -e 's/".*//')
CT=$(printf "%s" "$PAYLOAD" | sed -e 's/.*"ct":"//' -e 's/".*//')
MAC=$(printf "%s" "$PAYLOAD" | sed -e 's/.*"mac":"//' -e 's/".*//')
[ -n "$IV" ] && [ -n "$CT" ] && [ -n "$MAC" ] || fail "envelope fields missing from the payload"

KE=$(printf "%s" 'moshpit-push-enc-v1' | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $NF}')
KM=$(printf "%s" 'moshpit-push-mac-v1' | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $NF}')
WANT_MAC=$(printf 'v1|%s|%s' "$IV" "$CT" | openssl dgst -sha256 -hmac "$KM" -binary | openssl base64 -A)
[ "$MAC" = "$WANT_MAC" ] || fail "MAC does not verify after the round trip"
PLAIN=$(printf "%s" "$CT" | openssl enc -d -aes-256-cbc -K "$KE" -iv "$IV" -a -A) \
  || fail "decrypt failed"

printf "   plaintext: %s\n" "$PLAIN"
echo "$PLAIN" | grep -q "\"conn\":\"$CONN\"" || fail "conn missing (the app could not route this)"
echo "$PLAIN" | grep -q '"host":"m1-pro"' || fail "host wrong"
echo "$PLAIN" | grep -q '"pane":"%3"' || fail "pane wrong"
echo "$PLAIN" | grep -q '"state":"attention"' || fail "state wrong"
echo "$PLAIN" | grep -q '构建' || fail "multi-byte title did not survive"
echo "$PLAIN" | grep -q 'rm -rf \\"build\\"' || fail "quoted title did not survive JSON escaping"

say "working must never be pushed"
BEFORE=$(grep -c "DRY RUN push" "$TMP/relay.log")
MOSHPIT_PUSH_CONF="$TMP/push.conf" MOSHPIT_PUSH_HOST=m1-pro TMUX_PANE=%3 TMUX= \
  sh "$ROOT/scripts/moshpit-push.sh" working claude "reading a file"
AFTER=$(grep -c "DRY RUN push" "$TMP/relay.log")
[ "$BEFORE" = "$AFTER" ] || fail "a working state was pushed"

say "an unpaired host is a silent no-op"
OUT=$(MOSHPIT_PUSH_CONF="$TMP/absent.conf" sh "$ROOT/scripts/moshpit-push.sh" attention claude x 2>&1)
[ -z "$OUT" ] || fail "unpaired host printed something into an agent hook: $OUT"

say "a bad send token is refused"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "http://127.0.0.1:$PORT/v1/notify" \
  -H "authorization: Bearer deadbeef" -H "content-type: application/json" \
  -d '{"env":{"v":1,"iv":"00","ct":"x","mac":"y"},"cat":"done","thread":"t"}')
[ "$CODE" = "401" ] || fail "unknown send token got $CODE, want 401"

say "the sender as INSTALLED - whole file, comments and all - still works"
# The installer delivers this file verbatim over an exec channel, so what runs
# on a host is exactly what is in the repo. The minified variant this section
# used to check went away with the paste-a-command flow: nothing strips comments
# any more, and a user who looks at ~/.moshpit/ finds a readable program.
BEFORE=$(grep -c "DRY RUN push" "$TMP/relay.log")
sleep 3   # the relay enforces a 3s floor between pushes per host
MOSHPIT_PUSH_CONF="$TMP/push.conf" MOSHPIT_PUSH_HOST=m1-pro TMUX_PANE=%3 TMUX= \
  sh "$ROOT/scripts/moshpit-push.sh" done claude "finished" || fail "sender failed"
AFTER=$(grep -c "DRY RUN push" "$TMP/relay.log")
[ "$AFTER" -gt "$BEFORE" ] || fail "the done push was not sent"
grep "DRY RUN push" "$TMP/relay.log" | tail -1 | grep -q "collapse=moshpit.done.$CONN.%3" \
  || fail "done collapse id wrong"

say "the self-test carries the nonce the app matches on"
sleep 3
MOSHPIT_PUSH_CONF="$TMP/push.conf" MOSHPIT_PUSH_HOST=m1-pro TMUX_PANE=%3 TMUX= \
  sh "$ROOT/scripts/moshpit-push.sh" --test "selftest-abc123" || fail "self-test failed"
PAYLOAD2=$(grep "DRY RUN push" "$TMP/relay.log" | tail -1 | sed -e 's/^.*payload=//')
IV2=$(printf "%s" "$PAYLOAD2" | sed -e 's/.*"iv":"//' -e 's/".*//')
CT2=$(printf "%s" "$PAYLOAD2" | sed -e 's/.*"ct":"//' -e 's/".*//')
PLAIN2=$(printf "%s" "$CT2" | openssl enc -d -aes-256-cbc -K "$KE" -iv "$IV2" -a -A) \
  || fail "self-test decrypt failed"
echo "$PLAIN2" | grep -q '"agent":"moshpit-selftest"' \
  || fail "the self-test must use the reserved agent label the app filters on"
echo "$PLAIN2" | grep -q '"title":"selftest-abc123"' \
  || fail "the nonce did not survive into the sealed status"

say "the host scripts and their Swift copies have not drifted"
python3 - "$ROOT" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
swift = (root / "Moshpit/Services/Install/HostScripts.swift").read_text()
for name, rel in [("stamp", "scripts/moshpit-stamp.sh"), ("sender", "scripts/moshpit-push.sh")]:
    marker = f'    static let {name} = #"""\n'
    start = swift.index(marker) + len(marker)
    end = swift.index('\n"""#', start)
    if swift[start:end] != (root / rel).read_text().rstrip("\n"):
        sys.exit(f"HostScripts.{name} has drifted - run scripts/gen-host-scripts.py")
print("   both identical")
PY

printf "\nAll push e2e checks passed.\n"
