# TestFlight · 1.0.0 (351)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

351 is a certification build: it is the first one cut from a fully green
automated test run (592 unit tests + the UI test suite, which had been
dormant since the rebrand). If you already exercised 350, the app behaves
identically — the recap below is for testers coming from 349 or earlier.
**334 is the build in App Review** — TestFlight only.

---

## If you're coming from 349 or earlier — what 350 fixed

**Reconnecting no longer types into your agent.** Stray touches during a
reconnect (a thumb on the arrow keys, a swipe on the covered screen) used
to be queued and replayed into the pane after the session came back —
the classic symptom was Claude Code's prompt history popping open on its
own. Input is now dropped while the connecting screen is up; lock-screen
Allow/Deny and queued share-sheet images still deliver.

**The shortcut keys are sharp again.** Two rendering bugs made thin-stroke
keys (^C and friends) look blurry: the row rested on half-pixels after a
scroll, and the edge fade sat on top of whichever key straddled it. The
row now rests on whole pixels and clips crisply — a partially visible
key means "scroll for more".

Please try, if you haven't: mash the shortcut bar during a reconnect
(nothing should reach the agent), and eyeball ^C after scrolling the bar
around.

## What 351 itself changes

Nothing user-visible. Under the hood every build is now gated on the
full automated suite — including UI flow tests for Add Connection, the
theme gallery, and the icon gallery that had quietly stopped running.
Worth reporting: anything that regressed versus 350 — there should be
nothing, and "nothing" is exactly what this build is certifying.

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

Screenshot through TestFlight, or share the screenshot straight to your
agent from the share sheet. Please include: device model, iOS version,
SSH or mosh, and **tmux or herdr**.
