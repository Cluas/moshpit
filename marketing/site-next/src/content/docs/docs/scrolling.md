---
title: "Scrolling, and why it is harder than it looks."
description: "A swipe up means two completely different things depending on what is running in the pane. In a shell it means “show me what scrolled past”. In a full-screen program that grabbed the mouse — an agent, vim, htop — it means “scroll your own view”. Send the wrong one and either your history is unreachable or your keystrokes stop working."
---

## What a swipe does

- **In a plain shell** — you move through real scrollback. On tmux that is tmux's own buffer, entered and left cleanly.
- **In a full-screen app** — the swipe becomes a wheel event, forwarded to the app so it scrolls itself. Typing keeps working the whole time.

![Scrolled back in a plain shell pane: several minutes of a webhook server's request log](/32-scrollback.jpg)

## How it decides

On tmux, Moshpit asks tmux whether the pane's program has grabbed the mouse, and routes accordingly. On [herdr](/docs/herdr) it does not have to ask: scrolling is a protocol action and the *server* decides whether it becomes a mouse report or scrollback movement. That is one of the quiet places herdr is simply better.

## Leaving copy-mode

When a swipe put a tmux pane into copy-mode, the next thing you type needs to reach the shell, not the copy-mode overlay. Moshpit exits copy-mode for you on the same channel as the keystrokes, so the two cannot arrive out of order — an ordering bug here shows up as “my first character went missing”.

## Limits worth knowing

- **Without a multiplexer**, scrollback is whatever the app itself holds in memory for that session. Nothing is persisted across a disconnect.
- **On herdr**, history paging is done by the server on request — herdr exposes no API to export a whole scrollback buffer, so there is no local copy to search.
- **Over Mosh**, the protocol transmits the visible screen rather than a stream, so scrollback belongs to the multiplexer running inside, not to Mosh.
