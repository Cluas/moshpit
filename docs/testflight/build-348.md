# TestFlight · 1.0.0 (348)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

348 is 347 plus one fix: scrolling Claude Code (and other full-screen
agents) no longer dies after the first swipe. **334 is the build in App
Review** — TestFlight only.

---

## Scrolling Claude Code actually works now

347 fixed how far a flick scrolls — and in doing so made a hidden misroute
reliable: after the first swipe in Claude Code, scrolling went dead
entirely (reported within the hour — thank you). Full-screen agents like
Claude Code keep no tmux history, and the scroller was parking exactly
those panes in a mode with nothing to show. They now always scroll through
the agent itself, verified against a live Claude Code session: repeated
hard flicks, top of the transcript and back, no dead swipes.

Please try, in a Claude Code pane: flick up hard several times in a row,
read something several screens back, flick back down, then type — the
prompt should respond immediately. Also worth a pass in vim and less.
Worth reporting: ANY swipe that moves nothing while the pane visibly has
more content, or keystrokes going missing right after scrolling.

## Still fresh from 347 — keep exercising

- Attach images on pure-mosh connections (no Face ID mid-flow).
- Failed uploads show Retry; retry keeps what already landed.
- Long-press the photo chip: camera, clipboard image, and re-insert by #N.
- Settings ▸ Attached Images: uploads expire on the server (default 7 days).

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
2. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
3. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. iOS suspends the app; no push channel yet.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for scroll reports,
what was running in the pane and whether it was mid-output when you swiped.
