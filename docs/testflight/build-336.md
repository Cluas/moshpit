# TestFlight · 1.0.0 (336)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

336 is 334 plus one fix. **334 is the build in App Review** — this one is
TestFlight only, so anything found here is still in time for 1.0.1 rather than
for the release.

---

## New in this build: scrolling while an agent is answering

Swiping up while codex was streaming an answer made the screen judder — it
scrolled, snapped back to the bottom, scrolled, snapped back. Scrolling the
same pane while codex sat idle was fine.

Both halves were the clue. An agent that grabs the mouse gets your swipe
forwarded to it as a scroll wheel, and it scrolls its own transcript — which is
why idle worked. But mid-answer it is also repainting continuously, and every
frame pins the view back to the bottom. The two were fighting over the same
viewport.

Now a pane that has printed something in the last half second scrolls through
**tmux copy-mode** instead, which parks your view on the server where the
agent's repainting cannot reach it.

Please try, in a tmux pane:

1. **Ask an agent something long, and swipe up while it is still writing.**
   The view should move and STAY where you put it. No judder, no snapping to
   the bottom. New output keeps arriving underneath you.
2. **Keep scrolling after it finishes.** You should stay where you are, not get
   yanked to the live end mid-read.
3. **Type something.** That returns you to the live bottom, as before.
4. **Scroll an idle agent** (at its prompt, not writing). It should still scroll
   its own transcript the way it always did — that path was deliberately left
   alone, so if paging inside Claude Code got worse, that is a regression and
   we want it.

Worth reporting: judder that survives this build, a scroll position that drifts
on its own, or typing that fails to bring you back to the bottom.

**herdr is NOT fixed** — only tmux. herdr decides wheel-vs-scrollback on the
server and gives the app no way to insist, so the same judder may still be
there. Please say which one you were using.

## Still worth checking (unchanged from 334)

- **SSH key file import.** Keys ▸ add ▸ Import ▸ "Import File…" for the private
  key, and the folder button beside "Public key line". Try public-then-private
  in that order — that ordering is what would expose leftover state.
- **⇥⏎.** With Claude Code offering a prompt completion, the chip should accept
  AND send. The 120ms gap between the two keys is an estimate we cannot verify
  from here, so this needs your eyes most: report if it only accepts, sends
  twice, or feels laggy.
- **Switching agents** should fade cleanly, never showing a readable word of the
  agent you came from.
- **Your app icon** now appears in the home header, the empty state and the
  connecting loader.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (and resumes when you come
   back). Deliberate.
2. **The lock-screen Live Activity padding** has never been verified on a real
   device.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, and **tmux or herdr** — that last one decides
which code path you were on, and this build only changed one of them.
