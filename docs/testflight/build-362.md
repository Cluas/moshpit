# TestFlight · 1.0.0 (362)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

362 kills the herdr-over-mosh takeover storm from the 361 field report
("the screen keeps flashing: terminal attach taken over").

WHAT IT WAS: mosh sessions survive disconnects BY DESIGN (that is the
roaming feature) — so every reconnect left the previous session's
attach loop alive on the host, and all generations fought over the
same pane with --takeover, evicting each other three times a second.

WHAT 362 DOES:

• Loops now lose gracefully: an attach that exits on its own (evicted
  by another client, server stopped) stays down instead of grabbing
  the pane back. Only the app's own pane-switch (an uncatchable kill)
  re-attaches.
• Each connect gets its own generation of steering files — an orphaned
  loop from a previous session can never be re-triggered by the live
  session's pane switches.
• Connecting also retires all previous generations on the host before
  the new renderer boots.

TEST: mosh + herdr, then be mean to it: kill the app, reconnect,
background it, reconnect again, switch panes a lot. The screen must
never flash "terminal attach taken over" repeatedly. One eviction
notice when another DEVICE takes the pane is correct behavior — it
should stay quiet after that, not fight.

EVERYTHING 361 SHIPPED STILL APPLIES — white-block self-heal (long-
press the MOSH pill → Repaint screen; report whether blocks vanish),
raw single-pane herdr over mosh, push control plane, island fixes.

KNOWN ISSUES — DON'T RE-REPORT:

• White blocks over mosh+tmux after switching panes — under active
  investigation with a byte-level replay rig; the Repaint button is
  the workaround AND the diagnostic (please report its effect).
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
