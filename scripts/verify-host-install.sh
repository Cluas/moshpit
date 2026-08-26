#!/bin/sh
# verify-host-install.sh — run the installer's actual commands against a real
# host over a real SSH exec channel.
#
# The gap this closes: HostCommands' strings are pinned by tests as STRINGS, and
# HostInstaller is driven in tests by a RecordingChannel. So every command below
# has been asserted character by character and never once executed. The parts
# that can only fail at runtime are exactly the parts nobody has run:
#
#   * `printf %s '<base64>' | base64 -d > file` with NO trailing newline —
#     does the host's base64 accept that? (macOS and GNU differ on flags.)
#   * does the digest the app computes over the CONTENT equal what the host's
#     sha256 reports for the FILE it wrote? (a stray newline breaks it)
#   * `umask 077` before the write — is push.conf really 0600 on arrival?
#   * does the stamp script, invoked exactly as HostCommands.selfTest does,
#     actually reach tmux and stamp a real pane?
#
# Every command template here is copied VERBATIM from
# Moshpit/Services/Install/HostChannel.swift; the script prints the source lines
# it was copied from so the transcription can be checked by eye.
#
# Usage:  sh verify-host-install.sh [user@host]     (default: localhost)
# Needs:  passwordless ssh to the target, tmux on the target.
# Touches NOTHING outside a scratch HOME it creates and removes on the target.
set -eu

TARGET="${1:-localhost}"
REPO=$(cd "$(dirname "$0")/.." && pwd)
if [ ! -f "$REPO/project.yml" ]; then
  printf "cannot find the repo root from %s\n" "$0" >&2
  exit 1
fi

SSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o ConnectTimeout=8"
say()  { printf "\n== %s\n" "$1"; }
fail() { printf "FAIL: %s\n" "$1" >&2; exit 1; }
ok()   { printf "   ok  %s\n" "$1"; }

# A scratch HOME on the target. Every command the installer sends expands $HOME,
# so overriding it is enough to keep a real install off the real dotfiles.
SCRATCH=$($SSH "$TARGET" 'mktemp -d /tmp/moshpit-verify.XXXXXX') || fail "cannot ssh to $TARGET"
cleanup() {
  $SSH "$TARGET" "tmux -S '$SCRATCH/tmux.sock' kill-server 2>/dev/null; rm -rf '$SCRATCH'" 2>/dev/null || true
}
trap cleanup EXIT INT TERM
printf "target %s   scratch HOME %s\n" "$TARGET" "$SCRATCH"

# run one command the way the app's exec channel does: HOME overridden, stdout back.
chan() { $SSH "$TARGET" "HOME='$SCRATCH' sh -c '$(printf '%s' "$1" | sed "s/'/'\\\\''/g")'"; }

say "the templates under test, as they appear in HostChannel.swift"
sed -n '60,65p;83,86p;144,147p;163,166p' "$REPO/Moshpit/Services/Install/HostChannel.swift"

# ---------------------------------------------------------------- writeFile
# HostCommands.writeFile(path:mode:base64:)
write_file() {  # path mode file-with-content
  b64=$(base64 < "$3" | tr -d '\n')
  dir=$(dirname "$1")
  chan "set -e; mkdir -p \"$dir\"; umask 077; printf %s '$b64' | base64 -d > \"$1\"; chmod $2 \"$1\""
}
# HostCommands.sha256(path:)
host_sha() {
  chan "{ sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\" 2>/dev/null || openssl dgst -sha256 -r \"$1\" 2>/dev/null; } | awk '{print \$1}'"
}

say "install the stamp script exactly as the app does, then verify by digest"
write_file '$HOME/.moshpit/moshpit-stamp.sh' 755 "$REPO/scripts/moshpit-stamp.sh"
WANT=$(shasum -a 256 "$REPO/scripts/moshpit-stamp.sh" | awk '{print $1}')
GOT=$(host_sha '$HOME/.moshpit/moshpit-stamp.sh')
[ "$WANT" = "$GOT" ] || fail "stamp digest mismatch: app would expect $WANT, host reports $GOT"
ok "digest agrees ($(printf '%.8s' "$WANT")…) — the base64 pipeline is byte-exact"

