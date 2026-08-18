# TestFlight · 1.0.0 (371)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

371 closes the last known reply-pairing hole and stops the mosh+tmux
attach from typing into your pane.

WHAT 370 STILL GOT WRONG:

• 369 handled tmux's handshake reply by reserving one slot for it —
  but one connection can carry SEVERAL handshakes: reconnecting to a
  remembered session tries that session first, and a FAILED attempt
  produces a full reply block plus a second handshake from the
  fallback. Every extra block shifted reply pairing by one for the
  rest of the connection: black screens with a stray "2 51", wrong
  frames, and — the visible smoking gun — the app re-typing
  "tmux attach -t '0'" INTO a live pane, because the shifted reply
  made it believe its first attach never landed.
• 371 moves the handshake bookkeeping into the protocol parser, which
  sees session boundaries and swallows each session's handshake reply
  no matter how many there are. Verified against real tmux 3.6a byte
  captures of the failing chain.

TEST:

• mosh+tmux: connect, disconnect, reconnect several times (including
  once right after force-quitting the app). The attach line must
  never appear as typed text inside a pane, and no pane may show a
  lone pair of numbers or protocol text.
• SSH+tmux: reconnect to a server where your remembered session no
  longer exists (kill it server-side first). The app must land on
  another session with a clean screen.
• Everything 370 shipped still applies — scroll styles stay solid
  under SSH+tmux, mosh white blocks stay dead.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
