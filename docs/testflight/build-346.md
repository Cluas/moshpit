# TestFlight · 1.0.0 (346)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

346 is 345 plus one fix: the notification-tap crash. **334 is the build in
App Review** — this one is TestFlight only.

---

## The notification-tap crash is fixed

Entering Moshpit by tapping a notification could kill the app on the way in
(344's top crash — thank you to the tester who sent the report with steps;
that crash log is exactly what found it). The notification handler was
answering the system from the wrong thread; it now answers from the main
thread, which is the one iOS insists on.

Please try, with the app backgrounded: wait for an agent notification (or
any Moshpit notification) and enter the app by tapping it. Repeat a few
times — the crash was timing-dependent. Worth reporting: ANY crash on the
way in from a notification, and please say whether the app had been in the
background for seconds or minutes.

## Carried over from 345 — still worth exercising

- **Copy a screenshot, tap paste** → uploads and inserts the path.
- **Long-press the photo chip** → re-insert any earlier upload by number.
- **Reconnects** should play with zero black flashes.
- Settings ▸ Behavior ▸ **Remote Clipboard Read** (off by default).

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
2. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
3. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. Known limitation — iOS suspends the app and there
   is no push channel yet; it is honest rather than frozen.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for anything about
notifications, whether the app was foreground, backgrounded, or killed when
the notification arrived.
