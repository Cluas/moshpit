# TestFlight · 1.0.0 (356)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. Written for the tester: what to try, what changed, what
NOT to bother reporting.

RULES: under 4000 chars; PLAIN TEXT ONLY (TestFlight renders verbatim —
colon headers, "•" bullets, no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

356 instruments the two reports we can't reproduce in the lab, and takes
a swing at the screen-truncation one.


IF TYPING OVER MOSH STOPS WORKING — SCREENSHOT THE PILL:

Long-press the MOSH pill (top left) while it's happening. The popover
now shows the input path's vitals: packets sent, your newest keystroke
state vs the newest the server acknowledged, and the raw connection
state. Type one key, reopen it, screenshot both. That one picture tells
us whether input dies inside the app, on the wire, or on the way back.


SWITCHING APPS SHOULD NO LONGER EAT THE BOTTOM OF THE SCREEN:

Coming back from another app (especially with a Chinese keyboard open
beforehand) could leave the terminal missing its bottom chunk. The
screen now re-measures and repaints on every foreground return.

Please try: keyboard up, IME switched, hop to another app, come back.
Worth reporting: a truncated screen that a rotate-or-reenter doesn't
fix — plus whether typing still worked.


STILL OPEN — don't re-report, but extra observations welcome:

• White cursor-sized blocks scattered in claude output over mosh. Under
  investigation (suspected stale cursor artifacts during streaming).
• herdr over mosh currently shows herdr's own full-screen TUI rather
  than the native single-pane view SSH gets. Design work, not a quick
  fix — tracked.


KNOWN ISSUES — PLEASE DON'T RE-REPORT:

• Half-visible last key in the shortcut row on narrower phones — the
  row scrolls; deliberate.
• Model downloads pause while backgrounded.
• Home header "1 live connection" vs OFFLINE card. Cosmetic.
• Lock screen stops updating minutes after backgrounding — iOS
  suspends the app; no push channel yet.


REPORTING:

Screenshot through TestFlight, or share straight to your agent.
Include: device model, iOS version, SSH or mosh, tmux or herdr.
