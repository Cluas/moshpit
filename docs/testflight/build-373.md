# TestFlight · 1.0.0 (373)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

373 fixes the scroll garble for real — root-caused from a byte-level
recording of an actual failing session.

WHAT THE RECORDING SHOWED:

• The protocol stream was perfect. The garble was geometry: with a
  desktop terminal attached to the SAME tmux session, the window's
  size flapped between the phone grid and the desktop grid thirteen
  times in one scrolling session. Every time you switch away from
  Moshpit (screenshot, chat) it politely hands the window back to
  the desktop's size; every flap makes the pane's app repaint at the
  OTHER width, and 499-column repaints fed into a 70-column phone
  terminal wrap sevenfold — the shredded overlapping fragments.
  A repaint-from-tmux could even be captured mid-flap at the
  desktop's size and painted as "authoritative".

WHAT 373 DOES:

• While the window sits at another client's width, its output no
  longer reaches the phone's grid (the agent-bell still does) — the
  moment the width comes back, the screen repaints cleanly from
  tmux's model, twice, like resizes already did.
• A repaint frame that arrives taller than the phone grid is
  recognized as captured-at-the-wrong-size and dropped; the window
  is re-pinned and a correct frame follows.

TEST (needs a desktop terminal attached to the same tmux session):

• Open the same session in a big desktop terminal. On the phone:
  scroll a busy pane, switch to another app and back, scroll again —
  repeatedly. The screen may briefly hold still during a size
  hand-back, but what renders must always be laid out for the phone:
  no shredded overlapping fragments, no stray escape-code tails, no
  protocol text.
• Without a desktop client attached everything should look identical
  to 372.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
