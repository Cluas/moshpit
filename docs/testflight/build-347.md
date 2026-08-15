# TestFlight · 1.0.0 (347)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

347 completes attach-image: every connection type, every source, and a few
sharp edges filed off. **334 is the build in App Review** — TestFlight only.

---

## Attach images on mosh connections

Pure-mosh sessions used to show an apology instead of an upload. Moshpit now
dials a lightweight SSH connection on demand when you attach — no extra login,
no Face ID prompt — and keeps it for the session so the second image is
instant.

Please try, on a mosh connection without tmux: attach a photo. Worth
reporting: a Face ID prompt mid-attach, or an upload that hangs where SSH
connections work fine.

## Failed uploads retry

Lose the network mid-upload and the panel shows what failed with a Retry
button. Retry keeps the images that already landed and re-sends only the
missing ones.

Please try: airplane-mode during a multi-image upload, then Retry after.
Worth reporting: a retry that re-uploads everything, or a Retry that spins
forever after the network is back.

## Photos are smaller now

Screenshots and camera photos that carried an unused transparency channel
used to upload as huge PNGs (one tester photo: 4.6MB). Only images with
actual transparent pixels keep PNG; everything else is JPEG — same pixels,
a fraction of the bytes.

## The camera is a source

Long-press the photo chip → Take photo. Photograph a whiteboard, a sketch,
another screen; it runs through the same pipeline (scaled, location data
stripped on the phone, uploaded only to your server).

Please try it on cellular. Worth reporting: a sideways photo, or the camera
sheet coming up with misplaced controls.

## Uploads expire on the server

Settings ▸ Attached Images: 24 hours / 7 days (default) / keep forever.
Each connect quietly deletes uploads older than your choice — they're
working files for an agent, not a backup.

## The inserted path speaks your agent's dialect

Insert now looks at what's running in the pane: Gemini CLI and Qwen Code get
`@path` (the only form they read deterministically), aider gets `/add path`,
everyone else gets the plain path Claude Code and Codex already understand.

Please try, if you run Gemini/Qwen or aider: attach an image and check the
inserted text matches what your agent expects. Worth reporting: the wrong
format for what's visibly running in the pane. (Known limit: Gemini/Qwen
also require the file inside their workspace folder — uploading into the
project directory is planned, not in this build.)

## Known issues — please don't re-report these

1. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
2. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
3. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. iOS suspends the app; no push channel yet.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for upload reports,
Wi-Fi or cellular and roughly how large the originals were.
