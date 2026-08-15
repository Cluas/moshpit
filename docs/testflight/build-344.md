# TestFlight · 1.0.0 (344)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

344 is 343 plus attach-image and two keyboard-behaviour fixes. **334 is the
build in App Review** — this one is TestFlight only.

---

## New in this build: send a picture to your agent

The bar has a new photo chip. Tap it, pick up to ten images (screenshots,
photos of a whiteboard, a design), and Moshpit uploads them over the
connection you already have — no extra login, no Face ID — into
`~/.moshpit/uploads/` on the server, then pastes the file paths at your
prompt. Claude Code and friends read images from paths, so this is the whole
trick: describe the bug, paste the screenshot's path, let the agent look.

Insert leaves the paths on the prompt so you can type around them; Send
presses Return too. Images are shrunk to 2048px on the long side and their
EXIF/GPS is stripped before anything leaves the phone — the photo library's
location data has no business on a server.

Please try: attach a screenshot to a Claude Code prompt and ask about it.
Multi-select. A huge panorama. A photo taken with the camera (portrait — it
should arrive upright). On tmux, switch panes first: paths go to the pane
you're looking at. Worth reporting: an upload that hangs without failing, a
path pasted somewhere other than your active pane, a sideways image, or a
picker that comes back empty.

Two honest limits: pure-mosh connections show a notice instead of uploading
(mosh has no file channel; the fix is planned), and uploaded files currently
stay on the server until you delete them — automatic cleanup comes later.
Camera and paste-an-image-from-clipboard also come later; don't report their
absence.

## Two keyboard fixes on paths you use daily

**1. A reconnect keeps the keyboard down.** With the keyboard up, a dropped
line used to slide it away mid-reconnect and then pop it back over the
connecting screen — three layout moves for one event. Now the keyboard
collapses once when the reconnect starts, the whole cycle plays out in the
collapsed layout, and it stays down until you tap the terminal.

Please try: keyboard up, airplane-mode for a few seconds, watch the reconnect.
Worth reporting: the keyboard popping back up on its own after the session
returns.

**2. Tapping a link no longer summons the keyboard.** With the keyboard
dismissed, tapping a URL in agent output opened it AND raised the keyboard
over the very text you were reading — and it was still up when you came back
from Safari. A link tap now just opens the link. Tapping anywhere that isn't
a link still raises the keyboard, as before.

Please try: have Claude Code print a dev-server URL, put the keyboard away,
tap the link, come back. Worth reporting: a link that needs two taps, or the
keyboard appearing anywhere you didn't tap to type.

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
2. **The lock-screen Live Activity padding** has never been verified on a real
   device.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
4. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. Known limitation — iOS suspends the app and there
   is no push channel yet; it is honest rather than frozen.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for attach-image
reports, roughly how large the originals were and whether you were on Wi-Fi
or cellular.
