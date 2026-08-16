# TestFlight · 1.0.0 (349)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

Keep it under 4000 characters (TestFlight's limit). App Store Connect rejects
some glyphs in this field — ✛ is known-refused, `←↑↓→` passes.

349 adds three new ways to hand an agent a picture — none of them require
opening the terminal. **334 is the build in App Review** — TestFlight only.

---

## Share to Moshpit, from anywhere

Moshpit is now in the system share sheet. Take a screenshot, tap its
preview, Share → Moshpit → pick the agent. Works from Photos, Safari,
anywhere images share. The picture uploads and the path lands in that
agent's pane the moment the session is live — instantly if Moshpit is
connected, or queued (up to 48h) until you next connect.

Please try: screenshot → share → Moshpit while your session is live, then
again with Moshpit fully closed (open it after — the queued image should
deliver on connect). Worth reporting: an agent missing from the list that
Moshpit clearly shows on Home, images that never arrive, or arriving in
the wrong pane.

## Send from the Home card

Each agent row on Home has a photo button now — pick images, watch the
upload, Insert or Send. The path goes to that exact pane, phrased for
that agent, without ever opening the terminal.

Please try: two agents running, send a picture to the one that is NOT the
active pane. Worth reporting: the path landing in the wrong pane, or the
Insert/Send buttons acting on the pane you're looking at instead of the
row you tapped.

## Shortcuts: "Send Image to Agent"

A new Shortcuts action takes images and an agent, and queues them like
the share sheet does. "When I take a screenshot, send it to claude" is
now one automation away.

Please try, if you use Shortcuts: build that automation and screenshot
something. Worth reporting: the agent list coming up empty while agents
are visibly running, or the action failing silently.

## Known issues — please don't re-report these

1. The share sheet's agent list mirrors what Moshpit last saw — after a
   long time away it can list agents that have since exited. Queued images
   for a gone pane are dropped after 48h rather than delivered blind.
2. **A model download pauses when you switch apps** (resumes on return).
   Deliberate.
3. The home header can read **"1 live connection"** while the card below says
   OFFLINE. Cosmetic.
4. **The lock screen stops updating a couple of minutes after backgrounding**
   and shows a paused hint. iOS suspends the app; no push channel yet.

## Reporting

Screenshot through TestFlight, or send it directly. Please include: device
model, iOS version, SSH or mosh, **tmux or herdr**, and for share-sheet
reports, whether Moshpit was foreground, backgrounded, or closed when you
shared.
