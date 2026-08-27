# TestFlight · 1.0.1 (393)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

All groups. The touch release: reading a terminal is now a first-class
activity — taps, selection and long-press all behave like the rest of
iOS — plus a big privacy upgrade to push, and real iPad fixes.

---

A TAP IS A TAP:

• Tapping the terminal no longer summons the keyboard. Tap the input
  area — the cursor's own rows, like Claude Code's prompt box — and
  the keyboard comes up AND the cursor lands where you tapped. Tap
  anywhere else and it is just a tap: links open, history stays put.
• Buttons drawn by the app in the pane work with the keyboard down:
  Claude Code's "jump to bottom (click)" is one tap now, not a
  keyboard first and a second tap after.
• Long-press selects the word under your finger — keep holding and
  slide to extend, lift to get Copy. It used to summon the keyboard.
• Selection handles are finger-sized at every font size (they used to
  shrink with the font), the Copy menu shows without the keyboard,
  and a tap anywhere dismisses the selection.
• The keyboard toggle on the shortcut bar is the explicit way to
  bring the keyboard up; "raise keyboard on open" in Settings still
  works. With a hardware keyboard, tap the toggle once to focus.

PUSH: THE RELAY NOW STORES NOTHING:

• The push relay is stateless. It used to keep a small routing table
  (which device a paired host may wake); now every alert carries its
  own sealed routing and the relay verifies and forwards without
  keeping anything at all. Nothing to lose, nothing to hand over.
• Re-pairing is automatic: connect to each host once after updating
  and Moshpit refreshes the host's config itself.
• "Show detail on lock screen" and "Alert sound" now apply to pushed
  notifications too — off keeps the question off the lock screen even
  with the app closed. (They previously only governed local alerts.)

IPAD:

• Landscape trims the app's own chrome (slimmer top bar and shortcut
  bar) and drops the system assistant strip that rode above the
  keyboard — together roughly 80pt handed back to the terminal on an
  iPad mini with the keyboard up.
• Switching to a new pane repaints noticeably less line-by-line on
  big grids; more of this to come.
• The camera and dictation hand the keyboard back the way they found
  it — expanded stays expanded, collapsed stays collapsed.

WORTH KNOWING:

• After updating, open each paired host once so the push config
  refreshes. Until the new relay is live, brand-new pairings may show
  "Relay refused to mint" — existing notifications keep working.
• Long-press then slide extends the selection like iOS text; the
  drag-to-scroll gesture is unchanged (selection wins only while a
  selection is active).

WHAT TO POKE AT:

• Read scrollback, tap around, tap links: the keyboard should never
  appear unless you tapped the input area or the toggle.
• Double-tap or long-press agent output: select → drag handles →
  Copy → tap to dismiss. All with the keyboard down.
• iPad mini landscape with the keyboard up: is the terminal usable?
• Turn "Show detail on lock screen" off, have an agent ask something
  with the app closed: the lock screen should name the agent and
  host, never the command.
