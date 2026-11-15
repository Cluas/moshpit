#!/bin/sh
# moshpit-scripts: 1
# moshpit-stamp.sh — what a coding agent's lifecycle hook calls.
#
# Installed on the DEV HOST at ~/.moshpit/moshpit-stamp.sh. Each agent's config
# registers it against four events; every fire stamps the CURRENT tmux pane with
# @moshpit_state / @moshpit_agent / @moshpit_since, so Moshpit reads precise
# agent status over the tmux control channel instead of guessing from output.
#
# Outside tmux (herdr, a plain ssh or mosh shell) there is no pane to stamp,
# but the pushes below still go out (B370): what the pane would hold is kept
# instead in a private directory per agent session, ~/.moshpit/run/s-<id>,
# for the grace check and the finish's duration and prompt to read.
#
# It also hands `attention` and `done` — and only those two — to
# moshpit-push.sh, detached and silenced, so a paired host can wake a phone that
# is not running Moshpit. `working` fires on every tool call and is deliberately
# never pushed.
#
# Two rules this script must never break, because it runs INSIDE an agent's turn:
#   * exit 0 always. A hook that fails is an error the agent surfaces to the user.
#   * never block. The push is `( ... & )`, so a slow network costs the agent
#     nothing.
#
# The title is best-effort: with jq it is derived from the hook's stdin JSON,
# without jq it is skipped and only the state is stamped. While a turn runs it
# is what the agent is doing or asking; on `done` it becomes the PROMPT the turn
# answered (remembered in @moshpit_prompt from UserPromptSubmit), so the finish
# card can say what finished. Whenever no title is derivable it is cleared, so a
# stale one never lingers.
#
# Canonical copy. The installer (crates/install/src/scripts.rs) builds it in
# as it is here; its SHA-256 is how a host's copy is judged current.
ST="$1"; AG="${2:-agent}"; TITLE=""; SID=""; KEEP=""
# That this agent's hooks really fire on this host: Codex runs a new hook only
# once the person trusts it, and the setup screen reads this to know whether
# that step is still to take. Not for a test (MOSHPIT_SELFTEST, Moshpit's own
# fires).
if [ -z "${MOSHPIT_SELFTEST:-}" ] && [ "$AG" != "moshpit-selftest" ]; then
  case "$AG" in
    */*|.*|"") ;;
    *) [ -e "$HOME/.moshpit/seen/$AG" ] || { mkdir -p "$HOME/.moshpit/seen" && : > "$HOME/.moshpit/seen/$AG"; } 2>/dev/null ;;
  esac
fi
# The setup screen's test runs an agent's registered command itself (B380):
# its stamp carries the test's own label, which the island and the push ignore.
[ -z "${MOSHPIT_SELFTEST:-}" ] || AG=moshpit-selftest
if command -v jq >/dev/null 2>&1 && [ ! -t 0 ]; then
  IN=$(cat)
  EV=$(printf "%s" "$IN" | jq -r ".hook_event_name // empty" 2>/dev/null)
  SID=$(printf "%s" "$IN" | jq -r ".session_id // empty" 2>/dev/null)
  case "$EV" in
    PreToolUse|PermissionRequest|BeforeTool)
      TN=$(printf "%s" "$IN" | jq -r ".tool_name // empty" 2>/dev/null)
      AR=$(printf "%s" "$IN" | jq -r ".tool_input.command // .tool_input.file_path // .tool_input.path // .tool_input.dir_path // .tool_input.pattern // .tool_input.url // empty" 2>/dev/null)
      if [ -n "$AR" ]; then TITLE="$TN: $AR"; else TITLE="$TN"; fi ;;
    Notification)
      TITLE=$(printf "%s" "$IN" | jq -r ".message // empty" 2>/dev/null) ;;
    UserPromptSubmit|BeforeAgent)
      TITLE=$(printf "%s" "$IN" | jq -r ".prompt // empty" 2>/dev/null) ;;
  esac
  TITLE=$(printf "%s" "$TITLE" | tr "\n" " " | cut -c1-80)
  command -v iconv >/dev/null 2>&1 && TITLE=$(printf "%s" "$TITLE" | iconv -f UTF-8 -t UTF-8 -c 2>/dev/null)
  # Claude Code routes its own injected turns through UserPromptSubmit too:
  # a background task finishing (<task-notification>), a teammate or another
  # session writing in, a slash command's local output, an interrupt marker.
  # Nobody typed those, and a finish card that quoted one read
  # "<task-notification> <task-id>…" on a real lock screen. Drop the text and
  # keep the prompt the person actually gave — that is still what this work is.
  if [ "$EV" = "UserPromptSubmit" ]; then
    LEAD=${TITLE#"${TITLE%%[! ]*}"}
    case "$LEAD" in
      "<task-notification>"*|"<teammate-message"*|"<cross-session-message"*|\
      "<system-reminder>"*|"<local-command-"*|"<command-name>"*|\
      "[SYSTEM NOTIFICATION"*|"[Request interrupted"*) TITLE="" ;;
    esac
  fi
  # Remember the prompt for this turn's `done`. Only the prompt: a tool line
  # would overwrite it on the very next PreToolUse, and a Stop hook brings no
  # text of its own. Already cut to 80 bytes above — a prompt can be a
  # paragraph, and a lock screen shows two lines (the phone cuts again, by
  # character, before showing it).
  # (Gemini CLI calls the same moment BeforeAgent.)
  if { [ "$EV" = "UserPromptSubmit" ] || [ "$EV" = "BeforeAgent" ]; } && [ -n "$TITLE" ]; then
    KEEP=1
  fi
fi
# Where the stamps live: the tmux pane's options, or — outside tmux — files
# of the agent session (by its session id when jq could read one, else by its
# herdr pane, else by the agent), private, since a prompt is among them. A
# herdr pane keeps its stamps outside tmux even with a $TMUX inherited from
# where herdr's server started (B434): that tmux has no such pane, and its
# stamps landed on whichever pane the variable named.
if [ -n "${TMUX:-}" ] && [ -z "${HERDR_PANE_ID:-}" ]; then
  get() { tmux display-message -p -t "$TMUX_PANE" "#{@moshpit_$1}" 2>/dev/null; }
  put() { tmux set -p -t "$TMUX_PANE" "@moshpit_$1" "$2" 2>/dev/null; }
  del() { tmux set -pu -t "$TMUX_PANE" "@moshpit_$1" 2>/dev/null; }
  prev() { tmux display-message -p -t "$TMUX_PANE" "#{@moshpit_state}|#{@moshpit_since}" 2>/dev/null; }
else
  umask 077
  RUN="$HOME/.moshpit/run/s-$(printf '%s' "${SID:-${HERDR_PANE_ID:-$AG}}" | tr -c 'A-Za-z0-9._-' '_' | cut -c1-64)"
  mkdir -p "$RUN" 2>/dev/null || exit 0
  get() { cat "$RUN/$1" 2>/dev/null; }
  put() { printf '%s' "$2" > "$RUN/$1" 2>/dev/null; }
  del() { rm -f "$RUN/$1" 2>/dev/null; }
  prev() { printf '%s|%s' "$(get state)" "$(get since)"; }
fi
[ -n "$KEEP" ] && put prompt "$TITLE"
# What the pane was doing BEFORE this stamp, read in one round trip. Two things
# hang off it below: telling a real question apart from an idle nag, and
# measuring how long a finished turn ran.
PREV=$(prev)
PREV_ST=${PREV%%|*}
PREV_SINCE=${PREV#*|}

# A PARKED agent is not asking anything. Claude Code fires its Notification hook
# both for real blocking questions (a permission prompt, mid-turn) and for idle
# reminders ("Claude is waiting for your input") on a pane the user deliberately
# left at the prompt — and the user told us plainly which of those is noise:
# an agent parked on purpose must not light up phones and islands as NEEDS YOU.
#
# The reliable, language-independent tell is the TRANSITION: a real question can
# only interrupt a turn, so it arrives on a pane that was `working` (or already
# `attention`, when a second prompt replaces the first). An idle reminder
# arrives on a pane whose turn already ENDED — prior state `done`. The message
# text is a second, Claude-specific belt for panes with no prior stamp at all.
if [ "$ST" = "attention" ]; then
  IDLE=""
  case "$TITLE" in *"waiting for your input"*) IDLE=1 ;; esac
  case "$PREV_ST" in working|attention) ;; done) IDLE=1 ;; esac
  if [ -n "$IDLE" ]; then
    # Not just dropped — HEALED. A pane still marked `attention` when an idle
    # reminder arrives is a fossil (an older stamp, or a question that stopped
    # existing): the agent is at its prompt, which is what `done` means. Left
    # alone it stands forever — a real phone showed NEEDS YOU frozen for 24
    # hours — because this branch used to exit without touching it and nothing
    # else ever would. Silent: no push, no title; just the truth.
    if [ "$PREV_ST" = "attention" ]; then
      put state done
      put since "$(date +%s)"
      del title
    fi
    exit 0
  fi
fi

NOW=$(date +%s)
put state "$ST"
put agent "$AG"
# `@moshpit_since` marks when the pane ENTERED its state — the episode's
# identity — so it moves only when the state does. It used to be overwritten on
# every fire, which made "how long did this run" unanswerable: PostToolUse
# stamps `working` per tool call, so by `done` the timestamp was the last tool's,
# not the turn's. (The app already kept its own stable copy for the same reason;
# now the host agrees with it.)
if [ "$PREV_ST" != "$ST" ]; then
  put since "$NOW"
else
  case "$PREV_SINCE" in *[!0-9]*|"") put since "$NOW" ;; esac
fi
# The episode id the grace fork below must match: the standing since.
EPISODE=$NOW
if [ "$PREV_ST" = "$ST" ]; then
  case "$PREV_SINCE" in *[!0-9]*|"") ;; *) EPISODE=$PREV_SINCE ;; esac
fi
# A `done` carries the prompt this turn answered: "what finished" is the one
# thing a finish card is for, and the Stop event itself says nothing.
if [ "$ST" = "done" ]; then
  TITLE=$(get prompt)
fi
if [ -n "$TITLE" ]; then
  put title "$TITLE"
else
  del title
fi
case "$ST" in
  attention)
    # GRACE before the push: a question answered at the desk within this window
    # never reaches a phone. The fork sleeps, then re-reads the pane — same
    # state AND same episode (`@moshpit_since` unchanged) — before sending.
    # Detached, so the agent's turn is never held. Overridable for tests.
    #
    # The sender runs through `sh`, as the hooks run this script (B358): a
    # home mounted noexec fails `-x` and refuses to exec it, and every push
    # was dropped without a word while the setup screen's push test, which
    # runs it with `sh`, said all was well.
    if [ -f "$HOME/.moshpit/moshpit-push.sh" ]; then
      GRACE=${MOSHPIT_NOTIFY_GRACE:-30}
      (
        sleep "$GRACE"
        CUR=$(prev)
        [ "$CUR" = "attention|$EPISODE" ] || exit 0
        exec sh "$HOME/.moshpit/moshpit-push.sh" attention "$AG" "$TITLE" "" "$EPISODE"
      ) >/dev/null 2>&1 &
    fi
    ;;
  done)
    # A finish pushes immediately — but carries how long the turn RAN, so the
    # phone can tell a three-minute build (worth a chime) from a twenty-second
    # answer (list-only, silent). Only a turn measured from `working` counts;
    # anything else sends no duration and reads as short.
    if [ -f "$HOME/.moshpit/moshpit-push.sh" ]; then
      DUR=0
      if [ "$PREV_ST" = "working" ] || [ "$PREV_ST" = "attention" ]; then
        case "$PREV_SINCE" in
          *[!0-9]*|"") ;;
          *) [ "$NOW" -ge "$PREV_SINCE" ] && DUR=$((NOW - PREV_SINCE)) ;;
        esac
      fi
      ( sh "$HOME/.moshpit/moshpit-push.sh" done "$AG" "$TITLE" "$DUR" >/dev/null 2>&1 & )
    fi
    ;;
esac

exit 0
