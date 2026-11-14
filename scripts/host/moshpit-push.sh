#!/bin/sh
# moshpit-push.sh — seal one agent status and hand it to the Moshpit push relay.
#
# Installed on the DEV HOST at ~/.moshpit/moshpit-push.sh, next to the stamp
# script that the Vibe Island hooks already write there. The stamp script calls
# this one, fire-and-forget, for the only two states worth waking a phone for:
#
#     attention  — the agent is blocked on you
#     done       — the agent finished its turn
#
# Everything else (`working`, which fires on every tool call) stays on the tmux
# option bridge, where it costs nothing.
#
# Why this exists at all: the tmux bridge only reaches a phone that is running
# Moshpit with a live session. Once iOS suspends the app, the socket dies and no
# amount of local cleverness can wake it. A push can — but only Apple can
# deliver one, and only for a payload signed with the app's team key. So this
# script seals the status with a key the relay does not have, and the relay does
# the one thing it is for: sign, and forward.
#
# Config — written by the app into ~/.moshpit/push.d/<conn>.conf:
#
#     RELAY_URL=https://push.example.org
#     SEND_TOKEN=<64 hex>   bearer credential the relay minted; proves "may
#                           push to that phone" without the relay keeping a
#                           registry — it is an HMAC over the three fields below
#     SECRET=<64 hex>       E2E key; the relay never has it
#     CONN=<uuid>           the phone's own id for this connection
#     APNS_TOKEN=<hex>      where Apple should deliver; rides in each request
#     APNS_ENV=production|sandbox   which APNs host, a hint only
#     SEND_IAT=<unix>       when SEND_TOKEN was minted; tokens expire and the
#                           app rewrites this file well before they do
#
# Usage:
#     moshpit-push.sh <state> <agent> [title]
#     moshpit-push.sh --test [nonce]
#
# --test sends one `done` push labelled with the reserved agent name
# moshpit-selftest. That label is how the app proves a pairing actually works:
# it asks the host to fire one, then waits for a notification carrying the same
# nonce back. A signal that has to travel host -> relay -> Apple -> phone cannot
# be faked by a local check, and the app suppresses it rather than showing the
# user a notification about plumbing. AgentActivityMonitor ignores the label too,
# so proving the install leaves no phantom agent on the island.
#
# Requires: openssl, curl. Nothing else — no jq, no python, no bash. Prints
# nothing and exits 0 on every failure path: it runs inside an agent's hook, and
# a push problem must never become the agent's problem.
#
# WHAT IS VISIBLE TO OTHER LOCAL USERS. openssl takes its keys as command-line
# arguments and offers no file or fd alternative for `enc`, so while this script
# runs, the derived encryption subkey — and the pairing secret it came from —
# appear in this process's argv. On Linux /proc/<pid>/cmdline is world-readable
# by default, so another local user on a SHARED host can capture them by polling
# ps, and with them read or forge this host's notifications. The 0600 on
# push.conf does not help during those milliseconds, and a hook fires often.
#
# The send token is kept out of argv (curl reads it from a private --config
# file below), because that one CAN be protected. The encryption key cannot be,
# portably. On a machine you share with people you would not hand your agent
# transcripts to, either do not pair it or run a kernel with hidepid=2.

set -u

# EVERY paired device gets every push. One `push.conf` used to be the whole
# story, which quietly meant one PHONE per host: a second device pairing
# overwrote the first's secret and its notifications just stopped. Pairings now
# live one file per device under push.d/ (keyed by the device's connection id),
# and the legacy single file is still honored so nothing already installed
# breaks. `MOSHPIT_PUSH_CONF` narrows to a single file for tests.
CONFS=""
if [ -n "${MOSHPIT_PUSH_CONF:-}" ]; then
  [ -r "$MOSHPIT_PUSH_CONF" ] && CONFS="$MOSHPIT_PUSH_CONF"
