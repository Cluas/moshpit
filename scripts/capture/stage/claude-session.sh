#!/bin/sh
# A Claude Code turn, replayed for capture.
#
# Why replayed and not live: a real `claude` in this pane renders the operator's
# own account state — "Welcome back <name>", login-expiry and MCP warnings, the
# effort/mode footer. That is personal data in a marketing screenshot and it
# changes shape between runs, which makes captures irreproducible.
#
# What is real: everything OUTSIDE the pane. The Agents section, the breadcrumb,
# the state dots, the sheets — those are the app reading a live herdr server.
# This file only fills the pane with the thing the app exists to interrupt: a
# tool call waiting on a human. Keep it faithful to what Claude Code actually
# prints; if its prompt changes, change this.
printf '\033[H\033[2J\033[3J'
G='\033[38;5;114m'; D='\033[38;5;245m'; W='\033[38;5;180m'; B='\033[1m'; R='\033[0m'
C='\033[38;5;110m'

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b claude\n\n" \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"
printf "%b> %bthe webhook retry gives up too fast — make it back off and cap the wait%b\n\n" "$D" "$R" "$R"
printf "%b● %bI'll make the retry helper back off exponentially and cap the wait.%b\n\n" "$G" "$R" "$R"
printf "%b● %bSearch(pattern: \"retry\", path: \"src\")%b\n" "$C" "$B" "$R"
printf "  %b⎿  4 files%b\n" "$D" "$R"
printf "     %bsrc/retry.js%b\n" "$D" "$R"
printf "     %bsrc/webhook.js%b\n" "$D" "$R"
printf "     %bsrc/queue.js%b\n" "$D" "$R"
printf "     %btest/retry.test.js%b\n\n" "$D" "$R"
printf "%b● %bRead(retry.js)%b\n" "$C" "$B" "$R"
printf "  %b⎿  6 lines%b\n\n" "$D" "$R"
printf "%b● %bRead(webhook.js)%b\n" "$C" "$B" "$R"
printf "  %b⎿  84 lines%b\n\n" "$D" "$R"
printf "%b● %bIt retries three times with no delay, so a rate-limited endpoint\n" "$G" "$R"
printf "  gets three requests inside a second and then the event is dropped.\n"
printf "  I'll add exponential backoff with jitter, capped at 30s.%b\n\n" "$R"
printf "%b● %bWrite(backoff.js)%b\n" "$C" "$B" "$R"
printf "  %b⎿  24 lines%b\n\n" "$D" "$R"
printf "%b● %bUpdate(retry.js)%b\n" "$C" "$B" "$R"
printf "  %b⎿  Updating 1 addition and 1 removal%b\n\n" "$D" "$R"
printf "     %b3%b  %b-   return fn();%b\n" "$D" "$R" "\033[38;5;174m" "$R"
printf "     %b3%b  %b+   return backoff(fn, times, { capMs: 30_000 });%b\n\n" "$D" "$R" "$G" "$R"
printf "%bDo you want to make this edit to retry.js?%b\n" "$W" "$R"
printf "%b❯ 1. Yes%b\n" "$B" "$R"
printf "  %b2. Yes, allow all edits this session%b\n" "$D" "$R"
printf "  %b3. No, tell Claude what to do differently%b\n" "$D" "$R"
# Hold the frame: the capture wants this screen, and a returning shell prompt
# would scroll it away.
while :; do sleep 3600; done
