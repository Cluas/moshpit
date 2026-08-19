# TestFlight · 1.0.0 (376)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

376 = 375's root fix plus everything found while confirming it on a
real device, all verified there before this build was cut.

WHAT'S NEW OVER 375:

• The true root of the week's garble, found by bisecting on-device
  against build 334: since 368, each chunk of terminal output was
  delivered through its own unordered task, so under load the byte
  stream itself could arrive shuffled. Delivery is sequential again
  — order guaranteed by construction, not by luck.
• The renderer follows 334's law again: mid-session output is never
  dropped. A size fight with a desktop client may flash one
  mis-wrapped beat, then repaints clean — no permanent divergence.
• Pasted images now read as images over SSH+tmux: tmux itself
  decides the bracketed-paste wrapping (it knows the pane's real
  mode; the app's local guess was blind after a fresh attach), so
  Claude Code shows [Image #N] instead of a literal file path.
• Vibe Island's lock-screen card redesigned: real type hierarchy,
  the timer reads as a status badge, long commands truncate
  smartly (claude --resume e746…d64e), Allow/Deny become tonal
  pills with quick replies as quiet text buttons.
• mosh cleans up after itself: a connect attempt whose UDP never
  gets a reply now kills the mosh-server it spawned — both when
  the dead-return-path banner fires and when you back out before
  it. No more zombie servers piling up host-side.
• A dead or silently wedged connection reconnects itself within
  seconds (finished stream, or commands piling up unanswered).

TEST:

• The scroll torture, one more time, on the build that should end
  it: scroll hard and long, app-switch, type on an attached desktop
  terminal, open/close the keyboard. Transient one-beat flashes are
  fine; anything that STAYS wrong is a bug.
• Attach an image over SSH+tmux to Claude Code — it must show
  [Image #N], not a path.
• Trigger an agent permission prompt; check the lock-screen card's
  new look, then answer from the lock screen — the tap must reach
  the agent (if it can't, you now get an honest "Not delivered").
• mosh: connect, kill the network for 30s, restore — the session
  must resume; failed attempts must not leave mosh-server processes
  behind on the host (ps aux | grep mosh-server).

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
