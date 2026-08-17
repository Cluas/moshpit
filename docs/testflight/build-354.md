# TestFlight · 1.0.0 (354)

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

354 is the first build through the Canary flow: it reaches the internal
group only, and goes external via scripts/release-promote.sh after
device testing. 334 is the build in App Review — TestFlight only.

---

354 is a small follow-up to 353.


THE HOME PHOTO BUTTON IS GONE:

The per-agent photo button on Home's agent rows is removed — sending an
image starts from the screenshot (share sheet), from Shortcuts, or from
the terminal's image key, not from hunting down the agent first. If you
used that button, the share sheet does the same job with one less hop.

Worth reporting: anywhere the removal left a hole — an image flow you
used that now has no path.


EVERYTHING 353 SHIPPED STILL APPLIES:

• The 352 shredded-screen regression is fixed — leave and re-enter live
  sessions freely.
• Image upload over mosh probes its channel first — background the app,
  come back, send an image immediately; it should just work.
• herdr version conflicts show a banner (terminal) and a card notice
  (Home), both with a one-tap Restart server button.
• mosh recovers from brief network failures on its own.


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
