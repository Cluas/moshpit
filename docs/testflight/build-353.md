# TestFlight · 1.0.0 (353)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

RULES for this section (learned the hard way):
- Under 4000 characters (TestFlight's limit).
- PLAIN TEXT ONLY — TestFlight renders What to Test verbatim, so markdown
  (`##`, `**`, backticks) shows up as literal symbols in the app (user
  report, 2026-08-17). Headers are bare lines ending with a colon;
  bullets are "•"; emphasis is word choice, not markup.
- App Store Connect rejects some glyphs — ✛ is known-refused, `←↑↓→` passes.

353 fixes a rendering regression 352 shipped. Skip 352, take this.
334 is the build in App Review — TestFlight only.

---

353 fixes a regression that 352 introduced — if you're on 352, update
before anything else.


THE SHREDDED-SCREEN REGRESSION IS FIXED:

On 352, coming back into a live session could paint the terminal as
narrow vertical shards of text down the left edge — one or two
characters per line — and it stayed that way until a full disconnect
and reconnect. 352's re-entry fix reused the terminal but let it get
measured at zero size for one frame; the buffer reflowed to a
one-column grid and never recovered. The reuse now keeps the
terminal's real size through the swap.

Please try: connect (mosh or SSH), let the screen fill with real
content, go Home, come back — repeatedly. Worth reporting: any shredded
or half-width text after re-entry, even once, and whether the session
was mosh or SSH, tmux or herdr.


EVERYTHING 352 SHIPPED STILL APPLIES:

• herdr version-conflict banner with the one-tap "Restart herdr server"
  button (and the crash that scenario used to cause is gone).
• Re-entering a session shows the screen you left, not a lone cursor.
• mosh recovers from brief network failures by itself — try airplane
  mode on and off mid-session.


KNOWN ISSUES — PLEASE DON'T RE-REPORT THESE:

• On phones narrower than a Max, the last key in the default shortcut
  row sits half-visible at the right edge. Deliberate — the row scrolls.
• A model download pauses when you switch apps (resumes on return).
  Deliberate.
• The home header can read "1 live connection" while the card below
  says OFFLINE. Cosmetic.
• The lock screen stops updating a couple of minutes after backgrounding
  and shows a paused hint. iOS suspends the app; no push channel yet.


REPORTING:

Screenshot through TestFlight, or share it straight to your agent.
Please include: device model, iOS version, SSH or mosh, tmux or herdr.
