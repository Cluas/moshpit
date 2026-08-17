# TestFlight · 1.0.0 (363)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

363 closes the oldest open bug for real, and repairs two things 361
shipped that turned out not to work.

WHITE BLOCKS: ROOT-CAUSED AND FIXED:

• A byte-level replay rig reproduced it deterministically, no packet
  loss needed: when a repaint overwrites only the FIRST half of a
  wide (CJK) character, the terminal engine left the second half's
  placeholder cell behind — and that placeholder carries an inverted
  background, so it paints as a bright one-cell block. mosh's model
  says that cell is already blank, so it never repaints it: permanent.
  That is also why blocks appeared after SWITCHING panes (minimal
  in-place repaints) and clustered at the ends of Chinese lines.
• Fixed in the terminal engine itself (both write paths clear the
  other half; the renderer additionally treats any orphaned
  placeholder as a plain blank). The exact captured byte burst that
  used to seed 9 permanent blocks now replays with zero divergence,
  and the repro is checked in as a regression test.
• Test: mosh + tmux, CJK-heavy panes (claude sessions qualify),
  switch panes and windows a lot. No white blocks should EVER appear.

REPAINT, FIXED FOR REAL:

• 361's "Repaint screen" button and the post-background auto-repaint
  turned out to be no-ops against a stock mosh server (it ignores the
  acknowledgment trick they used) — worse, the trick could freeze the
  session. Repaint now nudges the terminal width by one column and
  back, which forces mosh to repaint every cell. You'll see a brief
  reflow flicker; that's the repaint working.

ALSO IN 363 (from the last field reports):

• herdr-over-mosh takeover storms are gone (attach loops now lose
  gracefully; reconnects retire their predecessors).
• After a LONG background, the tmux/herdr control plane (breadcrumb,
  sheets) rebuilds itself within one keepalive tick — the liveness
  check was trusting a flag iOS makes lie after suspension.
  Test: background the app 10+ minutes, return: breadcrumb and pane
  switching must come back by themselves within ~15s.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
