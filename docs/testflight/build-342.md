# TestFlight · 1.0.0 (342)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

342 is 336 plus the arrow-key work. **334 is the build in App Review** — this
one is TestFlight only, so anything found here is still in time for 1.0.1.

---

## New in this build: the arrow key stops making you hold it

Sending one arrow used to cost a press, a hold-still, and a drag. A plain tap
sent nothing, so nudging the cursor one column meant pressing the arrow pad and
pushing it — every single time.

The hold-still was there for a real reason. With the keyboard collapsed the key
row sits about a finger's width above the home indicator, close enough that
swiping the app away can *start* on the arrow pad — and that used to type a real `←` on
its way off screen, which is why Claude Code would jump to its session switcher
by itself when you backgrounded the app.

That can only happen with the keyboard **down**. With it up, the row rides above
the keyboard, nowhere near the swipe. So the wait is gone in that case, and the
push now registers on about a third less travel.

Please try, with the **keyboard up**:

1. **Nudge the arrow pad a couple of millimetres and let go.** One arrow, immediately —
   no perceptible pause before it takes. Do it in a shell where you can see the
   cursor move.
2. **Push and keep holding.** It should still repeat like a held-down key.
3. **Push through a word left, then back right.** The direction should follow
   your thumb without dropping keys or firing the wrong axis.

Then with the **keyboard down**:

4. **Same nudge.** There is still a brief settle here — deliberate, about 50ms.
   It should feel like a key that responds, not one you have to wait on. If it
   feels like a key that misses presses, that is worth reporting.
5. **Swipe the app away, starting your swipe right on the arrow pad.** Nothing should be
   typed. If your agent jumps to a session switcher or your shell recalls a
   command, that is the old bug back and we want to hear immediately.

## Also new, both switched off by default

Two things you have to add yourself from **Settings ▸ Shortcuts**:

- **`←↑↓→` "Tap arrows"** — four arrow keys in one control: tap sends one arrow,
  hold repeats it. A tap on the arrow pad can never send a direction (a tap has no
  direction to read), so this is for you if you would rather tap than push. It
  is about two and a half keys wide, which is why it isn't in the row already —
  adding it will push something off the end, and the row scrolls sideways.
- **`←` and `→` as single keys.** These never existed before; only `↑` and `↓`
  did. Handy if you only want one direction in the row.

Your existing key row is untouched — nothing moved, nothing was removed.

## Still worth checking (unchanged from 336)

- **Scrolling while an agent is answering.** Swipe up mid-answer in a tmux pane;
  the view should stay where you put it. **herdr is still NOT fixed here** — only
  tmux — so please say which one you were on.
- **SSH key file import.** Keys ▸ add ▸ Import, public-then-private in that order.
- **⇥⏎.** With Claude Code offering a completion, the chip should accept AND send.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (and resumes when you come
   back). Deliberate.
2. **The lock-screen Live Activity padding** has never been verified on a real
   device.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for anything about the
arrows, **whether the keyboard was up or down** — that is the one thing that
changes which code path you were on.
