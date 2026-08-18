# TestFlight · 1.0.0 (372)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

Canary-only until it survives device testing; then release-promote.sh.

---

372 stops mosh+tmux connections from leaving immortal ghost clients
on your server — the hidden cause of the SSH-side resize garble.

WHAT WAS ACCUMULATING ON YOUR SERVER:

• Every mosh+tmux connect attached a phone-sized tmux client on the
  server, and mosh keeps its server process alive across disconnects
  by design — so that client NEVER left. One server was found
  carrying eleven of them, one per connect. tmux sizes each window to
  whichever attached client touched it last, so the ghosts (75 cols),
  your desktop terminal (hundreds of cols) and the live phone fought
  over every shared window: sizes flapped, each flap forced a full
  redraw to a dozen clients, and on a slow link the phone rendered
  that flood as constant garble. herdr connections never touch tmux,
  which is why mosh+herdr felt fine while SSH+tmux suffered.
• 372 ties each renderer's life to its tmux client (detaching it now
  tears down the whole stack, mosh process included) and every
  connect first sweeps away the previous ones, same as the herdr
  renderer already did. Ghosts left by OLDER builds can't be
  attributed safely, so clear them once by hand: tmux list-clients,
  then tmux detach-client -t /dev/ttysNN for each phone-sized one.

TEST:

• mosh+tmux: connect and disconnect several times, then run
  tmux list-clients on the server — you must see only your real
  terminals plus at most ONE phone-sized client per open Moshpit
  session, and no pile-up after repeated reconnects.
• SSH+tmux, with a desktop terminal attached to the same session:
  switch windows, scroll, type — sizes must stay stable, no repeated
  resize flapping.
• Everything 371 shipped still applies — reply pairing across failed
  preferred attaches, scroll styles, mosh white blocks.

KNOWN ISSUES — DON'T RE-REPORT:

• Half-visible last shortcut key on narrower phones (row scrolls).
• Model downloads pause while backgrounded.

REPORTING: screenshot through TestFlight or share straight to your
agent. Include: device, iOS version, SSH or mosh, tmux or herdr.
