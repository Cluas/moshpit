---
title: "开一个任务，不碰你当前的工作区。"
description: "给 agent 派活通常意味着人要在机器前：建分支、在里面开个终端、起 agent、敲 prompt。Moshpit 的 New Agent Task 用一个表单做完这四件事——而且是在一个新建的 git worktree 里做，所以你原本 checkout 着的东西一动不动。这是 herdr 的能力，tmux 没有对应物。"
---

## 这个表单

herdr 连接的 AGENTS 区右上角那个 <b>+</b> 打开它。四个字段：

![The New Agent Task sheet: a Repo row set to payments-api, a branch name fix-webhook-retry, an Agent row set to claude, and a first message reading Cap the backoff at 30s and add a test](/06-new-task.jpg)

- **Repo**——自动找出来。Moshpit 把你窗格已经待着的目录反查成仓库根， 再扫一遍你的 home 目录补充其余的。「Other…」可以手输路径。
- **Branch**——新分支名。在发出去之前先按 git 的规则校验， 所以名字不合法会立刻失败，而不是等一个来回。
- **Agent**——起哪个 agent，从 herdr 能识别的列表里选。
- **First message**——可选。agent 起来之后替你敲进去， 这样你交代完就能把手机放下。

## 主机上实际跑了什么

按顺序：一条 `worktree create` 建出分支、checkout， 以及一个窗格已经在那个目录里的 herdr workspace；然后把 agent 命令敲进那个窗格， 和你自己敲的一模一样；最后是你写的第一句话（如果写了）。

checkout 落在 `~/.herdr/worktrees/<仓库>/<分支>`， 不在你的仓库旁边——你的工作目录完全不受影响。

## 清理

在树里长按那个 workspace，选 **Remove Worktree**。 它只出现在 herdr 从仓库建出来的 workspace 上，因为普通 workspace 没有目录可删。

<b>第一次尝试绝不加 --force。</b>如果 checkout 里有未提交或未跟踪的 文件，herdr 会拒绝并保留一切；只有到这时 Moshpit 才会问第二个、更尖锐的问题—— 那句「这些改动不存在于任何别处」。这张安全网是 herdr 自己的， 顺着它比替你做决定好。

## 限制

- <b>大仓库 checkout 要一会儿。</b>表单会显示进行中，它没有卡死。
- **分支名已存在会失败**，并原样显示 herdr 自己的错误。换个名字。
- <b>不能自定义路径。</b>worktree 就放在 herdr 放的地方； 在玻璃上敲路径不值得为它加一个字段。
- <b>不替你猜 agent 参数。</b>Moshpit 跑的是 agent 的默认命令。 要加参数就写在第一句话里，或者自己在窗格里起。
