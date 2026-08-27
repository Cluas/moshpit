---
title: "Agent notifications, end to end"
description: "An agent on your server stops and asks for permission. This page traces the whole path: how that state reaches your phone — even locked, even with the app closed — what gets encrypted where, and the rules that keep it from nagging you."
---

## The agent reports

On **herdr**, `agent_status` is a field on every pane — nothing to install. On **tmux**, hooks you install stamp four `@moshpit_*` options onto the pane the agent runs in. A real question (a permission prompt, mid-turn) and an idle reminder on an agent you deliberately parked are different things, and the hooks tell them apart — a parked agent never lights anything up.

![The Dynamic Island collapsed to a single teal dot next to a running 0:10 timer, showing an agent still working](/island-working.jpg)

## Moshpit reads it

While Moshpit is running it sweeps every tracked session every `2s`. On herdr the control poll is 2s while things move, easing to 8s once the tree goes quiet.

![The Dynamic Island collapsed to an amber dot next to an exclamation mark, showing an agent that needs you](/island-blocked.jpg)

## When the app is closed, your host pushes

The part a suspended iPhone can't do for itself, your dev host does: it seals the alert with a key **only your device holds** and hands the ciphertext to Moshpit's push relay, which passes it to Apple. Neither the relay nor Apple can read a byte of it — agent names, commands, everything decrypts inside a notification extension on your phone, even on the lock screen. Pairing happens automatically the first time you enable notifications on a host; the relay is part of Moshpit, with nothing to configure.

The notification's job is to get you to the pane: tap it and you land in the exact pane that asked. Reading the question before answering it is the point of a terminal in your pocket — there are no blind Allow/Deny buttons to press from the lock screen, on purpose.

![A locked iPhone showing one time-sensitive Moshpit notification: claude +2 — Edit src/retry.ts? — m1-pro · pit](/22-push-summary.jpg)

## Quiet by design

Notifying on everything is the same as notifying on nothing, so four rules stand between an agent and your attention:

- A question must **stand for 30 seconds** before any phone hears about it. Answered at your desk means never announced.
- All waiting agents on a host share **one summary card** ("claude +2"). Only the moment *nobody was waiting → someone is* rings and may break through Focus; everything after updates the card silently.
- A finished turn only chimes if it ran **three minutes or more**. Short turns file into the list without lighting the screen.
- **Parked agents stay silent.** An agent idling at its prompt because you left it there is not asking you anything.
