# TestFlight · 1.0.0 (360)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

360 fixes both things 359's device pass found.

HERDR OVER MOSH: NO MORE SIDEBAR — RAW SINGLE-PANE STREAM:

• 359 zoomed the pane but the sidebar stayed: herdr's chrome lives in
  the shared server layout, and the phone's terminal is wider than the
  mobile-layout cutoff (64 cols) at smaller fonts. 360 stops rendering
  herdr's TUI over mosh entirely. The mosh screen now carries
  `herdr terminal attach` — the raw stream of exactly one pane, no
  sidebar, no header, and the pane sized to your phone alone (the
  desktop's layout is untouched, unlike the TUI route).
• Picking a pane / tab / workspace from the native sheets redirects the
  stream server-side within a second. The zoomed-TUI mode from 359
  remains as the automatic fallback for herdr older than 0.8.
• Test: mosh + herdr, multiple panes and tabs. Connect → should land
  full-screen on the focused pane with zero herdr chrome. Switch from
  all three sheets; type immediately after each switch. Worth
  reporting: any leftover sidebar, a switch that takes >2s, typing
  that lands in the wrong pane, or a black screen after switching.
• Also: switching focus on the DESKTOP herdr should redirect the phone
  within a poll (~2s, instant with push) — try it both directions.

SELECT PANE NAMES AGENTS, NOT VERSION NUMBERS:

• Claude Code's versioned install makes the pane's process name a bare
  version ("2.1.227"). The pane list now names the agent the same way
  the Home tree does, with the raw process name demoted to the meta
  column. Panes without a detected agent keep showing their command.

EVERYTHING 358/359 SHIPPED STILL APPLIES — island over mosh+tmux,
herdr push control plane, Select Pane list, background survival.

KNOWN ISSUES — DON'T RE-REPORT:

• White cursor-sized blocks in claude output over mosh — lab rig for
  lossy networks still in progress.
• Two Moshpits on the same pane will trade the raw attach back and
  forth (herdr's attach is exclusive); one phone per pane for now.
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
