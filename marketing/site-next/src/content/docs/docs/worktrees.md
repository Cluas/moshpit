---
title: "Start a task without touching your working tree."
description: "Handing an agent a job usually means being at the machine: make a branch, open a terminal in it, start the agent, type the prompt. Moshpit's New Agent Task does those four things from one sheet — and it does them in a fresh git worktree, so whatever you had checked out stays exactly as it was. This is a herdr feature; tmux has no equivalent."
---

## The sheet

The <b>+</b> on the AGENTS section of a herdr connection opens it. Four fields:

![The New Agent Task sheet: a Repo row set to payments-api, a branch name fix-webhook-retry, an Agent row set to claude, and a first message reading Cap the backoff at 30s and add a test](/06-new-task.jpg)

- **Repo** — found for you. Moshpit resolves the directories your panes are already sitting in to their repository roots, and scans your home directory for the rest. “Other…” takes a path you type.
- **Branch** — the new branch name. Checked against git's rules before anything is sent, so a bad name fails instantly instead of after a round trip.
- **Agent** — which agent to start, from the list herdr knows how to detect.
- **First message** — optional. Typed into the agent once it is up, so you can hand over the job and put the phone away.

## What runs on the host

In order: a `worktree create` that makes the branch, the checkout and a herdr workspace whose pane is already in that directory; then the agent command typed into that pane, exactly as you would have typed it; then your first message, if you wrote one.

The checkout lands under `~/.herdr/worktrees/<repo>/<branch>`, not next to your repository — your working directory is not touched at all.

## Cleaning up

Long-press the workspace in the tree and choose **Remove Worktree**. It appears only on workspaces herdr created from a repository, because an ordinary workspace has no directory to delete.

<b>The first attempt never forces.</b> If the checkout has uncommitted or untracked files, herdr refuses and leaves everything intact; only then does Moshpit ask a second, sharper question — the one that says those changes exist nowhere else. That safety net is herdr's own, and following it beats deciding for you.

## Limits

- <b>A big repository takes a while to check out.</b> The sheet shows progress; it has not hung.
- **An existing branch name fails**, with herdr's own error shown verbatim. Pick another name.
- <b>No custom path.</b> Worktrees go where herdr puts them; typing a path on glass was not worth the field.
- <b>Agent arguments are not guessed.</b> Moshpit runs the agent's default command. If you want flags, type them in the first message or start it yourself in a pane.
