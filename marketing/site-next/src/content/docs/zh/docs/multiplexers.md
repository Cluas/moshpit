---
title: "tmux 还是 herdr —— 按连接各选各的。"
description: "多路复用器让你的活儿在断线时不死，也让 Moshpit 有比「一长条滚动」更好的东西可渲染。Moshpit 会说两种。它们不能互换，而且 Moshpit 绝不会偷偷替你换——因为两者的会话毫不相干，换一下等于给你看别人的工作。"
---

## 简短答案

- <b>本来就活在 tmux 里？</b>选 tmux。Moshpit 附着到你已有的会话上， 绝不改动它们的样式。
- <b>主要在跑 coding agent？</b>选 [herdr](/zh/docs/herdr)。 agent 状态零安装、窗格满宽渲染，而且能从手机上开一个隔离任务。
- <b>都没装，也不确定？</b>选 None。Moshpit 当普通 SSH 客户端也完全够用， 等你真的想念某个功能时再加。

## 并排看

|  | tmux | herdr |
| --- | --- | --- |
| 在哪儿 | 几乎每台服务器上都已经有了 | 自己装（一个二进制） |
| agent 状态 | 装 hook，之后准确 | 协议字段——什么都不用装 |
| 它管这些叫什么 | session · window · pane | workspace · tab · pane |
| 手机上的渲染 | 完整布局，含分屏 | 单窗格，满宽 |
| 你笔记本上的窗口尺寸 | 共享——最小的客户端说了算 | 不受影响——手机只改自己那个窗格 |
| 手机上开隔离任务 | — | 一个表单搞定 worktree + workspace + agent |
| 成熟度 | 二十年 | 新，而且在快速变化 |

## 各自真正的短板

**tmux** 的窗口尺寸是所有已连接客户端共享的，所以手机加入你的会话 可能会把笔记本上的布局挤扁。Moshpit 用 pin 窗口的办法绕开，但这个约束是真实存在的。 另外 agent 状态需要在主机上装 hook，不装的话 Moshpit 只能靠读输出来猜。

![A tmux session rendered natively: the window bar across the top and a full-width terminal pane below it](/07-tmux-terminal.jpg)

**herdr** 还年轻。它的直连是按窗格独占的，所以两个客户端连同一个 窗格会互相抢；App 会退避并告诉你，而不是默默闪烁。它的树是轮询的，所以你在笔记本上 的改动可能要几秒才反映过来。而且 0.7.3 不上报前台命令，所以窗格面包屑会兜底成 `pane N`。

## 改主意

多路复用器是连接的属性，在 **编辑 → ADVANCED** 里改，然后重连。 什么都不会丢——另一个多路复用器的会话还在主机上，跟你离开时一模一样， 因为 Moshpit 从没为了切换而杀掉任何东西。

## 走 Mosh 时两者都会降级

Mosh 传的是渲染完的屏幕差分，承载不了 tmux 的控制模式协议， 也承载不了 herdr 的帧协议。走 Mosh 时你在终端里看到的是多路复用器自己的文字界面， 而不是原生渲染——照样能用、照样漫游，只是没有原生列表。 详见 [Mosh 与漫游](/zh/docs/mosh)。
