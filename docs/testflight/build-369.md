# TestFlight · 1.0.0 (369)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

369 fixes the SSH+tmux garble 368 introduced, and hardens weak-network
behaviour across the board. Update from 368 as soon as you can.

WHAT 368 BROKE (AND 369 FIXES):

• A size-tracking change in 368 could hand tmux a garbage client size
  from one transient layout pass. Your window got pinned hundreds of
  columns wide, every line arrived laid out for that width, and the
  session list could even show a raw layout string as the session
  name. Reverted; mosh was unaffected (368's scrollback fix stands).

WEAK-NETWORK FIXES (OLD BUGS, EXPOSED BY SLOW LINKS):

• Command replies could get paired to the wrong request when the tmux
  handshake reply arrived late — the visible symptom was protocol
  text painted INTO a pane: a lone "2 51" on a black screen, rows of
  "%0||||", frames from the wrong query. Three separate holes closed.
• Switching windows over a slow link showed garbled/stale content for
  seconds before self-correcting. Switches now repaint in two passes
  (like resizes already did) and the transition cover's deadline
  scales with the measured round trip instead of assuming a fast one.
• If tmux ever paused a slow client's pane, the resume command we
  sent was invalid and the pane stayed frozen forever. Fixed, and the
  gap gets repainted on resume.
• Reconnecting after a drop that cut mid-character or mid-escape no
  longer garbles (or blanks) the new session's first screen — the
  dead connection's half-parsed input is discarded at reconnect.

TEST:

• SSH+tmux on a normal link first: connect, open a few windows,
  switch between them — everything 368 garbled must be clean.
• Then the weak-network pass (cellular with poor signal, or a hotel
  VPN): switch windows repeatedly while an agent prints output. A
  brief clean transition cover is fine; garbled or duplicated text
  that persists is a bug. Protocol text ("%begin", "%0||||", a lone
  pair of numbers) appearing inside a pane must never happen.
• Kill the network mid-session (airplane mode 10s) while CJK text is
  printing, reconnect: the first screen after reconnect must be clean.

EVERYTHING 368 SHIPPED STILL APPLIES — mosh scrollback no longer
shreds into 1-2 character columns, CJK wide-char hygiene.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
