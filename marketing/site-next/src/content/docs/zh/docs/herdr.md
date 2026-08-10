---
title: "用 herdr"
description: "herdr 是专门为 CLI coding agent 写的多路复用器。连 herdr 的时候， Moshpit 不用你在主机上装任何东西，就知道每个 agent 在干什么。这一页讲安装、术语、键位、 Agents 区、隔离任务——最后一节讲清楚哪些事它做不到。"
---

[herdr](https://github.com/herdrdev/herdr) 是一个 Rust 单二进制。Moshpit 支持它，是因为它把三件苦活从手机搬到了服务端——而 tmux 那条路上， 这三件事还得 Moshpit 自己扛。

### 协议里的一等字段

<b>Agent 状态。</b>每个 pane 上都有 `agent_status`，并自动从 pane 上卷到 tab、再到 workspace。tmux 上要你先装 hook，往 `@moshpit_*` 里写状态。

### 服务端决定去向

<b>滚动。</b>`terminal.scroll` 是协议动作，一次滑动该变成 mouse report 还是翻 scrollback 由 herdr 判。tmux 上是 Moshpit 自己读 `#{mouse_any_flag}` 再猜。

### 只影响自己那个 pane

<b>尺寸。</b>直连客户端的 resize 只打到自己那个 pane 的 PTY。tmux 上手机一连，整个 window 对所有客户端都被拉小——Moshpit 那套钉窗口尺寸的机制就是为此存在的。

<b>它是 tmux 的另一种选择，不是替代品。</b>两者是独立的服务器， 装的是毫不相干的会话。选哪个按连接决定，Moshpit 绝不替你换成另一个——herdr 没装就默默去 attach tmux，等于把别人的活儿端到你面前说这是你的。

<b>版本。</b>设计基准是 herdr main / v0.8.0，protocol 19； 实机验证跑的是 0.7.3，protocol 16——当时 `brew install herdr` 装到的就是它， 差了三个协议版本。解码器按「字段缺了就降级这一处细节、绝不让整份读取失败」写； 0.7.3 上会少点什么，这一页会明说。
