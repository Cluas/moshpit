# TestFlight · 1.0.1 (392)

RULES: under 4000 chars; PLAIN TEXT ONLY (colon headers, "•" bullets,
no ##/**/backticks). ✛ refused by ASC; ←↑↓→ ok.

All groups. The push release: your agents can now reach you with the
app closed and the phone locked — and everything about notifications
was redesigned to interrupt you less, not more.

---

PUSH, SEALED END-TO-END:

• When the app is not running, your dev host sends the alert through
  Moshpit's push relay. It is encrypted on your host with a key only
  your device holds — the relay and Apple carry ciphertext and can
  read none of it. Decrypts on the lock screen.
• Setup is automatic. Connect to a host and Moshpit installs, pairs
  and repairs everything itself; the only question you are ever asked
  is the first hook install on each host, once. The old Install/Pair
  button ceremony is gone.
• Several devices per host now work: iPhone and iPad both receive.
  (Before, pairing a second device silently killed the first.)

QUIET BY DESIGN:

• A question must stand for 30 seconds before any phone hears about
  it. Answered at your desk means never announced.
• All waiting agents on a host share ONE summary card ("claude +2").
  Only the moment nobody-was-waiting becomes someone-is rings — and
  may break through Focus. Everything after updates silently.
• A finished turn only chimes if it ran three minutes or more.
• Agents you parked at their prompt stay silent everywhere: no card,
  no NEEDS YOU on the island. Day-old frozen NEEDS YOU badges heal
  themselves.
• Looking at a prompt acknowledges it — the card comes down.
• Reconnects and app relaunches no longer re-announce prompts you
  already saw.

FASTER TMUX ATTACH:

• Connecting to a busy tmux session moves ~84% fewer bytes and paints
  sooner. Only the window on screen is filled at attach; the rest fill
  as you switch to them. Deep scrollback loads 400 lines instantly.

ALSO:

• Lock-screen Live Activity no longer clips at top and bottom (it had
  shipped over-height from day one — now measured and bounded).
• Live Activity buttons are gone on purpose: answering an agent you
  have not read is the one thing a terminal app should not make easy.
  Tap the notification; it lands in the exact pane that asked.
• Settings → Recent Log loading state no longer looks broken.
• The transport pill stops flashing the word "reconnecting"; the dot
  and border animate instead. Layout no longer jumps.
• "Set up this host" in Settings explains itself when offline and
  lists your paired hosts.

WHAT TO TEST:

• Pair a host (just connect — accept the one prompt), close the app,
  lock the phone, have an agent stop for permission. The notification
  should arrive within a minute, readable on the lock screen.
• Answer a prompt at your desk within 30s: the phone should stay
  silent.
• Leave an agent parked at its prompt: nothing should ring, ever.
• Long build finishing (3min+): one chime. Short turns: silent.

KNOWN ISSUES — DON'T RE-REPORT:

• Two clients on one herdr pane fight over it.
• Half-visible last shortcut key on narrower phones.

REPORTING: screenshot or share straight to your agent. Include: device,
iOS version, SSH or mosh, tmux or herdr.
