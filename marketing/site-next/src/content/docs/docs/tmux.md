---
title: "Using tmux"
description: "Moshpit attaches as a tmux control client and draws your sessions, windows and panes as native iOS views. Here is what that does, what it changes on your server, and where it stops."
---

You need tmux on the host, on `PATH` — or a path in the connection form's **Custom tmux Path** field. Nothing else. The [hook install](#agents) below is optional and separate.

If tmux is missing the session degrades instead of failing: a plain SSH single pane plus a banner reading <b>"tmux not found on this host — plain SSH session."</b>, and an **Install tmux** action that shows the command for your package manager and runs it only when you tap it. Moshpit never silently swaps in a different multiplexer — tmux and herdr hold unrelated sessions. A custom tmux path skips the check entirely, since the probe only walks `PATH`.

## Switching sessions and windows

The breadcrumb above the terminal is live navigation, not a label. Tap the session name to see every session on the server and switch; tap the window segment to see that session's windows.

![The session switcher sheet: every tmux session on the server, each showing its window count and whether it is attached elsewhere](/06-tmux-sessions.jpg)

![The window switcher sheet for one session: each window's pane count, current command, and which one needs you](/08-tmux-windows.jpg)

## One line into your login shell

Moshpit doesn't shrink tmux's own text UI onto a phone. It attaches as a control client — `tmux -CC`, the same control mode iTerm2 uses — and renders the result itself. On connect it writes exactly this:

```sh
tmux set -g history-limit 50000 2>/dev/null; tmux -CC attach
```

`attach` with no `-t` means your most recent existing session. <b>Moshpit never runs `tmux new` on its own.</b> With no sessions on the server you get an empty state saying so, and a **Create Session** button you press yourself.

Discovery is three commands on the same channel, re-run whenever the tree changes. `-a` is every session on the server — that's why the home screen can show several expanded at once:

```
list-sessions -F '#{session_id} #{session_attached} #{session_name}'
list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} …'
list-panes   -a -F '#{pane_id} #{window_id} #{pane_index} … #{pane_current_command}'
```

- <b>Attach has a 22-second deadline.</b> If tmux never confirms, the home card reads *stalled* — "Attach didn't complete — tmux never confirmed." — with a retry. SSH itself is still alive; it's the tmux handshake that didn't land.
- <b>Control mode cannot run over mosh.</b> Mosh sends screen diffs, not a line-framed protocol. On mosh + tmux, Moshpit renders tmux's ordinary full-screen UI over mosh and opens a **second, sidecar SSH connection** running `-CC` just to feed the breadcrumb and the sheets. No SSH, no native sheets — you get a bare `tmux attach`.
