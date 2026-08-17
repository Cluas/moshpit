# TestFlight · 1.0.0 (352)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

352 fixes a crash and three mosh/herdr behaviours reported on 351 — all
found from your feedback logs. **334 is the build in App Review** —
TestFlight only.

---

## The herdr-upgrade crash is fixed — and the black screen explains itself

If you upgraded herdr on your host while its old server kept running,
connecting showed a black pane and one tap crashed the app. Both halves
are fixed: the tap is harmless now, and the version conflict shows up as
a banner in the terminal saying exactly what's wrong — with a **Restart
herdr server** button that fixes it in one tap. (It stops the old server
first, which exits whatever ran in its panes — that's why it asks
instead of just doing it.)

Please try, if you use herdr: upgrade herdr on the host without
restarting its server, connect, read the banner, tap the button. Worth
reporting: a black herdr pane with NO banner, or the button claiming
success while the panes never come back.

## Coming back to a terminal shows the terminal

Leaving a session and re-entering the same pane used to show an empty
screen with a lone cursor (mosh especially — it only ever sends what
changed, and it didn't know the app had thrown its copy away). The
screen you come back to is now the screen you left.

Please try: connect over mosh, go back Home, tap straight back into the
same pane. Worth reporting: any blank re-entry, or stale content that
never catches up.

## mosh survives bad networks like it's supposed to

A brief network failure used to freeze the session permanently — the
screen stopped updating and only a full manual reconnect (with the
connecting animation mosh exists to avoid) brought it back. Transient
failures now recover on their own within a second or two.

Please try: mosh session up, flip airplane mode on and off, keep using
the session. Worth reporting: a frozen screen that never resumes, or
keystrokes lost after recovery. Note the FIRST connection still takes
the full SSH handshake — mosh's speed is in staying alive, not in
dialing.

## Known issues — please don't re-report these

1. On phones narrower than a Max, the last chip in the default shortcut
   row sits half-visible at the right edge. Deliberate — the row scrolls.
2. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
4. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. iOS suspends the app; no push channel yet.

## Reporting

Screenshot through TestFlight, or share it straight to your agent.
Please include: device model, iOS version, SSH or mosh, **tmux or
herdr** (and herdr's version if herdr).
