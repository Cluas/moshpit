# TestFlight · 1.0.0 (375)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

375 ends the scroll-garble saga at its true root, found by on-device
bisection against build 334 (the last release testers called clean).

THE ROOT CAUSE:

• Since build 368, every chunk of SSH terminal output was handed to
  the screen through its own detached task — and detached tasks
  don't keep their order. Under load (fast scrolling, an agent
  streaming) chunk N+1 could land before chunk N: the terminal BYTE
  STREAM itself arrived shuffled. Every symptom reported since —
  literal escape-code fragments, overlapping rows, characters in
  wrong positions, screens that garbled only after sustained
  scrolling — was this one line. Output is now delivered
  synchronously, in order, by construction.

ALSO IN 375:

• The renderer returns to 334's law: mid-session output is never
  dropped. A desktop terminal fighting over window size may cause a
  brief mis-wrapped flash that repaints clean — but no more
  permanent divergence from a missed repair.
• Scrolling a tmux pane never touches copy-mode (it is invisible to
  the app's tmux channel and hijacked desktop clients).
• A dead or silently-stalled connection now notices itself: piled-up
  unanswered commands or a finished stream trigger an automatic
  reattach within seconds (tmux keeps the session).
• The protocol parser survives lossy or reordered transit: glued
  protocol lines are recovered, runaway reply blocks are force-
  closed, pairing can no longer shift for good.
• Frame-size validation no longer counts invisible OSC hyperlink
  payloads as width (it was rejecting legitimate repaints).

TEST:

• The old torture: scroll a busy pane hard and long, app-switch and
  back, type on a desktop terminal attached to the same session,
  open/close the keyboard. Everything rendered must stay coherent —
  transient one-beat flashes are acceptable, anything that STAYS
  wrong is a bug.
• After killing the network (airplane mode 30s), the session must
  reattach by itself within ~15s of connectivity returning.
• mosh users: if mosh won't connect after updating, check
  Settings → Privacy & Security → Local Network → Moshpit is ON
  (iOS resets it sometimes); report if it fails with it on.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
