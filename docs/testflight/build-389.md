# TestFlight · 1.0.0 (389)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. The breadcrumb stops forgetting where you are.

---

389 fixes the thing a reconnect did to the top bar.

WHAT'S NEW OVER 388:

• Reconnecting no longer replaces the session / window / pane
  navigation with the machine's address. The breadcrumb is built from
  an ATTACHED tmux tree, and an automatic reconnect rebuilds the
  controller from scratch — so for the seconds that took, the bar fell
  back to "user@host · tmux" and then snapped back to the real crumbs.
  Nothing about the session had actually changed: the windows are still
  on the server, we were simply not attached for a moment. Showing the
  machine's identity instead reads as "your session is gone" at exactly
  the moment you are worried it might be.
  It now keeps showing the last tree it had, dimmed and not tappable,
  until the new one arrives — and falls back to the address only when
  the connection is genuinely offline, where that IS the honest answer.

  Verified by killing the SSH session out from under a live tmux
  connection: the app logged two real drops (channel silent, then
  transport closed) while the crumbs stayed on session/window/pane
  throughout.

TEST:

• Get a tmux session up, then break the network (airplane mode for a
  few seconds, or walk out of wifi range). The top bar should keep
  showing where you are, greyed out, and come back to full strength
  when it reattaches. It should NOT flash the host address.
• When it really cannot reconnect, the address coming back is correct.

STILL FROM 387 AND 388:

• The keyboard transition is covered by a still image rather than
  rendered live — the flicker comes from the remote app's own redraw.
• Switching input method is not covered (it changes nothing, and
  covering it made the switch feel slow).
• Connection work: fewer redundant probes, a slow probe no longer kills
  a session, idle cost about 12 exec channels a minute down to 1.3.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
