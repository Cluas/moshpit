# TestFlight · 1.0.0 (367)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

367 fixes up/down history navigation in zsh panes under herdr.

WHY ARROWS DID NOTHING IN ZSH OVER HERDR:

• Arrow keys come in two dialects (normal vs application cursor mode),
  and the app picks by asking its LOCAL terminal engine which mode the
  pane is in. Over herdr that engine only ever sees screen repaints —
  mode changes never reach it — so the app always sent the normal-mode
  dialect. zsh's line editor switches the pane to application mode,
  and history widgets bound the terminfo way only answer to THAT
  dialect: up/down fell on deaf ears. (tmux panes were fine — they
  feed the engine the raw byte stream, modes included.)
• Fix: over herdr, arrow keys now travel as SEMANTIC keys
  (`pane send-keys up`), which herdr encodes against the pane's REAL
  modes server-side. Verified live against zsh: up recalls, down walks
  back. Covers the on-screen D-pad AND hardware-keyboard arrows, on
  both SSH and mosh.

TEST:

• herdr pane running zsh (SSH and mosh): tap ↑ — the previous command
  must appear; ↓ walks forward; works the same mid-word with
  history-substring plugins. Arrows inside vim/htop/claude must also
  keep working (they ride the same semantic route).
• Expect a hair more latency per arrow than plain typing (each arrow
  is a control-plane command) — report if holding an arrow feels
  unusable rather than merely paced.

EVERYTHING 366 SHIPPED STILL APPLIES — shell-pane switching fallback,
immortal-renderer cleanup, white blocks root-fixed, probe timeouts,
sidecar self-heal.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
