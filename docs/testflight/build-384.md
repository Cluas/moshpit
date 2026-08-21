# TestFlight · 1.0.0 (384)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. One race, closed. Take this over 383.

---

384 closes a race in the resize gate that 382 introduced and 383 kept.
It shows up as TWO copies of Claude Code's interface on screen at once:
a stale one higher up and the live one at the bottom, with a blank band
between them (user screenshot on herdr).

WHAT'S NEW OVER 383:

• The gate now waits for the repaint of the size it ASKED FOR, not just
  for the next full repaint. herdr sends a full frame after every
  resize — verified directly against a server, 20 rows to 40 rows,
  full frame in the same breath — but a frame that was already in
  flight when the resize was sent describes the OLD size. Releasing on
  that one grows the buffer and then paints a short screen into it, and
  the rows below the paint keep whatever was there: a second, stale
  copy of the UI sitting under the live one. Frames carry their own
  width and height, so the gate matches on them now.

TEST:

• herdr: open and close the keyboard over a working Claude Code pane,
  repeatedly and fast. There must never be two input boxes, two status
  lines, or a band of old output stranded above the live UI.
• tmux: the same — 383's fix for the disappearing bottom rows is still
  in here and is the thing to confirm holds up over real use.
• Both: nothing should vanish, and nothing should be left behind.

STILL FROM 383, AND STILL WORTH WATCHING:

• Raising the keyboard no longer deletes the two rows under the cursor
  on tmux (measured: footer missing 6.23s before, 0.80s after).
• herdr's push channel works for the first time, so tree updates now
  arrive on an event rather than up to 8 seconds late. Anything that
  looks STALE — a new window not appearing, an agent's status not
  changing — is worth reporting.
• Idle cost about 12 exec channels a minute down to about 1.3.
• On a bad network it should reconnect LESS; the log names the cause
  when it does.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane still fight over it: herdr's direct
  attach is exclusive per terminal, so whoever attaches last evicts the
  other.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr, and
roughly when it happened.
