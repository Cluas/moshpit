# TestFlight · 1.0.0 (390)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. Reading history no longer summons the keyboard.

---

390 fixes a small thing that gets in the way constantly: scroll up to
read something, tap, and the keyboard throws itself over the very
output you were reading.

WHAT'S NEW OVER 389:

• A tap that lands while you are reading back does nothing instead of
  raising the keyboard. The terminal focuses itself on any tap it
  receives while unfocused — and on iOS, focus IS the keyboard — so
  there was no way to touch the screen mid-scroll without losing half
  of it. A tap within two seconds of a scroll is now read as "still
  reading" and swallowed.
• A SECOND tap goes through immediately. The first says "I'm reading";
  another right after says "no, I really do want to type", and
  swallowing that one too would just feel broken.
• Nothing else changes: tap on a terminal you have not been scrolling,
  and the keyboard comes up exactly as before. The shortcut bar's
  keyboard button is always available either way.

  Verified on device-shaped runs: scroll, tap → keyboard stays down;
  tap again → keyboard up; tap with no prior scroll → keyboard up.

STILL FROM 387 THROUGH 389:

• The keyboard transition is hidden behind a still image rather than
  rendered live — the flicker comes from the remote app's own redraw.
• Switching input method is not covered (nothing changes size, and
  covering it made the switch feel slow).
• Reconnecting keeps the session / window / pane breadcrumb instead of
  falling back to the machine's address.
• Connection work: fewer redundant probes, a slow probe no longer kills
  a session, idle cost about 12 exec channels a minute down to 1.3.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
