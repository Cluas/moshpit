# TestFlight · 1.0.0 (379)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only. 378 fixed half of the keyboard transition and broke the
Windows sheet doing it; this is the other half plus the repair.

---

379 finishes what 378 started, and undoes the damage 378 did on the way.

WHAT'S NEW OVER 378:

• The jump at the END of a keyboard transition is gone. 378 slid the
  terminal by the keyboard's height, which is the wrong distance: when
  the buffer shrinks, SwiftTerm pops the blank lines below the cursor
  first and only scrolls once they run out, so the content usually
  moves a fraction of that — and on a screen with room to spare it does
  not move at all. Measured on a real pane: the content owed 88 points
  of movement and 378 gave it 301, so the resize at the end yanked back
  the other 213. That is the jolt. The slide is now computed with the
  same arithmetic the resize will use, in both directions, so the
  moment the hold releases there is nothing left to move.
• The Windows / Sessions / Pane sheet no longer leaves the screen above
  it blank. 378 pushed the pane to the bottom of its container while a
  sheet was up — which was invisible in testing because the assumption
  was that a sheet covers the whole screen. It does not: its top edge
  sits around 40% down, so the top HALF went black behind the list.
  A sheet now freezes the pane exactly where it is, position and size
  both, and only a keyboard transition moves it.

STILL TRUE FROM 378 (please keep watching these):

• Opening and closing the keyboard costs ONE resize now, not three: a
  third of a point of layout jitter was flipping the column count from
  70 to 71 and back, and a width change re-wraps every line.
• That jitter was never keyboard-specific, so it remains the leading
  candidate for the "picture jumps while output is streaming" report.

TEST:

• Open and close the keyboard over a pane that is FULL of output, and
  again over one with only a couple of lines on it. Both should glide,
  and neither should snap at the end. The sparse one should barely move
  at all — that is correct, not a bug.
• Open the Windows sheet with the keyboard up: the terminal visible
  above the sheet must still show its content, not a black band. Close
  it: the keyboard comes back and nothing re-wraps.
• THE IMPORTANT ONE, still: leave an agent working, watch a long stream
  of output for a minute or two, keyboard up and keyboard down. If the
  picture still jumps, say whether the WHOLE screen jumps or only a
  line or two shifts — those point at completely different causes.
• 377's checks stand: agent switching on SSH + herdr should be near
  instant and must never show the agent you came from.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.
• mosh + herdr switching is still around a second on a high-latency
  link: each switch opens a fresh SSH channel to steer the host-side
  renderer.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
