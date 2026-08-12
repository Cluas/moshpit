# TestFlight · 1.0.0 (324)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

---

## The main thing: the terminal is correct the moment it opens

Four separate faults made the first screen of a tmux session wrong, and all of
them ended the same way — you tapped the terminal, and everything snapped into
place. If tapping ever "fixes" the display in this build, that is a bug and we
want it.

Please try, on a session you already have running:

1. **Open a session and don't touch anything.** The screen should be complete
   and correctly wrapped straight away — not blank, not one character per line,
   and not wrapped a character or two short with letters spilling onto the next
   line.
2. **Do the same while your desktop terminal is attached to that same tmux
   session**, at a much wider window. This was the worst case: the window stayed
   at the desktop's width, the program drew to that width, and the phone
   hard-wrapped the result. Moshpit now claims the window for the phone as it
   connects.
3. **Swipe the app away and come back.** No mis-wrapped frame, and no black
   rectangle where the terminal should be. You should see the frame you left on,
   then it quietly updates to current.

## Swiping the app away no longer types into your session

With the keyboard collapsed, the shortcut bar sits close to the home indicator,
and iOS's swipe-up-to-background could begin on the arrow-key joystick — which
sent a real arrow key to the remote on the way out. In Claude Code `←` opens its
session switcher, so backgrounding the app appeared to navigate the agent by
itself.

The two drag chips (**✛** arrows and **⇅** scroll) now engage on *press then
push* rather than on any moving touch. Please check both still feel right: press
and push should send its key immediately and repeat while held. If a deliberate
push ever does nothing, tell us — that is the trade this change could get wrong.

## Tap to put the cursor where you tapped

A tap on the terminal now moves the cursor there in anything that understands a
mouse — Claude Code's prompt, vim, less. Reaching a character in the middle of a
long prompt is one tap instead of walking the arrow keys.

- The **first** tap on an unfocused terminal still just raises the keyboard.
  Positioning starts once it has focus.
- A plain shell prompt gets nothing on purpose (it would print the click as
  text). Only programs that asked for the mouse.

## Also

- Agent "needs your input" notifications should behave exactly as before. They
  are detected differently while backgrounded now — a burst of them, or none at
  all, is worth reporting.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (and resumes by itself when
   you come back). Deliberate trade: background transfers wedged the download
   permanently after the app was killed.
2. **The lock-screen Live Activity padding** has still never been verified on a
   real device. Report it if it looks cramped or absurdly empty.
3. **Send's keystroke order is still unverified on a real device.** If tapping
   **Send** in the voice preview inserts the text but does not submit it, report
   that immediately.
4. The home header can read **"1 live connection"** while the card below it says
   OFFLINE. Known, cosmetic.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, and tmux or herdr — those four save about half
the back-and-forth.

If the terminal still opens at the wrong width, one thing helps most: while the
wrong screen is up, **before touching the terminal**, run this on the server and
send the output —

    tmux list-clients -F '#{client_name} #{client_width}x#{client_height} #{client_flags}'
    tmux display -p '#{window_width}x#{window_height} pane=#{pane_width}x#{pane_height}'

That tells us in one line whether the window was never claimed, claimed at the
wrong size, or claimed correctly while the phone measured itself wrong.
