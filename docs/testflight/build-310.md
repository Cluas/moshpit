# TestFlight · 1.0.0 (310)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit).

---

## The main thing: backspace over text you didn't type

Known issue #1 from build 301 is fixed. Holding the delete key used to delete
one character and then go dead — but only over text the *server* put on the
line, never over text you typed yourself, which is what made it look random.

Three ways to reproduce the old bug. All three should now delete continuously
for as long as you hold the key:

1. **Paste** something long with the paste key, then hold delete.
2. In Claude Code, press **up** to recall a previous prompt, then hold delete.
3. **Reconnect** to a session whose input line already has half a command in
   it, then hold delete.

A single tap should still delete exactly **one** character — if a tap ever eats
two, report that immediately, it is the one thing this change could break.

If you'd rather not hold anything: **`^U`** (clear to start of line) and
**`^K`** (clear to end) are now built in. They're not in the bar by default —
add them from the shortcut editor. One keystroke beats a held key over a slow
link, because every backspace is a separate round trip to the server.

## Reconnecting looks like one thing now

Losing the line used to put a modal **Connection Error** card over the terminal
whose only button dropped you out of the session — while the app was already
retrying behind it. The card came and went every twelve seconds as each retry
started and failed.

Now a dropped line is one screen: the pit loader in red, **LINE DROPPED —
RETRYING**, until it's back. The back arrow stays available the whole time.

Please try: turn off Wi-Fi mid-session, wait, turn it back on. You should get
that one screen and then your session, with no dialog at any point.

The error card is **still** there when *you* ask for a connection and it
fails — tap a host, or tap "Connection lost — tap to reconnect", and a wrong
host or password still tells you why. That difference is deliberate.

## Also

- A **non-standard port** now displays correctly. It was being formatted as a
  number, so 2222 showed up as "2,222". Only visible above port 999.

## Known issues — please don't re-report these

1. **After a tmux reconnect you may still see the previous input line's
   contents.** Only the *clearing* of it was fixed, not the reason it comes
   back. It is now clearable — hold delete, or use `^U`.
2. **A model download pauses when you switch apps** (and resumes by itself when
   you come back). Deliberate trade: background transfers wedged the download
   permanently after the app was killed — no progress, no error.
3. **The lock-screen Live Activity padding** has still never been verified on a
   real device. Report it if it looks cramped or absurdly empty.
4. **Send's keystroke order is still unverified on a real device.** If tapping
   **Send** in the voice preview inserts the text but does not submit it,
   please report that immediately.
5. The home header can read **"1 live connection"** while the card below it says
   OFFLINE. Known, cosmetic.

## Still worth testing from build 301

Voice input, if you haven't yet: the **mic** key on the shortcut bar, and
**Settings → Voice Input → Recognition** to pick Apple or a local Whisper
model. Whisper is the one that handles a sentence mixing your language with
English command names. Set **Language** explicitly rather than Auto-detect.

## Reporting

Screenshot through TestFlight, or send it to me directly. Please include: device
model, iOS version, SSH or mosh, and tmux or herdr — those four save about half
the back-and-forth.
