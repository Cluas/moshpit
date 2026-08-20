# TestFlight · 1.0.0 (381)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. One fix: the last flash left in the keyboard transition.

---

381 finishes the keyboard work. 379 got the slide right and 380 left it
alone; what remained was a flash at the end of PUTTING THE KEYBOARD
AWAY, and it turned out not to be the slide at all.

WHAT'S NEW OVER 380:

• Dismissing the keyboard no longer flashes. The slide itself was
  landing exactly where it should — logged against a real pane, the
  content was predicted to move 8 rows and moved exactly 8 rows, both
  directions. The flash was the space the keyboard uncovered: the
  terminal kept its old, smaller size for the whole animation, so that
  space stayed empty until the very last moment and then filled in one
  step. It now takes its new size AT THE START of the animation, while
  the keyboard is still moving, and is offset by exactly what the
  resize moved the content so nothing appears to jump. Frame-by-frame:
  the uncovered space already has text in it while the keyboard is
  halfway down.
• Raising the keyboard deliberately keeps the old behaviour (hold the
  size, slide). Resizing early in that direction would open the same
  empty band at the TOP instead — the space being taken away is under
  the rising keyboard, where nobody can see it.

TEST:

• Open and close the keyboard repeatedly, on a pane FULL of output and
  on a pane with only a few lines. Watch the moment the keyboard leaves
  the screen: the space it frees should already have terminal in it,
  and nothing should snap at the end.
• Switch input methods (globe key) where the two keyboards differ in
  height.
• 380's connection work is still what matters over a day of real use:
  it should reconnect LESS on a bad network. If it drops, note roughly
  when — the log names the cause now.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• mosh + herdr switching is still around a second on a high-latency
  link: each switch opens a fresh SSH channel to steer the host-side
  renderer.
• herdr's control plane still opens one exec channel per poll — about
  7 a minute at idle, measured. The fix is written and tested but is
  NOT in this build: it turns on a push channel that has never actually
  run before, and that deserves its own build to be blamed for.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr, and
roughly when it happened.