else
  [ -r "$HOME/.moshpit/push.conf" ] && CONFS="$HOME/.moshpit/push.conf"
  for F in "$HOME/.moshpit/push.d"/*.conf; do
    [ -r "$F" ] && CONFS="$CONFS
$F"
  done
fi
# What a test that found nothing to send with says (B353). Silence read as a
# push lost on the way: the app's verdict blamed delivery (Moshpit not in the
# foreground, notifications off) for a pairing that was not there.
NOPAIRING="this host has no usable pairing to test: pair again from the app"
if [ -z "$CONFS" ]; then
  [ "${1:-}" = "--test" ] && echo "$NOPAIRING"
  exit 0
fi
command -v openssl >/dev/null 2>&1 || exit 0
command -v curl >/dev/null 2>&1 || exit 0

STATE="${1:-}"
AGENT="${2:-agent}"
TITLE="${3:-}"
# Optional: how long the episode this status closes ran, in seconds. The phone
# uses it to decide whether a finished turn is worth a sound. Digits only;
# anything else reads as absent.
DUR="${4:-}"
case "$DUR" in *[!0-9]*) DUR="" ;; esac
# Optional: when the question an `attention` is about was asked, on this
# host's clock — the pane's `@moshpit_since` (B450). It is the question's
# identity on the phone: the pushed copy and the app's own copy of one
# question name the same moment, and a second question on the pane is
# another. Digits only; absent, the send time stands in, as it always did.
EPISODE="${5:-}"
case "$EPISODE" in *[!0-9]*) EPISODE="" ;; esac

TESTMODE=""
TESTCONN=""
# Whether a test reached the relay for any pairing at all.
TRIED=""
if [ "$STATE" = "--test" ]; then
  STATE="done"
  AGENT="moshpit-selftest"
  TITLE="${2:-pairing-selftest}"
  # Optional third argument: prove ONE pairing. The app passes its own
  # connection id so a self-test doesn't ping every other phone in the house
  # (it passed none before B353, and every other phone showed the test).
  TESTCONN="${3:-}"
  TESTMODE="yes"
fi

# Only these two ever justify a push. Guard here as well as in the caller: this
# script is also what a user runs by hand to test, and a stray "working" would
# spend a phone buzz on nothing.
case "$STATE" in
  attention|done) ;;
  *) exit 0 ;;
esac

# The pane this status is about: where a tap lands, and the key of its
# waiting entry and its finish card on the phone. A herdr pane goes by
# herdr's own id (B434). Every push from outside tmux used to name `%0`, a
# pane herdr never has: its card was not taken down once answered, the next
# question inside fifteen minutes came in silent, and every agent on the
# host shared one entry and one finish card. herdr's panes can inherit a
# $TMUX (its server started inside tmux) that knows nothing of them, so in
# a herdr pane tmux is not asked for the session. An id that couldn't be a
# collapse id (the relay's rule) falls back to `%0`.
PANE="${TMUX_PANE:-%0}"
SESSION=""
if [ -n "${HERDR_PANE_ID:-}" ]; then
  PANE="%0"
  case "$HERDR_PANE_ID" in
    *[!A-Za-z0-9:._-]*) ;;
    *) [ ${#HERDR_PANE_ID} -le 24 ] && PANE=$HERDR_PANE_ID ;;
  esac
elif [ -n "${TMUX:-}" ] && command -v tmux >/dev/null 2>&1; then
  SESSION=$(tmux display-message -p "#S" 2>/dev/null || echo "")
fi
HOST="${MOSHPIT_PUSH_HOST:-$(hostname 2>/dev/null || echo host)}"

# --- JSON string escaping ------------------------------------------------------
# Backslash first, then quote — the other order double-escapes. Control
# characters are deleted rather than escaped: they can only have come from a
# terminal title we are about to show on a lock screen, where they mean nothing.
esc() {
  printf '%s' "$1" \
    | tr -d '\000-\037' \
    | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# A lock screen shows ~2 lines. Cut to 120 characters, then let iconv drop any
# UTF-8 sequence the cut split in half — the same treatment the stamp script
# gives its titles, for the same reason (a half character breaks JSON parsers
# and renders as a replacement glyph).
TITLE=$(printf '%s' "$TITLE" | tr '\n\t' '  ' | cut -c1-120)
if command -v iconv >/dev/null 2>&1; then
  TITLE=$(printf '%s' "$TITLE" | iconv -f UTF-8 -t UTF-8 -c 2>/dev/null || printf '%s' "$TITLE")
fi

DURFIELD=""
[ -n "$DUR" ] && [ "$DUR" -gt 0 ] 2>/dev/null && DURFIELD=$(printf ',"dur":%s' "$DUR")
TS=$(date +%s)
[ "$STATE" = "attention" ] && [ -n "$EPISODE" ] && TS=$EPISODE

# The phones this status has already reached, by APNs token. A phone with two
# saved connections to this host holds two pairings here — the app pairs each
# connection on its own, and the second one silently, once the first has
# installed the hooks — and it got every push twice: two cards, two sounds,
# because each connection's card is its own 0->1 edge. One push per phone; the
# next conf only if the previous one did not get through.
DELIVERED=""

# One 0600 scratch file for the bearer token, reused per pairing — it never
# reaches argv where any local user could read it out of /proc. trap BEFORE the
# first write, so a crash between creating and writing still cleans up.
#
# A signal STOPS the script (B350). Trapping INT and TERM with only the `rm`
# used to let it carry on to the next phone, writing that phone's token into a
# file the redirect recreated with the default mode — readable by every local
# user — and a HUP (the pane or session closing under a grace fork) killed it
# with the file left in /tmp. Now each of them exits, which runs the EXIT trap;
# and umask 077 keeps any file this script makes private regardless.
umask 077
AUTHFILE=$(mktemp 2>/dev/null) || exit 0
trap 'rm -f "$AUTHFILE"' EXIT
trap 'exit 0' HUP INT TERM

# Seal and send this status to ONE paired device. Everything conn-specific
# lives in here: the plaintext echoes the pairing's connection id (that is how
# the phone routes a tap), the envelope is sealed with ITS secret, the collapse
# thread carries ITS conn, and the POST bears ITS token. A failure for one
# device must never cost another its notification, so nothing in here exits the
# script in hook mode.
send_one() {
  RELAY_URL=""; SEND_TOKEN=""; SECRET=""; CONN=""; APNS_TOKEN=""; APNS_ENV=""; SEND_IAT=""
  # A conf is READ, never run (B357). It used to be sourced, so whatever a
  # value held ran as shell on this host at every push, and the send token
  # is the relay's own answer: a relay returning `x;curl ... | sh`, or a
  # token with a second line in it, would have run code on every host paired
  # through it. Each known key is taken as text; any other line is ignored.
  while IFS= read -r LINE || [ -n "$LINE" ]; do
    case "$LINE" in
      RELAY_URL=*) RELAY_URL=${LINE#RELAY_URL=} ;;
      SEND_TOKEN=*) SEND_TOKEN=${LINE#SEND_TOKEN=} ;;
      SECRET=*) SECRET=${LINE#SECRET=} ;;
      CONN=*) CONN=${LINE#CONN=} ;;
      APNS_TOKEN=*) APNS_TOKEN=${LINE#APNS_TOKEN=} ;;
      APNS_ENV=*) APNS_ENV=${LINE#APNS_ENV=} ;;
      SEND_IAT=*) SEND_IAT=${LINE#SEND_IAT=} ;;
    esac
  done < "$1" 2>/dev/null || return 0
  [ -n "${RELAY_URL:-}" ] || return 0
  [ -n "${SEND_TOKEN:-}" ] || return 0
  # The token goes into curl's config file below, where a quote or anything
  # but hex could add options of its own: the relay mints 64 hex digits.
  case "$SEND_TOKEN" in *[!0-9a-fA-F]*) return 0 ;; esac
  [ -n "${SECRET:-}" ] || return 0
  # The stateless relay routes by what THIS request carries: the phone's APNs
  # token and the token's mint time ride in the conf. A conf without them is
  # from before the relay stopped keeping a registry — sending would only earn
  # a 401, so skip quietly and let the app rewrite the file on its next
  # connect (it re-checks every conf against its own copy byte for byte).
  [ -n "${APNS_TOKEN:-}" ] || return 0
  case "$SEND_IAT" in *[!0-9]*|"") return 0 ;; esac
  if [ -n "$TESTCONN" ] && [ "$CONN" != "$TESTCONN" ]; then return 0; fi
  PHONE=$(printf '%s' "$APNS_TOKEN" | tr 'ABCDEF' 'abcdef')
  case " $DELIVERED " in *" $PHONE "*) return 0 ;; esac

  PLAIN=$(printf '{"conn":"%s","host":"%s","sess":"%s","pane":"%s","agent":"%s","state":"%s","title":"%s","ts":%s%s}' \
    "$(esc "${CONN:-}")" "$(esc "$HOST")" "$(esc "$SESSION")" "$(esc "$PANE")" \
    "$(esc "$AGENT")" "$STATE" "$(esc "$TITLE")" "$TS" "$DURFIELD")

  # --- seal (format v1; see push-relay/sealbox/sealbox.go) ---------------------
  # The secret is lowercased first: the key derivation is defined over the ASCII
  # of the hex string, so "AB" and "ab" would otherwise derive different keys and
  # the phone would reject its own messages with a MAC error.
  SECRET=$(printf '%s' "$SECRET" | tr 'ABCDEF' 'abcdef')

  # `openssl dgst -hmac <string>` is used rather than `-mac HMAC -macopt hexkey:`
  # because the latter does not exist in the LibreSSL that macOS ships as
  # /usr/bin/openssl. That is also why both subkeys stay in hex-string form.
  KE=$(printf '%s' 'moshpit-push-enc-v1' | openssl dgst -sha256 -hmac "$SECRET" 2>/dev/null | awk '{print $NF}')
  KM=$(printf '%s' 'moshpit-push-mac-v1' | openssl dgst -sha256 -hmac "$SECRET" 2>/dev/null | awk '{print $NF}')
  [ ${#KE} -eq 64 ] || return 0
  [ ${#KM} -eq 64 ] || return 0

  IV=$(openssl rand -hex 16 2>/dev/null) || return 0
  CT=$(printf '%s' "$PLAIN" | openssl enc -aes-256-cbc -K "$KE" -iv "$IV" -a -A 2>/dev/null) || return 0
  [ -n "$CT" ] || return 0
  MAC=$(printf 'v1|%s|%s' "$IV" "$CT" | openssl dgst -sha256 -hmac "$KM" -binary 2>/dev/null | openssl base64 -A 2>/dev/null) || return 0

  # --- route -------------------------------------------------------------------
  # `cat` is the only field the relay reads besides the routing token: it picks
  # the localized fallback text and the interruption level for a phone whose
  # decryption extension times out.
  #
  # `thread` is the collapse id — the SAME string the app uses as its local
  # notification identifier, so a push replaces the local copy instead of
  # stacking. For `attention` it is per CONNECTION: every waiting prompt on a
  # host collapses into ONE summary card ("claude +2"). `done` stays per pane —
  # finishes are per-turn events, not a standing set.
  if [ "$STATE" = "attention" ]; then
    THREAD="moshpit.attention.${CONN:-none}"
  else
    THREAD="moshpit.$STATE.${CONN:-none}.$PANE"
  fi
  # `tok`/`tokEnv`/`conn`/`iat` are the routing facts the relay verifies the
  # bearer token against — it keeps no registry to look them up in. All four
  # were written into this conf by the app, next to the token they were minted
  # with; none of them is content (the conn already rides in `thread`).
  BODY=$(printf '{"env":{"v":1,"iv":"%s","ct":"%s","mac":"%s"},"cat":"%s","thread":"%s","tok":"%s","tokEnv":"%s","conn":"%s","iat":%s}' \
    "$IV" "$CT" "$MAC" "$STATE" "$(esc "$THREAD")" \
    "$(esc "$APNS_TOKEN")" "$(esc "${APNS_ENV:-production}")" "$(esc "${CONN:-}")" "$SEND_IAT")

  printf 'header = "authorization: Bearer %s"\n' "$SEND_TOKEN" > "$AUTHFILE"

  # -m 6 total, no retry: a hook is holding an agent's turn open. Losing a
  # notification is a nuisance; making every tool call wait on a slow network is
  # a broken product.
  #
  # In --test mode the status code is REPORTED — discarding it made the app
  # unable to tell "the relay refused us" from "still in flight". A hook fire
  # stays silent either way: an agent must never see output from this.
  if [ -n "$TESTMODE" ]; then
    TRIED="yes"
    CODE=$(curl -sS -m 6 -o /dev/null -w "%{http_code}" \
      --config "$AUTHFILE" \
      -X POST "$RELAY_URL/v1/notify" \
      -H "content-type: application/json" \
      --data-binary "$BODY" 2>/dev/null)
    case "$CODE" in
      200) DELIVERED="$DELIVERED $PHONE" ;;
      401) echo "relay rejected this host's send token (401): this phone has not registered with $RELAY_URL" ;;
      410) echo "relay says the device is gone (410): pair again from the app" ;;
      429) echo "relay is rate limiting (429): wait a few seconds" ;;
      502) echo "relay could not reach Apple (502)" ;;
      000|"") echo "could not reach $RELAY_URL" ;;
      *) echo "relay answered $CODE" ;;
    esac
  else
    # One retry, hook mode only. "No retry" was designed when this script ran
    # INSIDE an agent's turn; every caller now detaches it (the stamp's grace
    # fork, the backgrounded done push), so a retry costs the agent nothing —
    # and a cold DNS lookup was observed eating the whole 6s budget on the
    # first push after a quiet period, dropping it silently. Only NETWORK
    # failures retry (curl exits non-zero); an HTTP error is the relay's
    # answer and asking again would not change it.
    CODE=""
    for _ in 1 2; do
      CODE=$(curl -sS -m 8 -o /dev/null -w "%{http_code}" \
        --config "$AUTHFILE" \
        -X POST "$RELAY_URL/v1/notify" \
        -H "content-type: application/json" \
        --data-binary "$BODY" 2>/dev/null) && break
      CODE=""
      sleep 2
    done
    [ "$CODE" = "200" ] && DELIVERED="$DELIVERED $PHONE"
  fi
}

IFS='
'
for CONF in $CONFS; do
  send_one "$CONF"
done
unset IFS

if [ -n "$TESTMODE" ] && [ -z "$TRIED" ]; then
  echo "$NOPAIRING"
fi

exit 0
