#!/bin/sh
# moshpit-stamp.sh — what a coding agent's lifecycle hook calls.
#
# Installed on the DEV HOST at ~/.moshpit/moshpit-stamp.sh. Each agent's config
# registers it against four events; every fire stamps the CURRENT tmux pane with
# @moshpit_state / @moshpit_agent / @moshpit_since, so Moshpit reads precise
# agent status over the tmux control channel instead of guessing from output.
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
# without jq it is skipped and only the state is stamped. It is cleared on `done`
# and whenever no fresh title is derivable, so a stale one never lingers.
#
# Canonical copy. Moshpit/Services/Install/HostScripts.swift carries a
# byte-identical literal (the app cannot read a repo file on a phone);
# scripts/gen-host-scripts.py regenerates it and a test fails if they drift.
[ -n "$TMUX" ] || exit 0
ST="$1"; AG="${2:-agent}"; TITLE=""
if command -v jq >/dev/null 2>&1 && [ ! -t 0 ]; then
  IN=$(cat)
  EV=$(printf "%s" "$IN" | jq -r ".hook_event_name // empty" 2>/dev/null)
  case "$EV" in
    PreToolUse|PermissionRequest)
      TN=$(printf "%s" "$IN" | jq -r ".tool_name // empty" 2>/dev/null)
      AR=$(printf "%s" "$IN" | jq -r ".tool_input.command // .tool_input.file_path // .tool_input.path // .tool_input.pattern // .tool_input.url // empty" 2>/dev/null)
      if [ -n "$AR" ]; then TITLE="$TN: $AR"; else TITLE="$TN"; fi ;;
    Notification)
      TITLE=$(printf "%s" "$IN" | jq -r ".message // empty" 2>/dev/null) ;;
    UserPromptSubmit)
      TITLE=$(printf "%s" "$IN" | jq -r ".prompt // empty" 2>/dev/null) ;;
  esac
  TITLE=$(printf "%s" "$TITLE" | tr "\n" " " | cut -c1-80)
  command -v iconv >/dev/null 2>&1 && TITLE=$(printf "%s" "$TITLE" | iconv -f UTF-8 -t UTF-8 -c 2>/dev/null)
fi
# What the pane was doing BEFORE this stamp, read in one round trip. Two things
# hang off it below: telling a real question apart from an idle nag, and
# measuring how long a finished turn ran.
PREV=$(tmux display-message -p -t "$TMUX_PANE" "#{@moshpit_state}|#{@moshpit_since}" 2>/dev/null)
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
      tmux set -p -t "$TMUX_PANE" @moshpit_state done 2>/dev/null
      tmux set -p -t "$TMUX_PANE" @moshpit_since "$(date +%s)" 2>/dev/null
      tmux set -pu -t "$TMUX_PANE" @moshpit_title 2>/dev/null
    fi
    exit 0
  fi
fi

NOW=$(date +%s)
tmux set -p -t "$TMUX_PANE" @moshpit_state "$ST" 2>/dev/null
tmux set -p -t "$TMUX_PANE" @moshpit_agent "$AG" 2>/dev/null
# `@moshpit_since` marks when the pane ENTERED its state — the episode's
# identity — so it moves only when the state does. It used to be overwritten on
# every fire, which made "how long did this run" unanswerable: PostToolUse
# stamps `working` per tool call, so by `done` the timestamp was the last tool's,
# not the turn's. (The app already kept its own stable copy for the same reason;
# now the host agrees with it.)
if [ "$PREV_ST" != "$ST" ]; then
  tmux set -p -t "$TMUX_PANE" @moshpit_since "$NOW" 2>/dev/null
else
  case "$PREV_SINCE" in *[!0-9]*|"") tmux set -p -t "$TMUX_PANE" @moshpit_since "$NOW" 2>/dev/null ;; esac
fi
# The episode id the grace fork below must match: the standing since.
EPISODE=$NOW
if [ "$PREV_ST" = "$ST" ]; then
  case "$PREV_SINCE" in *[!0-9]*|"") ;; *) EPISODE=$PREV_SINCE ;; esac
fi
if [ -n "$TITLE" ] && [ "$ST" != "done" ]; then
  tmux set -p -t "$TMUX_PANE" @moshpit_title "$TITLE" 2>/dev/null
else
  tmux set -pu -t "$TMUX_PANE" @moshpit_title 2>/dev/null
fi
case "$ST" in
  attention)
    # GRACE before the push: a question answered at the desk within this window
    # never reaches a phone. The fork sleeps, then re-reads the pane — same
    # state AND same episode (`@moshpit_since` unchanged) — before sending.
    # Detached, so the agent's turn is never held. Overridable for tests.
    if [ -x "$HOME/.moshpit/moshpit-push.sh" ]; then
      GRACE=${MOSHPIT_NOTIFY_GRACE:-30}
      (
        sleep "$GRACE"
        CUR=$(tmux display-message -p -t "$TMUX_PANE" "#{@moshpit_state}|#{@moshpit_since}" 2>/dev/null)
        [ "$CUR" = "attention|$EPISODE" ] || exit 0
        exec "$HOME/.moshpit/moshpit-push.sh" attention "$AG" "$TITLE"
      ) >/dev/null 2>&1 &
    fi
    ;;
  done)
    # A finish pushes immediately — but carries how long the turn RAN, so the
    # phone can tell a three-minute build (worth a chime) from a twenty-second
    # answer (list-only, silent). Only a turn measured from `working` counts;
    # anything else sends no duration and reads as short.
    if [ -x "$HOME/.moshpit/moshpit-push.sh" ]; then
      DUR=0
      if [ "$PREV_ST" = "working" ] || [ "$PREV_ST" = "attention" ]; then
        case "$PREV_SINCE" in
          *[!0-9]*|"") ;;
          *) [ "$NOW" -ge "$PREV_SINCE" ] && DUR=$((NOW - PREV_SINCE)) ;;
        esac
      fi
      ( "$HOME/.moshpit/moshpit-push.sh" done "$AG" "$TITLE" "$DUR" >/dev/null 2>&1 & )
    fi
    ;;
esac

exit 0
