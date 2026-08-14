# TestFlight · 1.0.0 (343)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

343 is 342 plus the background/reconnect work. **334 is the build in App
Review** — this one is TestFlight only, so anything found here lands in 1.0.1.

---

## New in this build: backgrounding and coming back

Three changes, all on the same path — leaving the app and returning to it.

**1. A reconnect is one colour now.** A dropped line used to show three
different stories at once: blue "riding the handoff" on the terminal, amber
"WAIT" on the home card, and partway through the SAME reconnect the terminal
flipped to red "line dropped — retrying" while the app was actively dialling.
Now: blue means reconnecting (the app is dialling), red means offline (it has
given up, nothing is in flight), and every surface — terminal screen, transport
pill, home card — says the same thing at the same time.

Please try: drop your connection (airplane mode for a few seconds, or switch
Wi-Fi ↔ cellular on plain SSH) and watch the reconnect. Worth reporting: any
surface disagreeing with another, red showing while it's still retrying, or a
colour flip mid-reconnect.

**2. Backgrounding hands your tmux windows back properly.** Switching away
queues server-side cleanup (returning window sizing to your desktop), and iOS
could suspend the app before those commands reached the server — a desktop
attaching later got phone-width windows. The cleanup now finishes before the
app sleeps.

Please try, if you use the same tmux server from a desktop: open a session in
Moshpit, background the app (don't kill it), then attach from your desktop.
The windows should be desktop-sized. This was the "my terminal is 40 columns
wide" complaint — if you still see it, we want to know within the hour.

**3. Multiple connections come back together, with one Face ID.** Returning to
the foreground used to resume connections one at a time — the second one sat
dead until the first finished, up to ~20s each. They now resume concurrently,
and simultaneous reconnects to the same host share a single Face ID prompt
instead of stacking sheets.

Please try, with two or more connections live: background the app for half a
minute, come back. Both should start reconnecting immediately. Worth
reporting: a connection that waits for another before starting, or two Face ID
prompts back-to-back for one return.

## Also in this build

- A malformed mosh handshake (the `MOSH CONNECT` line carrying a number that
  can't be a port) now falls back to SSH with a notice instead of crashing at
  connect time. If you ever saw an instant crash when opening a mosh
  connection to a host with a chatty login banner, that was probably this.
- Arrow keys: tap-first arrows (`←↑↓→`, one tap = one arrow) and single `←`/`→`
  chips are available in Settings ▸ Shortcuts. The joystick stays the default
  and got faster: with the keyboard up it engages on touch instead of after a
  hold, and the push distance is a third shorter everywhere.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
2. **The lock-screen Live Activity padding** has never been verified on a real
   device.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
4. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. Known limitation — iOS suspends the app and there
   is no push channel yet; it is honest rather than frozen.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for anything about
reconnects, **whether the app had been backgrounded and roughly how long** —
under vs over ~20 seconds takes different code paths.
