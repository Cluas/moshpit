# TestFlight · 1.0.0 (382)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. The keyboard work again, this time at the real cause.

---

382 fixes what 379 and 381 were circling. Screenshots of the keyboard
coming up showed Claude Code's bottom two rows — the separator and the
"bypass permissions" line — simply GONE for a beat, then reappearing
with a jolt. That was never an animation problem.

WHAT'S NEW OVER 381:

• Raising the keyboard no longer deletes the rows under the cursor.
  Shrinking a terminal buffer is lossy: rows are removed by popping the
  lines BELOW the cursor first, and a full-screen TUI keeps its own
  furniture down there. Claude Code's input box sits two rows above its
  own footer, so the keyboard resize threw that footer away, and only
  the remote repaint — one round trip later — could put it back. That
  gap is what you were seeing.
  The local buffer is no longer resized until the repaint that replaces
  what the resize destroys is actually in hand. The two land in the
  same turn: resize, then paint the server's full screen over it, with
  nothing drawn in between. Measured against a TUI shaped exactly like
  Claude Code's: the footer is on screen in 258 of 269 recorded frames,
  and the one gap left is the moment the RISING KEYBOARD covers it, not
  the app losing it.
• Putting the keyboard away goes through the same gate. 381 grew the
  terminal early and filled the new rows with a local guess — old
  scrollback above, blanks below — which the remote repaint then had to
  rearrange, and the rearranging was its own visible step. One rule now,
  both directions: the buffer does not change size until the repaint
  that matches it has arrived.
• Transports that never repaint on resize (a plain shell has no reason
  to) opt out and keep 381's behaviour, which for them is the best
  available. There is a 1.2s timeout either way, so a silent remote
  costs a late resize, never a frozen screen.

TEST:

• Open and close the keyboard over a working Claude Code pane, several
  times. Its footer should never disappear, and nothing should
  rearrange itself after the keyboard settles.
• Same over a plain shell, and over a pane with only a few lines on it.
• Type while doing it — the text you are entering must survive.
• 380's connection work is still the thing that needs a day of real
  use: it should reconnect LESS on a bad network.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it: herdr's direct attach is
  exclusive per terminal, so whoever attaches last evicts the other and
  you get "Another client is using this pane". Working as designed for
  now; the better answer is to fall back to herdr's read-only observe
  mode instead of fighting, which is not written yet.
• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• herdr's control plane still opens one exec channel per poll, about 7
  a minute at idle. Fix written and tested, deliberately not in this
  build.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr, and
roughly when it happened.
