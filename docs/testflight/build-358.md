# TestFlight · 1.0.0 (358)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

358 is about herdr and about the island finally telling the truth over
mosh. Three things to hammer:

THE ISLAND NOW WORKS OVER MOSH + TMUX:

• Live Activity, lock-screen alerts and the Agents section were never
  wired up on mosh+tmux connections — the tracking hook sampled a
  controller that does not exist yet on that transport. It now waits
  for the control plane to come up (up to ~15s of sidecar handshake)
  and wires itself the moment it exists.
• Test: connect over mosh with tmux, run an agent, background the app.
  The island pill and lock-screen alerts should behave exactly like
  they do over SSH. Worth reporting: an island that stays dark for a
  connection whose Agents section shows a working agent.

HERDR CONTROL PLANE GOES PUSH (SSH AND MOSH):

• On hosts with python3, the app now subscribes to herdr's event
  stream instead of only polling every 2-8s. Focus changes, new tabs,
  agent status flips made from your laptop should reflect on the phone
  near-instantly.
• No python3, or anything at all goes wrong → silently stays on
  polling, same behavior as 357. Nothing should ever look WORSE.
• Test: with the phone attached over herdr, rename/create/close tabs
  and flip agent states from the desktop. Report anything that takes
  more than ~2s to show up, and anything that stops updating after
  the app comes back from background.

SELECT PANE IS A LIST NOW:

• The pane picker used to draw a miniature of the split layout in a
  fixed-height board; four stacked panes physically overflowed the
  sheet and could not be scrolled or tapped. It is now a plain list —
  pane number, command, agent status dot — same shape as the Windows
  and Sessions sheets, scrolls at any pane count.
• Test: a window with 4+ panes, open Select Pane. Everything visible,
  everything tappable, active pane highlighted.

EVERYTHING 357 SHIPPED STILL APPLIES — the mosh socket-replacement
survival story; keep hammering background/lock/network-switch typing.

KNOWN ISSUES — DON'T RE-REPORT:

• White cursor-sized blocks in claude output over mosh — reproduction
  needs lossy networks; a lab rig for that is in progress.
• herdr over mosh still shows the full TUI, not native tabs — the
  roaming-bridge design that fixes this properly has landed in
  docs/design/roaming-transport.md; native frames ride it next.
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
