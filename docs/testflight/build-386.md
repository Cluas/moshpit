# TestFlight · 1.0.0 (386)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. 385's rollback, plus back the two pieces that were asked
for by name — and an honest note about what is and is not proven.

---

386 is 385 (which was 377's code) with two things added back, chosen
deliberately rather than by which commit they happened to ride in.

BACK IN — CONNECTION WORK (this part is measured):

• A connection that is visibly carrying data is no longer interrogated
  about whether it is alive. Every 12s the app used to open a whole
  fresh channel to run "true", on top of herdr's control plane already
  round-tripping on the same connection.
• A slow probe no longer kills a working session. A refusal is proof
  and still reconnects at once; a silence has to repeat first.
• tmux's dead-channel watchdog scales with the link instead of a fixed
  12s, which a lossy path can eat without anything being wrong.
• herdr's push channel comes up for the first time (its subscribe used
  to be eaten by the login shell before python took the pipe), and the
  event debounce is a real trailing one instead of firing once per
  interval.
• Idle cost end to end: about 12 exec channels a minute down to
  about 1.3, and tree updates arrive on an event rather than up to 8
  seconds late.
• Every reconnect logs its cause; every terminal resize logs its
  before-and-after size.

BACK IN — THE tmux RESIZE HOLD (this part is NOT proven):

• The local buffer is not resized until the capture that replaces what
  the resize destroys has arrived, so the rows below the cursor —
  Claude Code's separator and footer — are not thrown away and
  repainted a round trip later.
• Straight talk: I told you this measured 6.23s of missing footer down
  to 0.80s. It does not reproduce. Re-running the same harness against
  the same mechanism gives 6.3s again, and instrumenting proves the
  hold really does run. The earlier number was my test rig being lucky,
  not the fix working, and I should not have quoted it. The mechanism
  is sound and cannot make things worse — it only delays a resize by
  one round trip — but whether YOU see the footer stop blinking is the
  open question this build asks.

NOT back in: everything that touched the keyboard animation, and the
herdr resize gate that put two copies of the UI on screen.

TEST:

• tmux, keyboard up and down over a working Claude Code pane: do the
  bottom two rows still blink out? A yes is a real answer, not a
  failure — it means the hold is not the cause and I look elsewhere.
• herdr: this build changes nothing about its resize behaviour. It
  should look exactly like 385/377.
• A day of ordinary use on a bad network: it should reconnect less.
• Anything that looks STALE on herdr (a new window not appearing, an
  agent's status not changing) — push is carrying those updates for
  the first time.

KNOWN ISSUES — DON'T RE-REPORT:

• Keyboard jitter on herdr, unchanged from 377.
• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
