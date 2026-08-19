# TestFlight · 1.0.0 (375)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

375 closes two escape hatches 374's repair gate still had — both
found by re-auditing the diffs against the protocol lab's own
measurements, both able to reproduce "still broken" on a device.

WHAT WAS STILL WRONG IN 374:

• The gate reopened on the FIRST repaint after a size flap. That
  repaint is taken deliberately early and can be a cropped stale
  image that passes every size check — so the gate held for one
  round trip instead of until the real repaired frame, and the pane
  app's still-draining desktop-width bytes (up to ~0.7s of them,
  often starting mid escape sequence) painted the same old garble.
  Now only the settled repaint reopens the gate.
• If the size war ended with the DESKTOP holding the width — easy:
  our anti-flicker backoff swallows a re-pin right as the desktop
  goes quiet — nothing ever brought the width back. The gate stayed
  closed forever: the pane froze on its last frame, only the agent
  bell got through. Now a quiet spell (2s without a size change)
  forces one re-pin and the normal repair runs.

TEST (desktop terminal on the same tmux session):

• The same war dance: scroll a busy pane, app-switch away and back,
  type on the desktop, stop typing, watch the phone. Within ~2s of
  the desktop going quiet the pane must repaint phone-width and
  resume live updates — no permanent freeze, no shredded fragments,
  no literal escape-code text.
• Type on the desktop CONTINUOUSLY for ten seconds: the phone may
  hold a still frame while the desktop owns the size (that is the
  design), but must recover on its own once the typing stops.

IF YOUR SERVER SAW BUILDS BEFORE 372: run "tmux list-clients" there
and detach stale phone-sized clients (or restart the tmux server)
once — pre-372 builds leaked clients that keep fighting for the
window size and will make ANY build look broken.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
