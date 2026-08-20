# TestFlight · 1.0.0 (377)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

377 is about one thing: switching between agents. Everything here was
measured against a real herdr server and a real tmux before and after,
not eyeballed.

WHAT'S NEW OVER 376:

• Switching agents over SSH + herdr is roughly 13x cheaper on the host:
  545ms per switch, down to 40ms. Almost none of that half-second was
  the protocol — 400ms was a sleep we guessed at instead of waiting for
  the event the server already sends, and ~250ms was your login shell
  redrawing its prompt, because the switch was typed as a command at
  that prompt. It now rides a small reader loop instead, and the
  handover waits on the server's own "released" message.
• The previous agent's screen no longer flashes up mid-switch. The
  cover over a switch used to expire on a fixed timer and uncover the
  pane you just left, still fully painted and indistinguishable from
  live. The buffer is wiped underneath the cover now, so every path
  that lifts it early shows an empty screen rather than the wrong
  agent, and the timer scales off how long switches actually take on
  your connection.
• mosh + herdr switches are covered too. That transport renders on the
  host and ships screen diffs, so it had no cover at all — you watched
  the old pane until the new one repainted. It now veils, and the veil
  lifts when the repaint stops arriving. The host-side re-attach pause
  also went from 300ms to 50ms.
• tmux window and pane switches reveal sooner. The cover was waiting
  600-700ms for a redraw to finish draining — but switching to an idle
  shell or a settled agent produces no output at all, so that wait was
  for a redraw that was never coming. It now waits a short, round-trip-
  scaled beat when nothing is painting, and the full drain window only
  once the first byte of a real redraw arrives.
• Opening the Windows / Sessions / Pane sheet no longer re-lays-out the
  terminal. Collapsing the keyboard for the sheet handed its space to
  the pane, which reflowed the buffer and resized the agent — then did
  it again in reverse when the sheet closed. Two SIGWINCHes per switch,
  for two sizes nobody ever saw (the sheet covers the screen). Measured
  after: zero.

TEST:

• SSH + herdr: switch between agents from the home card and from the
  breadcrumb, repeatedly and fast. It should feel close to instant, and
  you should never see the agent you came from — a black beat is fine,
  the wrong content is a bug.
• mosh + herdr: same switching. Slower than SSH by nature (the host
  re-attaches and mosh ships the repaint), but it must be covered the
  whole way, not showing the old pane.
• With the keyboard UP, open the Windows sheet, switch, and come back.
  The keyboard should return and the text must not re-wrap or jump.
• tmux: switch windows a few times and compare with 376 — the covered
  beat should be noticeably shorter.
• Everything the 376 notes asked for still applies, especially the
  scroll torture: scroll hard, app-switch, type on an attached desktop
  terminal, open/close the keyboard.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• mosh + herdr switching is still around a second on a high-latency
  link: each switch opens a fresh SSH channel to steer the host-side
  renderer. Known, and the next thing to fix on that path.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
