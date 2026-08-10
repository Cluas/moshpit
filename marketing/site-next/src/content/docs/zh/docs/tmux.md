---
title: "使用 tmux"
description: "Moshpit 以 tmux 控制客户端的身份 attach 上去，把你的 session、window、pane 用原生 iOS 视图画出来。这页讲清楚三件事：它做了什么、在你服务器上改了什么、以及到哪儿为止。"
---

前提是主机上有 tmux，在 `PATH` 里——或者你在连接表单的 **Custom tmux Path** 里填了路径。没别的了。下面[装 hook](#agents) 那节是可选的， 跟连接本身无关。

没有 tmux 的话，连接不会失败，只会降级：一个纯 SSH 的单 pane，加一条横幅 <b>“tmux not found on this host — plain SSH session.”</b>，旁边的 **Install tmux** 会按你的包管理器把命令摆出来，你点了才执行。Moshpit 绝不会偷偷把你切到另一个多路复用器 ——tmux 和 herdr 各自持有互不相干的 session。另外，一旦你填了自定义 tmux 路径，这套探测整个 跳过：探测只走 `PATH`，而你亲手填路径本身就是为那个二进制背书。

## 只往登录 shell 里写一行

Moshpit 不是把 tmux 那套字符界面塞进一块小屏幕，而是以控制客户端的身份 attach——`tmux -CC`，跟 iTerm2 用的是同一套控制模式——然后自己渲染。连上那一刻 写进去的就是这行：

```sh
tmux set -g history-limit 50000 2>/dev/null; tmux -CC attach
```

`attach` 不带 `-t`，接的是你最近用过的那个已有 session。 <b>Moshpit 自己绝不执行 `tmux new`。</b>服务器上一个 session 都没有时，你看到的是 一个把话说明白的空状态，和一颗要你自己按的 **Create Session**。

发现（discovery）是同一条通道上的三条命令，树有变动就重跑。`-a` 是服务器上的全部 session，所以首页那棵树能同时展开好几个：

```
list-sessions -F '#{session_id} #{session_attached} #{session_name}'
list-windows -a -F '#{session_id} #{window_id} #{window_index} #{window_layout} …'
list-panes   -a -F '#{pane_id} #{window_id} #{pane_index} … #{pane_current_command}'
```

- <b>attach 有 22 秒的期限。</b>tmux 一直不确认，首页卡片就变成 *stalled*， 并给出「Attach didn't complete — tmux never confirmed.」和一颗重试。注意这时 SSH 本身是活的， 没接上的是 tmux 这一握手。
- <b>控制模式跑不了 mosh。</b>mosh 传的是屏幕差分，不是按行分帧的协议流。所以 mosh + tmux 这条路上，Moshpit 在 mosh 里渲染 tmux 原本那套全屏界面，同时**另开一条 sidecar SSH 连接**跑 `-CC`，只为喂饱面包屑和那几个面板。这条 SSH 连不上， 你拿到的就是一个光秃秃的 `tmux attach`，一个原生面板都没有。
