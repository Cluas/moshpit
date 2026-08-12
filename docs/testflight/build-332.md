# TestFlight · 1.0.0 (332)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

---

## Two things you reported are fixed — please re-check them first

**Switching agents no longer shows you the previous one.** Tapping an agent in
the workbench used to leave the agent you came FROM on screen for about a
second before the new one appeared. Two causes: the renderer waited for two
round trips to the server to rediscover a pane we had already chosen, and
nothing covered the old screen meanwhile.

What you should see now: tap an agent, get a brief clean fade, then that
agent's screen. Specifically **you should never read a word of the previous
agent's output**. Worth telling us:

- Any moment where the old agent's text is legible after the tap.
- A dark rectangle that lingers — the cover is supposed to lift the instant the
  new screen arrives, and give up on its own after 1.6s regardless.
- Anything worse on a slow link than on a fast one. This changed the timing of
  the whole switch, so bad networks are the interesting case.

**⇥⏎ should now actually send.** With Claude Code offering a prompt completion,
the chip accepted the suggestion but never submitted it. The two keys were
going out in a single write, and a TUI receiving text and a Return together
commonly reads the whole thing as a *paste* — so the Return landed as a newline
in the box instead of submitting. They now go out as two presses 120ms apart.

**This one needs your eyes more than the others.** We fixed the half we control
and the timing is an estimate — if ⇥⏎ still only accepts without sending, or
now sends twice, or feels laggy, that is exactly the report we need. The chip
lives in the shortcut editor's AVAILABLE list; add it to the bar first.

## Your app icon now shows up inside the app

Picking an alternate icon (Settings ▸ Appearance ▸ App Icon — Ringdown, :wq,
Prefix, Localhost, No Carrier, Cursor, Hail) used to change the Home Screen and
nothing else, so the app kept showing the mark you had just replaced. The home
header, the "no connections yet" screen and the connecting loader now follow
your choice.

- The default icon keeps its own animation on the connecting screen (the surfer
  bobs, the crowd pulses). The alternates breathe instead — they have no surfer
  or crowd to animate.
- Look for artwork that reads wrong at these sizes: a visible border, a harsh
  edge, a glow in the wrong colour. The reused artwork is 180×180 and the
  largest placement is 112pt, so blur would be a surprise — report it if so.

## The multiplexer picker has icons

The connection form's ADVANCED ▸ Multiplexer row showed two lowercase words.
Each choice now carries a symbol as well: a plain shell, a split grid for tmux,
and the same sparkle the workbench puts on every agent row for herdr. The
selected row in the menu still shows a checkmark.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (and resumes by itself when
   you come back). Deliberate trade: background transfers wedged the download
   permanently after the app was killed.
2. **The lock-screen Live Activity padding** has still never been verified on a
   real device. Report it if it looks cramped or absurdly empty.
3. The home header can read **"1 live connection"** while the card below it says
   OFFLINE. Known, cosmetic.
4. The App Icon gallery's own thumbnails are unchanged — only the in-app
   placements started following your pick.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, and tmux or herdr — those four save about half
the back-and-forth.

For the agent-switch timing specifically, the useful detail is **how far away
the server is**: switching between two agents on a box across an ocean is the
case this build changed most.
