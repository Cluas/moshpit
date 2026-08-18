# TestFlight · 1.0.0 (368)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

368 fixes scrollback turning into a column of 1-2 character shards
over mosh.

WHY SCROLLING UP SHOWED SHREDDED TEXT:

• The terminal view was born two columns wide (a zero-size frame's
  minimum grid) and only later resized to the phone's real grid. Over
  mosh the first screenful arrives BEFORE that resize — it got wrapped
  at two columns, and those shards sank into the 50,000-line local
  scrollback. The later resize can't un-wrap them (the reflow is
  one-way), and the server only repaints the visible screen, never
  the phone's history — so the live screen looked fine while
  everything above it was shredded (screenshot report, 2026-08-18).
• Fix: the terminal is now born at the screen's size, so the first
  screenful lands in the shape the server drew it for. Two smaller
  holes closed alongside: a transient zero-size layout pass (sheet
  transitions) could re-shred a live buffer the same way, and the
  size first told to mosh-server now comes from the real layout
  instead of an estimate that ran a column narrow.

ALSO IN 368:

• CJK hygiene in the terminal engine: overwriting half of a wide
  character now clears the whole cell — no more stray half-characters
  left behind in mixed Chinese/English output.

TEST:

• herdr over mosh: connect, let an agent print a few screenfuls, then
  swipe up through history — every line must read at full width, no
  single-character columns anywhere.
• Existing sessions carry their already-shredded history until you
  reconnect once on this build. Shards appearing in a FRESH 368
  session is the bug and must be reported.
• Same check on SSH+tmux and plain SSH — their scrollback should be
  unchanged.

EVERYTHING 367 SHIPPED STILL APPLIES — semantic arrows over herdr,
shell-pane switching fallback, white blocks root-fixed, probe
timeouts, sidecar self-heal.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
