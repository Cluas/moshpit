# TestFlight · 1.0.0 (391)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. An instrument, not a fix — the jitter while Claude streams
could not be reproduced here, and this is how we find out why.

---

391 puts the app's own log on screen, because the last several bugs
were all diagnosed by guessing.

THE JITTER WHILE CLAUDE OUTPUTS: NOT REPRODUCED

Ran it here against a real Claude Code pane over tmux, recorded at
30fps while it streamed 120 lines: the input box sat at the same pixel
for all 709 frames and the app logged ZERO terminal resizes for the
whole run. Whatever is shaking on your phone is not shaking here — and
the most likely reason is the link: this test talks to localhost with
no latency and no loss, and yours does not.

Rather than guess a fifth time, this build makes your phone able to
answer the question.

WHAT'S NEW:

• Settings → DIAGNOSTICS → Recent Log. The last 30 minutes of what the
  app logged about itself, newest first, with a Copy button. Two lines
  matter for this: "layout: terminal 402x674 -> 402x373" is written
  every time the terminal actually changes size (a WIDTH change there
  re-wraps every line, and is the ugly kind), and "health: dropped
  (…)" names the cause of every reconnect.
  Nothing is uploaded or stored — it reads the log the system already
  keeps for this app, in the place where you saw the problem.

WHAT TO DO WHEN IT SHAKES:

• Let it shake, then immediately open Settings → Recent Log and
  screenshot it (or Copy and paste it to me). If there are "layout:"
  lines during the shaking, the app is resizing the terminal while
  Claude writes and I will know exactly what to fix. If there are
  none, it is something else and I will stop looking there.

STILL FROM 387 THROUGH 390:

• The keyboard transition is covered rather than rendered live.
• Switching input method is not covered.
• Reconnecting keeps the breadcrumb instead of showing the host address.
• A tap while reading history no longer summons the keyboard.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
