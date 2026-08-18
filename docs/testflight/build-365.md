# TestFlight · 1.0.0 (365)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

365 finishes the herdr-over-mosh takeover saga: the cleanup now also
retires the IMMORTAL renderers left behind by builds 360/361.

WHAT WAS STILL FIGHTING (field evidence, one host):

• Three attach processes contending for the same pane, plus eight
  orphaned mosh sessions from earlier builds' testing.
• 360/361's renderer loops never learned to lose AND used an older
  state-file layout that 362's boot cleanup didn't match — so they
  kept stealing the pane forever. That is why "terminal attach taken
  over" survived 364, and why picking a claude-less tab could land on
  the claude pane: the server DID switch (the breadcrumb was right),
  then an old loop stole the screen right back.

WHAT 365 DOES:

• Boot cleanup now retires BOTH state-file layouts, in an order that
  neutralizes even loops that never learned to lose: targets are
  removed first, so a killed old loop wakes to "no target" and idles
  forever instead of re-attaching.
• One fresh mosh+herdr connect per host is all it takes — no manual
  cleanup needed on other machines you tested earlier builds against.

TEST, ON A HOST THAT SAW 360/361:

• Connect mosh + herdr once. From then on: no "taken over" flashes,
  and picking any pane/tab/workspace from the sheets must land the
  screen on exactly that pane (type immediately to confirm focus).
• If a switch still lands on the wrong pane on a CLEAN host, that is
  a different bug — screenshot the breadcrumb (it shows where the
  server thinks you are) and report.

EVERYTHING 363/364 SHIPPED STILL APPLIES — white blocks root-fixed,
working Repaint, probe timeouts that cannot wedge recovery, sidecar
self-heal after long backgrounds.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
