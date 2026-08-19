---
title: "Scrolling, and why it is harder than it looks."
description: "A swipe up means two completely different things depending on what is running in the pane. In a shell it means “show me what scrolled past”. In a full-screen program that grabbed the mouse — an agent, vim, htop — it means “scroll your own view”. Send the wrong one and either your history is unreachable or your keystrokes stop working."
---

## What a swipe does

- **In a plain shell** — you move through real scrollback, held locally on the phone. At attach, Moshpit pre-loads the pane's recent history from tmux, so there is something to scroll back through even for output printed before you connected.
- **In a full-screen app** — the swipe becomes a wheel event, forwarded to the app so it scrolls itself. Typing keeps working the whole time.

![Scrolled back in a plain shell pane: several minutes of a webhook server's request log](/32-scrollback.jpg)

## How it decides

On tmux, Moshpit asks tmux whether the pane's program has grabbed the mouse, and routes accordingly. On [herdr](/docs/herdr) it does not have to ask: scrolling is a protocol action and the *server* decides whether it becomes a mouse report or scrollback movement. That is one of the quiet places herdr is simply better.

## Reading while output streams

Scrolling up while the pane is still printing would normally be a fight: you scroll to read, the next burst of output yanks the view back to the bottom. Moshpit holds new output while you are scrolled up and replays it the moment you return — scroll back down, or just type, and the view snaps to live with nothing lost.

## Why tmux copy-mode is never involved

The obvious way to scroll a tmux pane is tmux's own copy-mode — and it is exactly wrong for how Moshpit talks to tmux. Moshpit drives tmux over control mode, and tmux draws copy-mode only for regular terminal clients: a control-mode client receives *nothing* while copy-mode scrolls. Worse, entering copy-mode is server-side state — it would hijack a desktop terminal attached to the same window into copy-mode along with you. So on SSH+tmux, a swipe never touches copy-mode; scrollback lives on the phone. (The [mosh+tmux](/docs/mosh) path is different: there the renderer *is* a regular client, and copy-mode is the right tool — see that page.)

## Limits worth knowing

- **Without a multiplexer**, scrollback is whatever the app itself holds in memory for that session. Nothing is persisted across a disconnect.
- **On SSH+tmux**, the local buffer is seeded with a recent slice of the pane's history at attach, not the whole thing — tmux still keeps its full scrollback server-side.
- **On herdr**, history paging is done by the server on request — herdr exposes no API to export a whole scrollback buffer, so there is no local copy to search.
- **Over Mosh**, the protocol transmits the visible screen rather than a stream, so scrollback belongs to the multiplexer running inside, not to Mosh.
