# TestFlight · 1.0.0 (378)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only — do NOT promote until the keyboard transition and a long
agent run have both been watched on a real device.

---

378 is one screenful of change: how the terminal behaves when the
keyboard opens and closes, and one bug found while measuring it that
was never about the keyboard at all.

WHAT'S NEW OVER 377:

• The terminal now moves WITH the keyboard instead of being repainted
  after it. Measured at 60fps before this: the keyboard glided for
  fifteen frames and the content teleported 242 pixels in one. Two
  causes, both fixed. The frame was held anchored at its top, so a
  rising keyboard clipped away the bottom of the screen — the prompt,
  the cursor, the line being typed — and the resize at the end slammed
  it back into view; it is anchored at the bottom now, so the part you
  are actually looking at does not move at all and the space opens and
  closes above it. And the move itself now runs on the keyboard's own
  duration and curve rather than landing in a single layout pass.
  Only the position is animated, never the size, so nothing reflows
  mid-slide.
• A third of a point was costing two full-screen repaints per
  keyboard toggle. SwiftUI hands the terminal widths that oscillate
  between 402.0 and 402.333 across consecutive layout passes, and that
  fraction is enough to move the column count from 70 to 71 and back.
  Each flip is a whole-buffer reflow, a resize sent to the host, and a
  repaint — and a WIDTH change re-wraps every line, which is the ugly
  kind. The grid is computed from whole points now: one keyboard
  toggle, one resize, down from three.

  This one is worth watching beyond the keyboard. The fractional width
  was never keyboard-specific — anything that re-renders the screen
  could land on it, and while an agent is working the screen re-renders
  constantly (the control poll, the breadcrumb following the pane
  title, the agent status dot). That is a strong candidate for the
  "picture jumps while output is streaming" report, which could not be
  reproduced any other way.

TEST:

• Open and close the keyboard, repeatedly, over a pane with content on
  it. The text should slide with the keyboard, not cut. Nothing should
  re-wrap, and the line you were looking at should stay where it is.
• Switch input methods (globe key) with CN/EN keyboards of different
  heights — same expectation.
• THE IMPORTANT ONE: leave an agent working and watch a long stream of
  output for a minute or two, keyboard up and keyboard down. Report
  whether the picture still jumps — and if it does, whether the whole
  screen jumps or only a line or two shifts. Those two point at
  completely different causes.
• 377's checks still stand: agent switching on SSH + herdr should be
  near-instant and must never show the agent you came from; the
  Windows sheet must not re-wrap the pane.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• mosh + herdr switching is still around a second on a high-latency
  link: each switch opens a fresh SSH channel to steer the host-side
  renderer.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
