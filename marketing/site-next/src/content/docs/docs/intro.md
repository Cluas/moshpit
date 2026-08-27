---
title: "What Moshpit does."
description: "One paragraph, in case you landed here from a search: Moshpit is a paid iPhone and iPad terminal for people who run AI coding agents on machines they own. It connects over SSH or Mosh, renders tmux and herdr natively, and — the part no other iPhone terminal does — puts an agent's “I need a human” on your lock screen with the buttons that answer it."
---

## The problem it solves

A coding agent runs for a long time and then stops, because it wants permission to do something. If you are not at the desk, it waits. People come back hours later to an agent that finished its thinking in the first ten minutes and spent the rest of the afternoon holding a question.

![Moshpit's home screen: an Agents section listing Claude Code needing you, codex working and an idle claude, above a tree of workspaces](/01-agents.jpg)

Moshpit's whole reason to exist is closing that gap: the question reaches your phone, and you answer it without unlocking into anything.

## What it is

- <b>A real terminal.</b> SSH and Mosh, a proper VT emulator, hardware keyboard support, themes and a shortcut bar you build yourself. Not a chat window with a terminal bolted on.
- <b>A multiplexer client.</b> tmux over control mode, or [herdr](/docs/herdr) — the multiplexer written for coding agents — each rendered natively rather than squeezed into a phone-sized fake desktop.
- <b>An agent console.</b> Which agent is blocked, for how long, and one tap to answer it from the lock screen or the Dynamic Island.

## What it is not

- <b>Not a service.</b> There is no Moshpit account, no relay and no cloud — because there is no Moshpit server at all. Your phone talks to your machines.
- <b>Not a file manager.</b> No SFTP browser, no mounting servers into Files. If that is your day, other apps do it well and this one does not do it.
- <b>Not an agent.</b> Moshpit does not think, summarise or write code. It carries your agents' output to you and your keystrokes back.

## What it needs from you

A machine you can already SSH into. Everything else is optional: `mosh` buys roaming, `tmux` or `herdr` buy session persistence and the agent features. Moshpit tells you what is missing and shows the command — it never installs anything on your host on its own.

## The limit worth knowing on day one

Live agent state only updates while Moshpit is running in the foreground or briefly in the background — once iOS suspends the connection, the Live Activity keeps showing the last thing it saw (and says "paused") until you come back. What crosses that gap is the push path: your own host seals an alert only your phone can decrypt and sends it through Moshpit's relay, so "an agent needs you" still reaches a locked phone with the app closed. [Agent notifications, end to end](/docs/agents) has the whole story.

## Where to go next

[Set up a connection](/docs/setup) if you want to start, [Choosing tmux or herdr](/docs/multiplexers) if you are deciding, or [How Moshpit compares](/compare) if you are still working out whether this is the right app at all.
