# TestFlight · 1.0.0 (387)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. The keyboard flicker, finally diagnosed — against real
Claude Code this time, not a TUI I made up.

---

387 stops showing the keyboard transition at all.

WHERE THE FLICKER ACTUALLY COMES FROM:

Not from our renderer. Measured on the host against a real Claude Code
pane: when the window resizes, tmux's OWN copy of the pane loses its
footer for about 50ms while the app redraws itself. Our immediate
capture-pane lands inside that window — so we faithfully paint a screen
the app had half-erased, and the corrected one only arrives with the
settled pass ~700ms later. That 700ms is the blink.

Which is why 382 through 386 changed nothing: they all held or reordered
the local resize, and the repaint being waited for was itself the broken
frame.

WHAT 387 DOES:

• The transition is covered by a still image of the pane, taken before
  anything moves, and taken down once the pane has stopped repainting
  underneath (quiet for 200ms, capped at 1.6s). The resize, the app's
  redraw and the settling captures all happen behind it. Recorded at
  30fps over a real Claude Code pane: the content never disappears —
  before, it went blank the instant the keyboard rose and faded back
  half a second later.
• The cover does NOT slide. That was the first attempt and it made
  things worse: sliding it to follow the keyboard assumes the content
  is bottom-anchored, and a TUI's is not — Claude Code pins its footer
  to the bottom but keeps the conversation at the top, so the slide
  pulled blank filler into view and looked exactly like the content
  vanishing. No rigid motion can stand in for a reflow. It holds still
  and cross-fades instead.

TEST:

• tmux + Claude Code, keyboard up and down, repeatedly: the content
  must not blink, blank, or jump. The footer arriving at its new place
  as the cover fades is expected.
• A pane with very little on it, and a plain shell: same.
• If it still flickers, say so — the cover can only hide what happens
  under it, and something else would be at fault.

STILL FROM 386:

• Connection work: fewer redundant probes, a slow probe no longer kills
  a session, herdr's push channel running for the first time, idle cost
  about 12 exec channels a minute down to about 1.3.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
