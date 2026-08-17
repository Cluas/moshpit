# TestFlight · 1.0.0 (355)

Paste the section below into App Store Connect ▸ TestFlight ▸ this build ▸
**What to Test**. It is written for the tester, not for us: what to try, what
changed under them, and what NOT to bother reporting.

RULES for this section (learned the hard way):
- Under 4000 characters (TestFlight's limit). PLAIN TEXT ONLY — TestFlight
  renders What to Test verbatim (headers end with a colon, bullets are "•",
  emphasis is word choice). ✛ is refused by ASC; `←↑↓→` passes.

Canary-only until it survives device testing; then release-promote.sh.

---

355 fixes both herdr problems from the 354 canary run.


HERDR OVER MOSH ACTUALLY STARTS:

On mosh connections, herdr's TUI never launched — the session sat at a
bare shell with an empty workspace tree. The launch command was being
swallowed by the reconnect input gate (a 350-era regression). It now
rides the transport directly.

Please try: connect a mosh+herdr host — the herdr TUI should appear
without any typing.


HERDR 0.8 AGENTS SHOW UP AGAIN:

herdr 0.8 moved the agent's name out of the pane objects into a new
top-level list; the app decoded a status with no identity and rendered
nothing. Agents running under a 0.8 server show up in the tree and on
Home again.

Please try: run claude (or any agent) inside a herdr 0.8 workspace and
check the agents tree on Home. Worth reporting: an agent visibly
running in herdr that the app doesn't list within ~8 seconds.


STILL INVESTIGATING — mosh+tmux input dying after a while. If you hit
it, note three things before reconnecting: (1) is the pane still
UPDATING (a running agent's spinner, a ticking clock) or frozen in
place; (2) had the phone been locked/backgrounded since you last
typed, or was the app foreground the whole time; (3) does typing show
ANYTHING on screen (wrong characters count as something).


KNOWN ISSUES — PLEASE DON'T RE-REPORT THESE:

• On phones narrower than a Max, the last key in the default shortcut
  row sits half-visible at the right edge. Deliberate — the row scrolls.
• A model download pauses when you switch apps (resumes on return).
• The home header can read "1 live connection" while the card below
  says OFFLINE. Cosmetic.
• The lock screen stops updating a couple of minutes after
  backgrounding. iOS suspends the app; no push channel yet.


REPORTING:

Screenshot through TestFlight, or share it straight to your agent.
Include: device model, iOS version, SSH or mosh, tmux or herdr.
