# TestFlight · 1.0.0 (380)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. Connection-stability work plus the logging that makes the
next "it jumped" / "it dropped" report answerable instead of guessable.

---

380 is about staying connected, and about being able to prove what
happened when we don't.

WHAT'S NEW OVER 379:

• A connection that is visibly carrying data is no longer interrogated
  about whether it is alive. Every 12 seconds the app used to open a
  WHOLE fresh channel to run "true" — on top of herdr's control plane,
  which already round-trips every 2 to 8 seconds on the same
  connection. That is real load on exactly the flaky links where it
  hurts. Bytes arriving in the last 11 seconds now count as the answer.
• A slow probe no longer kills a working session. The check could not
  tell "the socket refused" from "the reply is late", and treated both
  as death — followed by a full re-handshake, 6 to 9 seconds on the
  link where it just happened. A refusal still reconnects instantly
  (that is real evidence); a silence now has to happen twice in a row.
• The tmux dead-channel watchdog scales with the link. It declared a
  channel dead after 12 seconds of silence with commands outstanding —
  fine on a LAN, trigger-happy on a lossy cellular path where one
  stalled retransmit eats that much without anything being wrong. It
  now scales with the round trip this connection actually measures.
• Every reconnect says WHY in the log now (transport closed / probe
  refused / probe silent / tmux channel silent), and every terminal
  resize logs its before-and-after size. Both are the questions that
  could not be answered after the fact for the last two reports.

TEST:

• Use it on a bad network on purpose: lift, tunnel, weak cellular. It
  should hold on through hiccups it used to drop through. What matters
  is whether it reconnects LESS, not whether it reconnects faster.
• Long idle sessions: leave a session open for ten minutes untouched,
  come back and type. It should still be there.
• Everything 379 asked for still stands — the keyboard should glide,
  the Windows sheet must not blank the screen above it, and agent
  switching must never show the agent you came from.
• If the picture jumps or the connection drops, the log now has the
  answer. Note the time it happened and tell me; with the phone
  plugged in the cause is one grep away.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• mosh + herdr switching is still around a second on a high-latency
  link: each switch opens a fresh SSH channel to steer the host-side
  renderer.
• herdr's control plane still opens one exec channel per poll. That is
  the next stability item — the machinery to move it onto a single
  persistent socket already exists (it is what the push upgrade uses).

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr, and
roughly when it happened.
