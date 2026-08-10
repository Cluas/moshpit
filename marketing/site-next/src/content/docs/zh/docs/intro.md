---
title: "Moshpit 是做什么的。"
description: "一段话说完，以防你是从搜索来的：Moshpit 是一个付费的 iPhone / iPad 终端，给那些在自己机器上跑 AI coding agent 的人用。它通过 SSH 或 Mosh 连接、原生渲染 tmux 和 herdr，而且——这是别的 iPhone 终端都没做的——把 agent 那句「我需要一个人」送到你的锁屏上，连回答用的按钮一起。"
---

## 它解决什么问题

coding agent 跑很久，然后停下来，因为它想要做某件事的许可。 如果你不在桌前，它就一直等。很多人回来才发现：agent 前十分钟就想完了， 剩下一下午都举着一个问题站在那儿。

![Moshpit's home screen: an Agents section listing Claude Code needing you, codex working and an idle claude, above a tree of workspaces](/01-agents.jpg)

Moshpit 存在的全部理由就是补上这个缺口：问题送到你手机上， 你不用解锁进任何东西就能回答它。

## 它是什么

- <b>一个真正的终端。</b>SSH 与 Mosh、正经的 VT 模拟、外接键盘支持、 主题，以及你自己搭的快捷键栏。不是一个聊天窗口外挂了个终端。
- <b>一个多路复用器客户端。</b>tmux 走控制模式，或者 [herdr](/zh/docs/herdr)——那个为 coding agent 而写的多路复用器—— 都是原生渲染，而不是把桌面塞进手机屏幕里假装。
- <b>一个 agent 控制台。</b>谁被卡住了、卡了多久，以及在锁屏或灵动岛上 一次点击就回答它。

## 它不是什么

- <b>不是一项服务。</b>没有 Moshpit 账号、没有中转、没有云——因为根本没有 Moshpit 服务器。是你的手机在跟你的机器说话。
- <b>不是文件管理器。</b>没有 SFTP 浏览器，也不能把服务器挂进「文件」App。 如果那是你每天在做的事，别的 App 做得很好，而这个不做。
- <b>不是 agent。</b>Moshpit 不思考、不总结、不写代码。它只是把你的 agent 的输出带给你，再把你的按键带回去。

## 它需要你有什么

一台你本来就能 SSH 进去的机器。其余都是可选的：`mosh` 换来漫游，`tmux` 或 `herdr` 换来会话持久和 agent 相关能力。 缺什么 Moshpit 会直说并把命令给你——它绝不自作主张在你的主机上装东西。

## 第一天就该知道的那个限制

没有推送服务器，所以 agent 状态只在 Moshpit 处于前台、 或刚进后台的短时间内更新。一旦 iOS 挂起连接，锁屏上就停在它最后看到的样子， 直到你回来。这是「中间什么都没有」的代价，两种传输都一样。

## 接下来去哪

想直接开始就看[配置一个连接](/zh/docs/setup)； 还在选多路复用器就看[选 tmux 还是 herdr](/zh/docs/multiplexers)； 还在判断这个 App 到底适不适合你，就看[和别的比怎么样](/zh/compare)。
