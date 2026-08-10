---
title: "Troubleshooting"
description: "Organised by what you see, not by what went wrong underneath. Every entry says what the screen means and what to do next. Several of them end in \"nothing, this is a limit\" — those are the honest ones."
---

Three rules shape almost everything on this page, and knowing them explains most of what you will run into.

## You always get connected

A missing dependency never blocks the connection. It degrades. SSH is the floor transport, a bare shell is the floor experience, and the terminal stays fully usable behind any banner.

![A Connection Error alert reading ](/31-connection-error.jpg)

## Plain language, not stderr

Moshpit says what is missing, what it costs you, and how to install it. A raw `command not found` is a bug, not a message.

## Nothing happens behind your back

No silent installs. No silent session creation. No quietly attaching the other multiplexer because the one you picked isn't there.

:::note
<b>Three surfaces to read before anything else.</b>

<b>The banner at the top of the terminal.</b> One slot, one banner at a time, always dismissible. Precedence is fixed: a dead mosh return path outranks the roaming banner, which outranks the degraded-host banner.

<b>The transport pill in the top bar.</b> It reads `SSH` or `MOSH` when live, `MOSH · roaming` while roaming, `reconnecting` with an amber dot, or `offline` with a red one. A dropped session is visible, not silent.

<b>Long-press that pill on a mosh session.</b> You get **MOSH DIAGNOSTICS** — four raw counters, meant to be screenshotted and sent, not admired.
:::
