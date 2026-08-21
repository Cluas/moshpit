# TestFlight · 1.0.0 (388)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. Takes the cost of 387's cover off the transitions that
never needed it.

---

388 fixes something 387 introduced: switching input method felt slow.

WHAT'S NEW OVER 387:

• Switching input method (globe key) is not covered any more. 387 put
  a still image over the pane for every keyboard notification, and an
  input-method switch fires one with an IDENTICAL frame on both ends —
  CN and EN are the same height, so the terminal never resizes and the
  remote never redraws. There was nothing to hide, and hiding it cost
  about half a second of frozen picture per switch. The cover is now
  only raised when the keyboard's frame actually moves.
• The cover also starts looking for its exit as soon as the keyboard
  lands, rather than a further quiet-window later, and the quiet it
  waits for is 150ms instead of 200ms. Both only matter in the case
  where the pane repaints quickly — it still will not come off while
  output is still arriving.

TEST:

• Tap the globe key a few times with the keyboard up. It should feel
  immediate — no frozen beat, no dimming.
• Then the thing 387 was for: tmux + Claude Code, keyboard up and
  down. The content must not blink or blank.
• A CN keyboard that is TALLER than the EN one (candidate bar) is a
  real resize and WILL be covered — that is correct, and should look
  like the keyboard show/hide does.

STILL FROM 387 AND 386:

• The keyboard transition is hidden behind a still image rather than
  rendered live, because the flicker comes from the remote app's own
  redraw (tmux's copy of a Claude Code pane loses its footer for ~50ms
  on resize, and our capture used to land inside that window).
• Connection work: fewer redundant probes, a slow probe no longer kills
  a session, herdr's push channel running, idle cost about 12 exec
  channels a minute down to about 1.3.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.
• Model downloads pause while backgrounded.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
