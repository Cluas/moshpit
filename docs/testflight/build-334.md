# TestFlight · 1.0.0 (334)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

334 is 332 plus the SSH-key import fix. Everything 332 asked for still needs
checking, so it is repeated here rather than assumed done — this is the build
that went to App Review.

---

## New in this build: importing an SSH key file works

**Keys ▸ add a key ▸ Import ▸ "Import File…"** used to do nothing at all —
you picked a file and got no key text and no error. The file was being read
correctly and then thrown away, because the picker cleared its own record of
*which* field the contents belonged in before handing them over.

Please try all three:

1. **Private key.** Import ▸ "Import File…" ▸ pick a private key. Its contents
   should land in the private-key box above the button.
2. **Public key.** The small folder button beside "Public key line (optional)".
   Same fault, same fix — both buttons share one picker.
3. **Both, public first.** Import a public key, then a private one. This is the
   order most likely to expose leftover state: if anything lands in the wrong
   box, that is the bug coming back.

Files from iCloud Drive and from other apps' folders are the interesting cases;
those are the ones needing permission to read.

## Two things you reported earlier — please re-check

**Switching agents no longer shows you the previous one.** Tapping an agent used
to leave the agent you came FROM on screen for about a second. You should now
get a brief clean fade, then the new agent — and **never a readable word of the
previous agent's output**. Worth reporting: old text legible after the tap, a
dark rectangle that lingers, or anything noticeably worse on a slow link than a
fast one.

**⇥⏎ should now actually send.** With Claude Code offering a prompt completion,
the chip accepted the suggestion but never submitted it: both keys went out in
a single write, and a TUI receiving text and a Return together commonly reads
the lot as a *paste*, so the Return became a newline in the box. They now go out
as two presses 120ms apart.

**This one needs your eyes most.** The timing is an estimate we cannot verify
from here. If ⇥⏎ still only accepts without sending, sends twice, or feels
laggy — that is exactly the report we need. The chip starts outside the bar; add
it from the shortcut editor first.

## Your app icon now shows up inside the app

Picking an alternate icon (Settings ▸ Appearance ▸ App Icon) used to change the
Home Screen and nothing else. The home header, the "no connections yet" screen
and the connecting loader now follow your choice. The default icon keeps its own
loader animation (the surfer bobs, the crowd pulses); the alternates breathe
instead, since none of them has a surfer or a crowd. Look for artwork that reads
wrong at those sizes — a harsh edge, a glow in the wrong colour.

## The multiplexer picker has icons

Connection form ▸ ADVANCED ▸ Multiplexer showed two lowercase words. Each choice
now carries a symbol too: a plain shell, a split grid for tmux, and the sparkle
the workbench puts on agent rows for herdr.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (and resumes when you come
   back). Deliberate: background transfers wedged it permanently after a kill.
2. **The lock-screen Live Activity padding** has never been verified on a real
   device. Report it if it looks cramped or absurdly empty.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Known, cosmetic.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, and tmux or herdr — those four save about half
the back-and-forth.

For the key import specifically, say **where the file came from** (on-device,
iCloud Drive, another app) — that is what decides which permission path runs.
