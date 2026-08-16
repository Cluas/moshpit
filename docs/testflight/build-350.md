# TestFlight · 1.0.0 (350)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

350 is a fix build for two things testers reported on 349. **334 is the build
in App Review** — TestFlight only.

---

## Reconnecting no longer types into your agent

Stray touches during a reconnect — a thumb brushing the arrow keys, a
swipe on the covered screen — used to be delivered to the pane AFTER the
session came back (mosh queues keystrokes typed while disconnected and
replays them). The classic symptom: Claude Code's prompt history popping
open on its own right after a reconnect. Input is now dropped while the
connecting screen is up; the lock-screen Allow/Deny buttons and queued
share-sheet images still deliver, because those reconnect first on
purpose.

Please try: while the reconnecting screen is up, mash the shortcut bar's
arrows and swipe the covered terminal — nothing should reach the agent
once the session returns. Worth reporting: anything typed during a
reconnect that still shows up afterwards, or a lock-screen Allow that
stops working.

## The shortcut keys are sharp again

Two rendering bugs made some keys (^C and friends) look blurry: the row
rested on a half-pixel after you scrolled it, and the edge "fade" hint
sat on top of whichever key straddled the edge. The row now rests on
whole pixels and clips crisply at the edges — a partially visible key
means "scroll for more", same as everywhere else in iOS.

Please try: scroll the shortcut bar back and forth, let it rest at a few
positions, and look closely at the thin-stroke keys (^C, arrows). Worth
reporting: any key that still looks soft or smeared at rest, on any
zoom/text-size setting.

## Known issues — please don't re-report these

1. On phones narrower than a Max, the last chip in the default shortcut
   row sits half-visible at the right edge. Deliberate — the row scrolls,
   and the half chip is the hint.
2. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
4. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. iOS suspends the app; no push channel yet.

## Reporting

Screenshot through TestFlight, or send it directly — the share sheet now
delivers screenshots straight to your agent, which is also a fine way to
test it. Please include: device model, iOS version, SSH or mosh, and
**tmux or herdr**.
