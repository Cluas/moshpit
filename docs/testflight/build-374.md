# TestFlight · 1.0.0 (374)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

374 finishes what 373 started, proven this time against a real tmux
in a purpose-built lab: after any size war, the phone's screen must
CONVERGE to tmux's — and SSH+tmux scrollback moved onto the phone.

WHAT CHANGED:

• Scrolling a plain-shell pane in SSH+tmux no longer drives tmux
  copy-mode. Measured against tmux 3.6a: a control-mode client
  receives ZERO output while copy-mode scrolls — we were scrolling
  a screen the phone can never see, and dragging any desktop client
  on the same window into copy-mode with us. Scrollback now reads
  from the phone's own buffer, like plain SSH: new output holds
  while you read and replays when you return to the bottom; typing
  snaps you back to live.
• 373's output gate now stays closed until the repair repaint has
  actually landed. The first bytes after a size flap can be the
  tail half of an escape sequence cut at the flap, and painting
  them wrote literal "5;210m" fragments into the grid.
• A repair repaint captured at the desktop's size is now rejected
  by width as well as height, and a rejection triggers a fresh
  capture — before, two unlucky captures in a row starved the
  repair, leaving stale cells that refused to scroll with the rest.

TEST (needs a desktop terminal attached to the same tmux session):

• Same war dance as 373, now with reading: scroll a busy pane,
  app-switch away and back, scroll again — repeatedly. Everything
  rendered must be laid out for the phone: no shredded fragments,
  no literal escape-code text, no cells left behind while the rest
  scrolls.
• Scroll up a quiet shell pane, read some history, then type — the
  view must snap back to the live prompt with your keystroke echoed.
• The desktop terminal on the same session must never get yanked
  into copy-mode by phone scrolling anymore.
• Without a desktop client attached everything should look
  identical to 373.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