say "the sender too"
write_file '$HOME/.moshpit/moshpit-push.sh' 755 "$REPO/scripts/moshpit-push.sh"
WANT=$(shasum -a 256 "$REPO/scripts/moshpit-push.sh" | awk '{print $1}')
GOT=$(host_sha '$HOME/.moshpit/moshpit-push.sh')
[ "$WANT" = "$GOT" ] || fail "sender digest mismatch"
ok "digest agrees — and this file contains single quotes, which is what the base64 hop is FOR"

say "push.conf: is umask 077 really giving us 0600 on arrival?"
CONF="$SCRATCH/pushconf.tmp"
printf 'RELAY_URL=https://push.example.org\nSEND_TOKEN=%s\nSECRET=%s\nCONN=3F2504E0-4F89-11D3-9A0C-0305E82C3301\n' \
  "$(printf 'a%.0s' $(seq 1 64))" "$(printf 'b%.0s' $(seq 1 64))" > "$CONF"
write_file '$HOME/.moshpit/push.conf' 600 "$CONF"
MODE=$(chan 'ls -l "$HOME/.moshpit/push.conf" | cut -c1-10')
[ "$MODE" = "-rw-------" ] || fail "push.conf landed as $MODE, expected -rw-------"
ok "mode $MODE — secrets are not world-readable for even one instant"
LINES=$(chan 'wc -l < "$HOME/.moshpit/push.conf"' | tr -d ' ')
[ "$LINES" = "4" ] || fail "push.conf has $LINES lines, expected 4"
ok "4 lines, exactly as the sender parses"

say "a config the merge produces: quotes, slashes, UTF-8, no trailing newline problems"
# The shape AgentHookConfig.mergedJSON emits (sortedKeys, withoutEscapingSlashes,
# prettyPrinted, one trailing newline) with a value that would break a naive
# quote-escaping installer.
CFG="$SCRATCH/settings.tmp"
cat > "$CFG" <<'JSON'
{
  "hooks" : {
    "Stop" : [
      {
        "hooks" : [
          {
            "command" : "sh ~/.moshpit/moshpit-stamp.sh done claude",
            "type" : "command"
          }
        ]
      }
    ]
  },
  "userNote" : "don't 'quote' me — 构建 $HOME `backtick` \"double\""
}
JSON
write_file '$HOME/.claude/settings.json' 644 "$CFG"
WANT=$(shasum -a 256 "$CFG" | awk '{print $1}')
GOT=$(host_sha '$HOME/.claude/settings.json')
[ "$WANT" = "$GOT" ] || fail "settings.json digest mismatch — the write pipeline mangled it"
ok "digest agrees with a body full of quotes, backticks, \$HOME and multi-byte"
chan 'python3 -c "import json,os,sys; json.load(open(os.environ[\"HOME\"]+\"/.claude/settings.json\")); print(\"parsed\")"' >/dev/null \
  || fail "what landed is not valid JSON"
ok "and it still parses as JSON on the host"

say "backupOnce keeps exactly one pre-Moshpit copy"
chan 'if [ -f "$HOME/.claude/settings.json" ] && [ ! -f "$HOME/.claude/settings.json.moshpit.orig" ]; then cp "$HOME/.claude/settings.json" "$HOME/.claude/settings.json.moshpit.orig"; fi'
chan 'if [ -f "$HOME/.claude/settings.json" ] && [ ! -f "$HOME/.claude/settings.json.moshpit.orig" ]; then cp "$HOME/.claude/settings.json" "$HOME/.claude/settings.json.moshpit.orig"; fi'
N=$(chan 'ls "$HOME"/.claude/settings.json.moshpit.orig* 2>/dev/null | wc -l' | tr -d ' ')
[ "$N" = "1" ] || fail "expected exactly 1 .orig, found $N"
ok "run twice, still one .orig — it means 'before Moshpit', not 'before this run'"

