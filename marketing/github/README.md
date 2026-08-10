<p align="center">
  <img src="assets/icon-192.png" width="96" alt="Moshpit icon">
</p>
<h1 align="center">Moshpit</h1>
<p align="center"><b>Always on the line.</b><br>
The iOS terminal built for agent work — SSH · Mosh · tmux · <a href="https://github.com/herdrdev/herdr">herdr</a>.</p>

<p align="center">
  <a href="https://moshpit.cluas.eu.org">Website</a> ·
  <a href="https://moshpit.cluas.eu.org/privacy.html">Privacy</a> ·
  <a href="../../issues">Issues</a> ·
  App Store — <i>coming soon, $4.99 launch price (one-time)</i>
</p>

---

Your Claude Code and Codex sessions keep running on your servers. Moshpit is
how they reach you when you're not at your desk:

- **Answer from the lock screen.** A blocked agent pings you with
  Allow · Deny · Reply actions — a permission prompt costs a glance, not a desk.
- **Who's waiting on you.** An Agents section above the session tree: who needs
  you, who's working, for how long, who's idle and ready for work. The Dynamic
  Island tracks the most urgent one.
- **Start work from your phone.** Pick a repo, name a branch, hand over the
  first prompt — Moshpit creates an isolated git worktree on the host and
  starts the agent in it.
- **A real terminal first.** Mosh sessions that survive Wi-Fi/5G handoffs and
  hotel networks; native tmux control mode (no tiny fake desktop); herdr
  rendered one full-width pane at a time.

<p align="center">
  <img src="assets/40-agents-home.png" width="260" alt="Agents section: Claude Code needs you">
  <img src="assets/43-agent-breadcrumb.png" width="260" alt="Agent breadcrumb in the terminal">
</p>

## Honest by design

- **One-time purchase.** No subscription, no accounts, no feature tiers.
- **Nothing collected.** No analytics, no tracking. Keys live in the Secure
  Enclave; traffic goes only to your servers. App Store privacy label:
  *Data Not Collected*.
- **Never behind your back.** Moshpit never installs anything on your host
  silently, never creates sessions you didn't ask for, and says so when it
  degrades instead of quietly switching transports.

## Does it need anything on my server?

No — it's a standard SSH client out of the box. `mosh`, `tmux` and `herdr`
are each optional; if one is missing, Moshpit shows the install command and
runs it only when you say so. Agent status is zero-setup on herdr; on tmux
it's a one-line hook install offered inside the app.

## This repo

Moshpit itself is **not open source**. This repository is the public home for:

- 🐛 **[Issues](../../issues)** — bug reports and feature requests
- 💬 **[Discussions](../../discussions)** — questions, setups, workflows
- 📸 Screenshots and release notes

## FAQ

**Why "Moshpit"?** Off-hook is the telephone state where the handset is up and
the line is live — which is also the logo. And your agents taking the work
means they take you *off the hook*. 中文名：「摘机」。

**Which agents does it understand?** Anything that runs in a terminal.
Status detection is protocol-level on herdr (Claude Code, Codex, and friends),
hook-based on tmux.

**iPhone only?** iPhone first. (iPad: watch the issues.)
