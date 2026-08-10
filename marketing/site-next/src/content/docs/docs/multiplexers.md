---
title: "tmux or herdr — pick per connection."
description: "A multiplexer keeps your work alive when the connection drops, and gives Moshpit something better to render than one long scroll. Moshpit speaks two. They are not interchangeable and it will never quietly swap one for the other, because their sessions are completely unrelated — switching would show you someone else's work."
---

## The short answer

- <b>Already living in tmux?</b> Pick tmux. Moshpit attaches to the sessions you already have and never restyles them.
- <b>Mostly running coding agents?</b> Pick [herdr](/docs/herdr). Agent status works with nothing installed, panes render full width, and you can start an isolated task from the phone.
- <b>Neither installed and not sure?</b> Pick None. Moshpit is a perfectly good plain SSH client; add a multiplexer when you miss one.

## Side by side

|  | tmux | herdr |
| --- | --- | --- |
| Where it is | On nearly every server already | Install it yourself (one binary) |
| Agent status | Hook install, then accurate | Protocol field — nothing to install |
| What it calls things | session · window · pane | workspace · tab · pane |
| Rendering on the phone | Full layout, panes and splits | One pane, full width |
| Your laptop's window size | Shared — the smallest client wins | Untouched — your phone resizes only its own pane |
| Isolated task from the phone | — | git worktree, workspace and agent in one sheet |
| Maturity | Twenty years of it | New, and moving fast |

## Where each one is genuinely worse

**tmux** shares one window size across every attached client, so a phone joining your session can squeeze the layout on your laptop. Moshpit pins the window to work around it, but the constraint is real. Agent status also needs the hook installed on the host, and without it Moshpit falls back to reading output, which is a guess.

![A tmux session rendered natively: the window bar across the top and a full-width terminal pane below it](/07-tmux-terminal.jpg)

**herdr** is young. Its direct attach is exclusive per pane, so two clients on the same pane fight over it; the app backs off and tells you rather than flickering silently. Its tree is polled, so somebody else's change on the laptop can take a few seconds to show up. And on 0.7.3 it reports no foreground command, so the pane crumb falls back to `pane N`.

## Changing your mind

The multiplexer lives on the connection, in **Edit → Advanced**. Change it and reconnect. Nothing is lost — the other multiplexer's sessions are still on the host, exactly as you left them, because Moshpit never killed anything to make the switch.

## Over Mosh, both fall back

Mosh transmits rendered screen differences, which cannot carry tmux's control-mode protocol or herdr's frame protocol. Over Mosh you get the multiplexer's own text UI inside the terminal instead of native rendering — still useful, still roaming, just not the native lists. See [Mosh and roaming](/docs/mosh).