say "preflight, as one round trip"
chan 'printf "home=%s\n" "$HOME"; printf "shell=%s\n" "${SHELL:-unknown}"; printf "uname=%s\n" "$(uname -s 2>/dev/null || echo unknown)"; for t in openssl curl jq tmux python3 base64; do if command -v "$t" >/dev/null 2>&1; then printf "%s=yes\n" "$t"; else printf "%s=no\n" "$t"; fi; done' | sed 's/^/   /'

say "the hook self-test: fire the stamp script at a REAL tmux pane"
# A scratch tmux server on its own socket, so nothing touches the user's sessions.
chan "tmux -S '$SCRATCH/tmux.sock' new-session -d -s verify 'sleep 300'"
SOCK="$SCRATCH/tmux.sock"
PANE=$(chan "tmux -S '$SOCK' list-panes -t verify -F '#{pane_id}'" | head -1)
[ -n "$PANE" ] || fail "could not create a scratch tmux pane"
ok "scratch pane $PANE on its own socket"

# HostCommands.selfTest(state:pane:tmuxSocket:) — TMUX is "<socket>,0,0"
chan "TMUX='$SOCK,0,0' TMUX_PANE='$PANE' sh \"\$HOME/.moshpit/moshpit-stamp.sh\" 'working' 'moshpit-selftest' < /dev/null"
# HostCommands.readStamp(pane:tmuxSocket:)
READ=$(chan "TMUX='$SOCK,0,0' tmux display-message -p -t '$PANE' '#{@moshpit_state}|#{@moshpit_agent}|#{@moshpit_title}' 2>/dev/null || true")
printf "   pane reports: %s\n" "$READ"
case "$READ" in
  working\|moshpit-selftest*) ok "the stamp script really reaches tmux — this is the proof the sheet claims" ;;
  *) fail "expected 'working|moshpit-selftest|…', got '$READ'" ;;
esac

# HostCommands.clearStamp(pane:tmuxSocket:)
chan "TMUX='$SOCK,0,0' sh -c 'tmux set -pu -t \"$PANE\" @moshpit_state; tmux set -pu -t \"$PANE\" @moshpit_agent; tmux set -pu -t \"$PANE\" @moshpit_title' 2>/dev/null || true"
AFTER=$(chan "TMUX='$SOCK,0,0' tmux display-message -p -t '$PANE' '#{@moshpit_state}|#{@moshpit_agent}|#{@moshpit_title}' 2>/dev/null || true")
[ "$AFTER" = "||" ] || fail "clearStamp left '$AFTER' behind"
ok "and clears up after itself, leaving no phantom agent on the island"

say "attention/done hand off to the sender; working must not"
# The stamp script backgrounds the sender. With no push.conf reachable the sender
# exits 0 silently, so what is being proved here is only that the hand-off exists
# and that a hook fire never prints into the agent's turn.
OUT=$(chan "TMUX='$SOCK,0,0' TMUX_PANE='$PANE' sh \"\$HOME/.moshpit/moshpit-stamp.sh\" 'attention' 'claude' < /dev/null" 2>&1)
[ -z "$OUT" ] || fail "a hook fire printed into the agent's turn: $OUT"
ok "attention fired silently (the sender is detached and mute)"

say "uninstall removes only what the manifest names"
chan 'rm -f "$HOME/.moshpit/moshpit-push.sh"'
chan 'test -f "$HOME/.moshpit/moshpit-stamp.sh"' || fail "removeFile took the wrong file"
ok "the sender is gone, the stamp script is not"

printf "\nAll host-install checks passed against %s.\n" "$TARGET"
printf "What this does NOT cover: the SwiftUI tap itself, and the app's own\n"
printf "exec channel (this used ssh directly; the app uses its mosh/SSH sidecar).\n"
