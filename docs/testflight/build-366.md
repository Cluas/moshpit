# TestFlight · 1.0.0 (366)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

366 fixes herdr pane switching to panes that aren't running an agent.

TWO PANES IN A TAB, COULDN'T SWITCH TO THE SHELL ONE:

• Switching panes rode `herdr agent focus`, which historically moved
  the focus even while complaining the pane had no agent. Current
  herdr fixed their ordering: it now fails outright on a plain shell
  pane — focus doesn't move, so tapping the shell pane in Select Pane
  did nothing (verified against the live server: focused_pane_id
  stays put).
• The switch now carries a fallback in the same command: when agent
  focus refuses, a zoom bounce (--on then --off) moves the focus and
  restores the layout — both halves verified live. Agent panes keep
  the old fast path. (The socket's id-based pane.focus was rejected:
  the current server acts on it but never replies.)
• Test: a tab with an agent pane AND a plain shell pane, over both
  SSH and mosh. Switch back and forth from Select Pane; type after
  each switch. Both directions must land and take input. A desktop
  TUI on the same workspace will see a brief zoom flicker when
  switching TO a shell pane — that's the fallback working.

ALSO IN 366 (carried from 365, minutes older):

• Boot cleanup retires the immortal 360/361 renderer loops — one
  fresh mosh+herdr connect per host ends the "terminal attach taken
  over" fights and the wrong-pane-after-switch stealing for good.

EVERYTHING 363/364 SHIPPED STILL APPLIES — white blocks root-fixed,
working Repaint, probe timeouts that cannot wedge recovery, sidecar
self-heal after long backgrounds.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
