# TestFlight · 1.0.0 (361)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

361 goes after the oldest open report: white cursor-sized blocks in
claude output over mosh.

WHITE BLOCKS: SELF-HEALING REPAINT (AND A DIAGNOSIS TO CONFIRM):

• Root-cause analysis points at render divergence: this client feeds
  mosh's screen diffs straight into the terminal view with no local
  framebuffer model, so if the view's idea of a cell ever drifts from
  the server's (prime suspect: wide-CJK width disagreement — the
  blocks cluster at the ends of wrapped Chinese lines), the drift
  STAYS: the server never resends cells it believes are correct. A
  loss-free network ships full frames that mask the drift, which is
  why it never reproduced in the lab; a lossy link's partial diffs
  leave it standing.
• The protocol has a free remedy: acking state 0 makes the server
  repaint everything from blank. 361 does that automatically when the
  app returns from a real suspension, and adds a "Repaint screen"
  button to the MOSH pill's long-press diagnostics.
• Test, when blocks appear: long-press the MOSH pill → Repaint screen.
  If the blocks VANISH, the divergence diagnosis is confirmed (please
  report that, plus a screenshot from BEFORE the repaint — it tells us
  which glyphs seeded the drift). If they SURVIVE a repaint, that is a
  different bug and we very much want that screenshot too.

ALSO IN 361:

• Promoting builds to external groups no longer trips over a shell
  quirk in the release tooling (team-facing, not user-facing).

EVERYTHING 360 SHIPPED STILL APPLIES — raw single-pane herdr over
mosh, agent names in Select Pane, island over mosh+tmux, push control
plane. 360 is what external groups just received; 361 is canary-first.

KNOWN ISSUES — DON'T RE-REPORT:

• Two Moshpits on the same herdr pane trade the raw attach back and
  forth (herdr's attach is exclusive); one phone per pane for now.
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
