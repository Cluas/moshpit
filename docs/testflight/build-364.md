# TestFlight · 1.0.0 (364)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

364 fixes the one bug underneath three of 363's remaining reports.

THE WEDGED PROBE (WHY 363'S SELF-HEALING SOMETIMES NEVER RAN):

• Every liveness probe runs under a timeout. The timeout RACED the
  probe but then waited for it anyway — and a probe stuck on a
  half-open socket (a long background's silent NAT death, no RST)
  ignores cancellation and never finishes. One stuck probe wedged an
  internal guard for the rest of the session, which silently blocked
  every keepalive, sidecar rebuild, reconnect, and protocol switch
  that came after. That is why, on some devices, 363's "breadcrumb
  comes back within 15s" never happened, herdr-over-SSH went weird
  after a long background, and switching a connection between mosh
  and SSH stopped working.
• The timeout now returns unconditionally at its deadline; a stuck
  probe is abandoned to die with its dead connection instead of
  taking the session's whole recovery machinery hostage. This is
  covered by a contract test that fails on the old implementation.

RETEST ALL THREE, ON THIS BUILD:

• mosh + tmux → background 10+ minutes (or airplane-mode a minute) →
  return: breadcrumb and pane switching must self-heal within ~15s.
• herdr over SSH → same long background → return: typing, pane
  switching and agent status must all resume by themselves.
• Edit the connection and switch protocol mosh ↔ SSH a few times:
  every switch must land in a working session.

EVERYTHING 363 SHIPPED STILL APPLIES — white blocks root-fixed (CJK
panes + heavy pane switching should never show them), working Repaint,
takeover storms gone.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
