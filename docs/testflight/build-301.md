# TestFlight · 1.0.0 (301)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

---

## The main thing: voice input

There's a new **mic** key on the shortcut bar. Speak, and the text lands in a
preview panel first — **Insert** types it, **Send** types it and presses
Return. Until you tap one of those, nothing reaches the terminal.

**Settings → Voice Input → Recognition** offers two engines:

- **Apple (built in)** — nothing to download, but one language per session
- **Whisper (local model)** — needs a model downloaded once, and understands a
  sentence that mixes two languages

Please concentrate on **Whisper with mixed-language speech**: say a sentence in
your own language with English command names in the middle of it, e.g.
"把这个 commit rebase 到 main" or "帮我看下这个 pod 为什么 CrashLoopBackOff".
Set **Settings → Voice Input → Language** explicitly to your language rather
than leaving it on Auto-detect — detection wavers on short or heavily mixed
phrases.

Download a model under **Settings → Voice Input → Model**: Small (~470 MB) is
enough, Large v3 Turbo (~954 MB) is the most accurate but needs a recent chip.
**Please interrupt a download on purpose** — switch to another app, or swipe up
to kill Moshpit entirely. Coming back should show "xxx MB of ≈470 MB — paused"
with a **Resume** button, and continue from there rather than starting over.

## These changed on purpose — not bugs

- **Opening a terminal no longer raises the keyboard.** Tap the terminal, or the
  keyboard key at the right of the shortcut bar. Want the old behaviour?
  **Settings → Behavior → Keyboard on Open**.
- **`^L` is no longer in the default shortcut bar** — the mic took its slot. Add
  it back from the shortcut editor if you use it.
- **The mic is an ordinary chip**: reorder it, or move it out of the toolbar.
- An overflowing shortcut bar now **fades at the edge** instead of cutting a
  chip in half, to show there's more to drag to.

## New shortcuts (outside the bar by default — add them yourself)

- **`⏎`** — Return
- **`⇥⏎`** — accept Claude Code's suggested prompt and send it (Tab then Return
  in one tap)

## Interface language

Chinese and Japanese are now **fully translated** (612 strings, no gaps). If you
find anything **still in English**, or wording that reads badly, please
screenshot it.

The `git status` output in the theme editor's preview, and the Whisper model
names (Tiny/Base/Small/Large v3 Turbo), are **left in English deliberately** —
no need to report those.

## Known issues — please don't re-report these

1. **After a tmux session reconnects**, you may see whatever was in the input
   bar before the drop, and backspace may not clear it. Being investigated. If
   you hit it, please answer one question: pressing backspace **one tap at a
   time (not held down)** — do those characters disappear? That answer decides
   the fix.
2. **A model download pauses when you switch apps** (and resumes by itself when
   you come back). This is a deliberate trade: using background transfers wedged
   the download permanently after the app was killed — no progress, no error.
3. **The lock-screen Live Activity padding** was changed but never verified on a
   real device. Report it if it looks cramped or absurdly empty.
4. **Send's keystroke order is unverified on a real device.** If tapping Send
   inserts the text but does not submit it, please report that immediately.

## Reporting

Screenshot through TestFlight, or send it to me directly. Please include: device
model, iOS version, SSH or mosh, and tmux or herdr — those four save about half
the back-and-forth.
