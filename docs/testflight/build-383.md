# TestFlight · 1.0.0 (383)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. The disappearing footer, fixed on the transport it actually
happens on — plus the connection work that has been waiting.

---

382 fixed the vanishing rows for herdr and shipped it to someone using
tmux. The breadcrumb in the screenshots said so — "0 › 1:moshpit ›
2.1.237" is tmux's three-part shape — and the gate 382 added only ever
accepted herdr's frame channel. Same fix, right transport, this time
with the before-and-after to prove it.

WHAT'S NEW OVER 382:

• Raising the keyboard over tmux no longer deletes the rows under the
  cursor. Shrinking a buffer is lossy — rows go by popping the lines
  BELOW the cursor, which is exactly where Claude Code keeps its
  separator and its "bypass permissions" line. tmux's own reflow does
  not lose them, so the capture taken after every resize has them; all
  that was missing was to hold the local resize until that capture is
  in hand, then apply both in one turn. Recorded at 30fps against a TUI
  shaped like Claude Code's: the footer was missing for 6.23 seconds
  before this change and 0.80 seconds after, and what is left is the
  rising keyboard covering it, not the app losing it.

• The connection work measured last round, held back until now:
  - herdr's push channel has never once worked. The subscribe was
    written before python had taken over the pipe, so the login shell
    ate it and the request timed out — every session paid for a second
    SSH connection and a 10s timeout and then fell back to polling
    anyway. It waits for the pump to announce itself now.
  - With push finally up, the first measurement showed 47 snapshot
    reads in 8 seconds: the "debounce" fired once per interval for as
    long as events kept coming, and a subscribe replays state as a
    burst. It is a real trailing debounce now, with a cap so a
    never-ending stream still gets read.
  - Idle cost, measured end to end: about 12 exec channels a minute
    before this run, about 1.3 after — and tree updates arrive on an
    event instead of up to 8 seconds late.

TEST:

• THE ONE THAT MATTERS: open and close the keyboard over a working
  Claude Code pane, repeatedly. Its bottom two rows must not vanish.
• Then the same over a plain shell and over herdr, if you use it.
• Leave a session for ten minutes and come back; type. Still there?
• On a bad network, it should reconnect LESS. If it drops, note the
  time — the log names the cause.
• Anything that looks like the tree is stale (a new window not showing
  up, an agent's status not changing) is worth reporting: push is
  carrying those updates for the first time.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane still fight: herdr's direct attach is
  exclusive, so whoever attaches last evicts the other.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr, and
roughly when it happened.
