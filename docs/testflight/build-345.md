# TestFlight · 1.0.0 (345)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

345 is 344 plus three follow-ups to attach-image and one reconnect polish.
**334 is the build in App Review** — this one is TestFlight only.

---

## Copy a screenshot, tap paste

The paste chip now understands images: when your clipboard carries a picture
and no text, tapping paste uploads it and inserts the path — same flow as the
photo chip, one step shorter. Copying text still pastes text; text wins when
the clipboard has both.

Please try: take a screenshot, tap Copy in its preview, then tap the paste
chip in Moshpit. iOS will ask "allow paste?" the first time — that prompt is
the system's, not ours. Worth reporting: a copied image that lands as garbled
text instead of the upload panel, or text paste behaving differently than
before.

## Every sent picture keeps its number

Uploads are numbered #1, #2, … for the session (the badge shows on each
thumbnail). Long-press the photo chip to re-insert any earlier picture's path
— no re-upload, the file is already on the server. The long-press menu also
offers "paste image from clipboard" when there is one.

Please try: send two images, long-press the photo chip, re-insert #1. Worth
reporting: numbers repeating or jumping, or a re-inserted path pointing at
the wrong image.

## Reconnects lost their black flash

A drop-and-reconnect used to flash black frames between the keyboard
collapsing and the connecting screen, and again just before the session came
back. Both gaps are gone — the connecting cover now cuts in over the live
terminal and fades out only after the fresh session has painted.

Please try: airplane-mode a few seconds mid-session (tmux especially) and
watch the whole cycle. Worth reporting: any black blink, or the connecting
screen lingering more than ~a second after the prompt is visibly back.

## For the curious: Remote Clipboard Read

Settings ▸ Behavior has a new toggle, off by default. On, remote programs
that ask for your clipboard via the OSC 52 escape get your clipboard TEXT —
useful for vim/tmux integrations. Off, they get an empty answer. Leave it
off unless you know you want it; iOS will additionally show its own paste
prompt each time something asks.

## Known issues — please don't re-report these

1. **Tapping a notification to enter the app can crash it.** A tester
   reported this with steps and we're on it — thank you. Fix is planned for
   the next build; no need to send more reports unless yours crashes from
   somewhere OTHER than a notification tap.
2. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
4. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. Known limitation — iOS suspends the app and there
   is no push channel yet; it is honest rather than frozen.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for clipboard-paste
reports, what app you copied the image from.
