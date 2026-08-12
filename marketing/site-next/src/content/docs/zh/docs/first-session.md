---
title: "跑通第一个会话。"
description: "你已经添加好主机了（还没有的话，先看配置一个连接）。这一页讲的是之后那五分钟：第一次连接长什么样、顶上那条栏是什么意思，以及怎么不用瞎找就能走动。"
---

## 连接

点主页上的连接卡片。第一次连任何主机都会弹出 **New Host**， 上面有密钥指纹和在服务器上核对它的命令。能核对就核对一下；点 **Trust** 继续， **Cancel** 中止。Moshpit 按主机记住这个答案，而且以后那个密钥要是变了， 它会大声警告。

## 看懂顶栏

从左到右：返回、传输标记（**SSH** 或 **MOSH**，漫游时转琥珀色）， 然后是面包屑。面包屑的每一段都是一个按钮，点开对应的选择面板：

![A live terminal pane with the top bar above it: the back control, a transport pill reading SSH, and a breadcrumb naming the session, window and pane](/02-agent-terminal.jpg)

- **第一段**——tmux 上是 session，herdr 上是 workspace。
- **第二段**——tmux 上是 window，herdr 上是 tab。
- **第三段**——窗格。里面住着 agent 时，它会改成显示 agent 的名字加一个 状态点；那个 agent 等你回话时整段转琥珀。

不用多路复用器时没有树可显示，这条栏就显示连接身份。

## 打字

点终端调出键盘。键盘上面那条是快捷键栏：默认是 `esc`、 `tab`、`^C`、`^L`、粘贴和方向键—— 全部可替换，见[键盘与快捷键](/zh/docs/keyboard)。 外接硬件键盘也能用，常见的 control 组合键直接透传。

终端拿到键盘之后，点哪儿光标就去哪儿：这一下会作为鼠标点击发出去， 认鼠标的程序（Claude Code 的输入行、vim、less）把它读成「光标移到这里」， 于是长提示词中间那个字符点一下就到，不用一格一格按方向键。 纯 shell 不发——它只会把点击的转义序列当文本打进命令行。

## 走动

- **切窗口或窗格**——点对应的面包屑段，在面板里挑。tmux 上还可以在终端上 横向划动。
- **滚动**——纵向划。Moshpit 会判断这该变成真正的回滚， 还是给全屏程序的滚轮事件；见[滚动与回滚](/zh/docs/scrolling)。
- **新建、重命名、关闭**——每个面板里的 <b>+</b> 负责新建；长按一行是重命名和关闭。

## 离开再回来

回到主页，会话还活着：卡片显示 **LIVE** 和已连接时长。 息屏、飞行模式、隧道断了之后 Moshpit 会自己重连，并把你放回原来那个窗格。

*撑不住*的是 App 在后台被挂起很久——iOS 会停掉连接， agent 状态在你回来之前不再更新。真想彻底断开时，点卡片里的 **Disconnect**。

## 接下来

如果你要跑 agent，下一个值得做的是 [用 iPhone 跑 Claude Code](/zh/guide/claude-code)。 如果已经有地方看着不对，[排障](/zh/docs/troubleshooting)是按症状写的。
