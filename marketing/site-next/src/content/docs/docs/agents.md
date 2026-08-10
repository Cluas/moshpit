---
title: "Answering agents from the lock screen"
description: "An agent on your server stops and asks for permission. This page traces the whole path: how that state gets to your phone, exactly what each button types back, which settings change it, and the point where iOS stops all of it."
---

## The agent reports

On **herdr**, `agent_status` is a field on every pane — nothing to install. On **tmux**, hooks you install stamp four `@moshpit_*` options onto the pane the agent runs in.

## Moshpit reads it

While Moshpit is running it sweeps every tracked session every `2s`. On herdr the control poll is 2s while things move, easing to 8s once the tree goes quiet.

## Your tap types back

The lock-screen buttons run inside Moshpit, not in the widget. Each sends raw bytes into that pane — Enter, Esc, Ctrl-C, or your text. Nothing else.

:::note
<b>There is no Moshpit server in this picture.</b> Every alert on your phone is a local notification the running app generated from the live session. That is the trade for having nothing in the middle, and it has a cost you should know before you rely on it.

Read [where this stops working](#limits) before you decide to walk away from your desk.
:::
