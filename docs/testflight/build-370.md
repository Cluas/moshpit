# TestFlight · 1.0.0 (370)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

370 fixes styles shredding while scrolling under SSH+tmux — the bug
363's white-block fix introduced.

WHAT WAS WRONG SINCE 363:

• 363 fixed mosh's permanent white blocks by clearing the leftover
  half of an overwritten CJK character. It cleared it to the DEFAULT
  style — but apps like Claude Code repaint only what they think
  changed, and they think that cell still holds their own colors. So
  every overwrite of a CJK boundary left a permanent default-colored
  hole in a colored region, and scrolling (which repaints in small
  patches, thousands of overwrites) shredded the styling visibly.
  mosh was unaffected — the default fill happened to match its model
  — which is why this only showed on SSH+tmux.
• 370 clears those halves the way a real erase does: keeping the
  colors the app painted with, no reverse-video leakage. The mosh
  white-block fix is preserved (verified against mosh's own frame
  model, byte-for-byte, attribute-for-attribute).

TEST:

• SSH+tmux, a pane with colored CJK-heavy output (Claude Code is
  ideal): scroll up and down repeatedly, fast and slow. Colored
  regions must stay solid — no black/default-colored holes, no
  bright single-cell blocks appearing as you scroll.
• mosh: confirm white blocks stay dead (they should — the fix is
  attribute-only) and scrollback stays clean.
• Everything 369 shipped still applies: window switches repaint in
  two passes, protocol text never appears inside a pane, reconnects
  start clean.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
