#!/bin/sh
# A test run, replayed to fill a terminal pane for capture.
#
# Why this exists: the staged panes used to be seeded with `git log -6`, which
# puts six lines into a thirty-four row pane. A design review measured the
# resulting screenshots at 0.57%–9.91% ink inside the terminal area — a phone
# frame containing a few lines and a lot of black, printed on a page directly
# under the claim that every screen is the shipping app. The frame was honest;
# the emptiness made it look like a rendering failure.
#
# So: a plausible, dense turn of output that fills the pane top to bottom.
# Same rule as claude-session.sh — everything OUTSIDE the pane is a live app
# reading a live server; this only decides what the shell was doing.
printf '\033[H\033[2J\033[3J'
G='\033[38;5;114m'; D='\033[38;5;245m'; R='\033[0m'; C='\033[38;5;110m'
Y='\033[38;5;180m'; RD='\033[38;5;174m'; B='\033[1m'

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b npm test -- --run\n\n" \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"

printf "%b RUN %b v2.1.4  %b/srv/payments-api%b\n\n" "$D" "$R" "$D" "$R"

printf " %b✓%b src/retry.test.js %b(9 tests)%b %b412ms%b\n"   "$G" "$R" "$D" "$R" "$D" "$R"
printf " %b✓%b src/webhook.test.js %b(14 tests)%b %b806ms%b\n" "$G" "$R" "$D" "$R" "$D" "$R"
printf " %b✓%b src/signing.test.js %b(6 tests)%b %b118ms%b\n"  "$G" "$R" "$D" "$R" "$D" "$R"
printf " %b✓%b src/queue.test.js %b(11 tests)%b %b1.20s%b\n"   "$G" "$R" "$D" "$R" "$D" "$R"
printf " %b✓%b src/idempotency.test.js %b(8 tests)%b %b284ms%b\n" "$G" "$R" "$D" "$R" "$D" "$R"
printf " %b✓%b src/backoff.test.js %b(12 tests)%b %b96ms%b\n"  "$G" "$R" "$D" "$R" "$D" "$R"
printf " %b✓%b test/e2e/replay.test.js %b(4 tests)%b %b2.31s%b\n" "$G" "$R" "$D" "$R" "$D" "$R"
printf "\n"

printf " %bTest Files%b  7 passed %b(7)%b\n"   "$D" "$R" "$D" "$R"
printf " %b     Tests%b  64 passed %b(64)%b\n" "$D" "$R" "$D" "$R"
printf " %b  Start at%b  09:41:12\n"           "$D" "$R"
printf " %b  Duration%b  5.24s %b(transform 288ms, setup 0ms, collect 1.02s)%b\n\n" \
  "$D" "$R" "$D" "$R"

printf "%b ✓ %b%b64 passing%b — backoff now caps at 30s\n\n" "$G" "$R" "$B" "$R"

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b git diff --stat\n" \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"
printf " src/retry.js            %b|%b 18 %b+++++++++++%b%b-------%b\n" "$D" "$R" "$G" "$R" "$RD" "$R"
printf " src/backoff.js          %b|%b 24 %b++++++++++++++++++++%b%b----%b\n" "$D" "$R" "$G" "$R" "$RD" "$R"
printf " src/backoff.test.js     %b|%b 41 %b+++++++++++++++++++++++++++++++++++++++++%b\n" "$D" "$R" "$G" "$R"
printf " 3 files changed, 71 insertions(+), 12 deletions(-)\n\n"

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b git log --oneline --graph -8\n" \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"
printf "%b*%b %be3f1a9c%b cap backoff at 30s, with a test\n"        "$C" "$R" "$Y" "$R"
printf "%b*%b %b7b20d14%b jitter the retry window\n"                "$C" "$R" "$Y" "$R"
printf "%b*%b %b1c8ef03%b pull retry policy out of the handler\n"   "$C" "$R" "$Y" "$R"
printf "%b*%b %b9a4d772%b log the attempt number on give-up\n"      "$C" "$R" "$Y" "$R"
printf "%b*%b %b40b6e18%b idempotency key on replayed webhooks\n"   "$C" "$R" "$Y" "$R"
printf "%b*%b %bd51c930%b drop the dead retry queue table\n"        "$C" "$R" "$Y" "$R"
printf "%b*%b %b2f77ba4%b verify signatures before parsing\n"       "$C" "$R" "$Y" "$R"
printf "%b*%b %b88e0c15%b bump undici to 6.19\n\n"                  "$C" "$R" "$Y" "$R"

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b npm run lint\n\n" \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"
printf "%b> payments-api@2.1.4 lint%b\n" "$D" "$R"
printf "%b> eslint src test --max-warnings 0%b\n\n" "$D" "$R"
printf "%b✓%b no problems found %b(48 files, 1.9s)%b\n\n" "$G" "$R" "$D" "$R"

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b node scripts/replay.js --since 1h\n\n" \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"
printf "  %bevt_8812%b  charge.succeeded    %b202%b  %b41ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8813%b  charge.refunded     %b202%b  %b38ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8814%b  payout.paid         %b202%b  %b52ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8815%b  charge.failed       %b202%b  %b44ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8816%b  charge.succeeded    %b202%b  %b39ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8817%b  dispute.created     %b202%b  %b61ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8818%b  payout.failed       %b429%b  %b— retry in 2s%b\n" "$Y" "$R" "$Y" "$R" "$D" "$R"
printf "  %bevt_8818%b  payout.failed       %b202%b  %b58ms (attempt 2)%b\n" "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8819%b  charge.succeeded    %b202%b  %b37ms%b\n"  "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %bevt_8820%b  invoice.paid        %b202%b  %b45ms%b\n\n" "$Y" "$R" "$G" "$R" "$D" "$R"
printf "  %b10 delivered, 1 retried, 0 dropped%b\n\n" "$B" "$R"

printf "%b➜%b  %bpayments-api%b %bgit:(%bfix-webhook-retry%b)%b " \
  "$G" "$R" "$C" "$R" "$D" "$C" "$D" "$R"
