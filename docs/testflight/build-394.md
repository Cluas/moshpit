# TestFlight · 1.0.1 (394)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

All groups. One fix on top of 393: a link that wraps across two lines
now opens as the whole address. Everything from 393 still applies —
recap below.

---

WRAPPED LINKS OPEN WHOLE:

• Agents print long URLs that wrap onto a second line at phone widths.
  Tapping one used to open only the first line's half — a dead link.
  Now the address is stitched back together and tapping EITHER line
  opens the whole thing. Try it on any artifact/PR link Claude Code
  prints: both rows, same full address.
• A URL that legitimately ends at the right edge, with ordinary text
  on the next line, is left exactly as before — no false joins.

STILL IN THIS RELEASE (SINCE 393):

• A tap is a tap: the keyboard only rises when you tap the input area
  (the cursor's own rows) or the keyboard toggle. Everywhere else a
  tap just taps — links open, in-pane buttons like Claude Code's
  "jump to bottom (click)" work with the keyboard down.
• Long-press selects the word under your finger; keep holding and
  slide to extend; lift for Copy. Handles are finger-sized at every
  font size, the Copy menu shows without the keyboard, and a tap
  anywhere dismisses the selection.
• Push relay is stateless: every alert carries its own sealed routing,
  the relay stores nothing. Re-pairing is automatic — connect to each
  host once after updating.
• "Show detail on lock screen" and "Alert sound" now govern pushed
  notifications too, not just local ones.
• iPad landscape trims chrome (~80pt back with the keyboard up on an
  iPad mini) and pane switches repaint less line-by-line.
• The camera and dictation hand the keyboard back the way they found
  it.

WHAT TO POKE AT:

• Find a wrapped URL in agent output and tap each of its lines — both
  must open the full address, and the browser must land on a real
  page, not a 404.
• Read scrollback, tap around, tap links: the keyboard should never
  appear unless you tapped the input area or the toggle.
• Turn "Show detail on lock screen" off, have an agent ask something
  with the app closed: the lock screen should name the agent and
  host, never the question itself.
