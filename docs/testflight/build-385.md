# TestFlight · 1.0.0 (385)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. A ROLLBACK: this is 377's app code, rebuilt.

---

385 is a rollback. Its app code is byte-for-byte 377's — the last build
before the keyboard and resize work started — because that work did not
fix herdr and kept finding new ways to be wrong on the way.

WHAT THIS UNDOES (378 through 384):

• The keyboard transition work: the slide, the predicted content shift,
  the early resize on dismissal, and the gate that held a resize until
  its repaint. Some of it measured well in isolation; none of it earned
  its place, and 382 shipped a fix for herdr to a tmux user while 382's
  own gate then put two copies of Claude Code's UI on screen.
• The connection work from 380 and 383: the keepalive that skipped
  redundant probes, the timeout-is-not-death rule, the scaled tmux
  watchdog, and herdr's push channel finally coming up. These measured
  well (idle cost about 12 exec channels a minute down to about 1.3)
  and nothing was reported against them — they are being withdrawn only
  because they rode in the same builds, and a rollback that keeps half
  the changes is not a rollback.

So: the keyboard behaves exactly as it did in 377 — including its
original jitter. Nothing is fixed here. This is a known floor to stand
on while the herdr case gets a reproduction it actually deserves.

WHAT IS STILL IN (it predates 378):

• Agent switching on SSH + herdr, roughly 13x cheaper on the host.
• The switch no longer showing the pane you came from.
• The Windows / Sessions / Pane sheet no longer re-wrapping the pane.
• tmux revealing sooner after a window switch.

TEST:

• Mostly: does it feel like 377 again? Anything that is WORSE than 377
  is a mistake in this rollback and I want to hear about it immediately.
• The keyboard will still jitter. That is expected here.

KNOWN ISSUES — DON'T RE-REPORT:

• Keyboard open/close jitter, and Claude Code's bottom rows blinking
  out on tmux. Both are back, deliberately.
• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
