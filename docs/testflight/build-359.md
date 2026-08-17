# TestFlight · 1.0.0 (359)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

359 is one feature on top of 358: herdr over mosh gets the mosh+tmux
treatment — one pane, full-screen, driven from the native sheets.

HERDR OVER MOSH: IMMERSIVE PANE, LIKE TMUX:

• Connecting over mosh with herdr used to drop you in herdr's desktop
  TUI with its sidebar eating a third of the phone. Now the moment the
  control plane attaches, the focused pane is zoomed server-side to
  fill the screen — and every pane you pick from Select Pane (or tab /
  workspace you pick from the sheets) zooms as it focuses, in one
  server command.
• Same semantics as mosh+tmux immersive zoom: zoom is server state, so
  a desktop TUI on the same workspace sees the pane zoomed too.
  prefix z (ctrl+b z by default) brings the splits back, on either end.
• Test: mosh + herdr connection, several panes across a couple of tabs.
  Switch panes from Select Pane, tabs from the Windows sheet,
  workspaces from Sessions — each landing should fill the screen with
  exactly that pane within a second or two. Worth reporting: any
  landing that leaves you in the stacked mobile layout, a zoom that
  lands on the wrong pane, or typing that goes somewhere other than
  the pane on screen.

EVERYTHING 358 SHIPPED STILL APPLIES — island over mosh+tmux, herdr
push control plane, Select Pane as a list. Keep hammering those too.

KNOWN ISSUES — DON'T RE-REPORT:

• White cursor-sized blocks in claude output over mosh — reproduction
  needs lossy networks; a lab rig for that is in progress.
• herdr over mosh is still the TUI underneath (zoomed) — native frame
  rendering rides the roaming bridge, which is the next big piece.
• A phone-sized herdr TUI client can narrow the desktop TUI's layout —
  herdr sizes shared clients together (their issue #2404/#2405).
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
