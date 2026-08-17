# TestFlight · 1.0.0 (357)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

357 rebuilds mosh's survival story. One idea, three layers: whether the
UDP socket is alive after iOS has had its hands on it is unknowable, so
the app stops asking and just replaces it.

MOSH: BACKGROUND, COME BACK, KEEP TYPING:

• Returning from more than ~20s of background replaces the UDP flow
  outright — the server re-homes instantly, exactly like a roam.
• A liveness monitor catches the flow dying mid-session (you send,
  nothing answers for 9s) and replaces it then too. This is the fix
  aimed at "typing stopped working after a while".
• If three replacements in a row still hear nothing back, the dead-
  network banner appears — and it keeps retrying, so it heals itself
  the moment the network returns.

Please hammer this: background the app for minutes, lock the phone,
switch networks, then come back and type immediately. Worth reporting:
any frozen session that a 10-second wait doesn't heal — long-press the
MOSH pill and screenshot the counters if so.

EVERYTHING 356 SHIPPED STILL APPLIES — foreground re-layout for the
truncated-screen bug, and the diagnostics popover.

KNOWN ISSUES — DON'T RE-REPORT:

• White cursor-sized blocks in claude output over mosh — reproduction
  needs lossy networks; a lab rig for that is in progress.
• herdr over mosh shows the full TUI, not a native single tab — design
  decision pending.
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• Lock screen stops updating minutes after backgrounding.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
