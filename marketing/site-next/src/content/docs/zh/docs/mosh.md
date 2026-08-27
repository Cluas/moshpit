---
title: "Mosh 与漫游"
description: "mosh 到底给你带来什么、要让它跑起来得开哪些口子、以及屏幕黑着只剩一个光标闪时该怎么办。 包括目前只存不用的那四个开关。"
---

mosh 是可选的。不开它，Moshpit 就是一个正常的 SSH 客户端，除了漫游之外一切照旧。 mosh 换来的是「手机切网络时会话不死」——代价也先说清楚： <b>走 mosh 时，tmux 和 herdr 画的是它们自己的全屏 TUI，Moshpit 不再原生渲染它们。</b> 原生 sheet 还在，由旁边另一条 SSH 连接喂着；变的是终端画面本身，那是多路复用器自己画的。 这不是待修的 bug，原因写在[下面那一节](#tui)。

## 它会漫游

当 iOS 报告出现了更优的网络路径——Wi-Fi 起来了、或者切到蜂窝——Moshpit 会把会话标为 roaming，并立刻从新路径推一个包出去，让服务端把地址迁到新的这个上。

整个过程不重新协商。mosh 的 State Synchronization Protocol 是无连接的：客户端自己维护状态号， 没有握手要重做。收到第一个通过认证的回包，roaming 标记立刻清掉。

## 它扛得住休眠

回到前台时，如果 UDP 连接在 iOS 收走 socket 期间进了 failed 或 waiting 状态， Moshpit 会把它重启，并马上 flush 一个包让服务端重新定位到你的地址。

SSH 会话在长时间挂起后走的是彻底重连；mosh 会话走自己的 resume 路径， 因为 SSP 一般能自愈。但它做不到的是「你不在的时候继续跑」——见 [最后一节](#suspend)。

## 它容忍丢包

只有当一个屏幕 diff 的 `old_num` 正好等于 Moshpit 已经应用的状态时才会被应用， 这样 ANSI 才拼得对。出现空洞时，Moshpit 会 ack 自己真实的状态、让服务端重新算 diff， 而不是硬套一个错的。

1 秒一次的心跳负责保住 NAT 映射，也让 ack 一直在流动。

Moshpit 不去调用 `mosh` 这个可执行文件，也没有打包 mosh 的 C++。 它是同一套协议的一次从零实现。

实现

## 净室实现，拿 RFC 的向量对过

传输层是 Swift 直接架在 Apple 的 Network framework 上。加密是 AES-128-OCB3—— RFC 7253 里的 `AEAD_AES_128_OCB_TAGLEN128` 参数集—— 测试套件跑的是 RFC 附录 A 自带的测试向量，包括它那条「篡改 tag 必须拒绝」的用例。

- 按公开的协议描述写的，不是从 mosh 的 GPL-3.0 C++ 翻译过来的。完整声明在仓库的 `NOTICES.md` 里
- zlib 外壳是围着 Apple 的裸 DEFLATE 手写的，为的是跟 mosh-server 的预期对上
- 数据报按 1300 字节分片
- 「mosh」是 Keith Winstein 的项目，这里用这个名字只是为了指明在说哪个协议

![Moshpit 的一个 Mosh 会话，正在显示 git log 输出，标题栏上是 MOSH 标签](/09-mosh.jpg)

## 一处故意跟上游 mosh 不一样的地方

没被 ack 的击键，10 秒之后就不再重传了。上游 mosh 是永远重传—— 链路只是抖一下时这么做是对的，链路真的死透了时就是错的： 你对着冻住的屏幕敲的那些键，会在几分钟后突然回放进 shell； 而你重连后已经重新敲过一遍的命令，会被执行第二次。

resize 事件是例外——它是幂等的状态，不是动作。 过期的状态以空 diff 的形式排掉，好让状态编号继续连贯，因为服务端是按编号追踪输入流的。

Moshpit 在花掉一个来回之前会先探测主机。没装 `mosh-server` 时，你永远不会看到一条裸的 `command not found`——它会降级。

## 没装也照样给你一个终端

Moshpit 会复用刚刚认证过的那条 SSH 会话，把你接成一个普通 shell， 并顶一条可关闭的横幅：

```sh
⚠  mosh-server not found — connected over SSH instead.
   Install mosh                                     ×
```

首页那张连接卡片上的 MOSH 标签同时会变灰并挂一个警告三角， 免得卡片继续声称自己有漫游。你选的多路复用器不会跟着一起丢：选 tmux 的话， 这条降级后的 SSH 会话照样拉起 `tmux -CC`；选 herdr 的话， 走的是原生帧通道，和你本来就用 SSH 连过去时一样，是整屏宽的单个 pane。 掉的是漫游，不是多路复用。

## Install Assist

点 **Install mosh** 会弹出一个 sheet，里面是按探测到的包管理器生成的命令：

```sh
sudo apt-get install -y mosh        # Debian、Ubuntu
sudo dnf install -y mosh            # Fedora
sudo yum install -y mosh            # 老一点的 RHEL / CentOS
sudo pacman -S --noconfirm mosh     # Arch
sudo apk add mosh                   # Alpine
brew install mosh                   # macOS
```

只有三个动作，没有第四个。**Run in terminal** 把命令粘进你看得见的那个 shell 并回车， 所以 sudo 提示和全部输出都在你眼皮底下走完；**Copy command** 只是复制； **Re-check** 重跑一次探测。Moshpit 绝不静默安装。 如果这条连接同时用 tmux，它会把 `tmux mosh` 合成一条命令给你。 要是探测压根没找到包管理器，那就没有命令可给：sheet 会直说，只留一段通用说明和同一个 **Re-check** 按钮。

重连会重跑能力探测，所以你后来才装上的依赖，下次连接就自动被认出来， 不需要去改任何设置。

## Homebrew 的坑

`ssh host command` 起的是非登录、非交互 shell， 它不会 source `.zprofile` 或 `.bash_profile`， 所以 `brew shellenv` 往 PATH 里加的东西在这条通道上是看不见的—— 同一个二进制，你手动登录时跑得好好的，探测时却「不存在」。

Moshpit 在探测和启动这两处都会把同一批目录前置进去：

```
/opt/homebrew/bin   /opt/homebrew/sbin   /usr/local/bin   /usr/local/sbin   $HOME/.local/bin
```

如果这还不够，就去连接表单 ROAMING · MOSH 里的 **mosh-server path** 填绝对路径——不是设置页里那个叫 Server binary 的行， 连接流程根本不读它（见[设置那一节](#settings)）。 填了绝对路径就意味着你为它背书：Moshpit 原样使用、不再前置 PATH， 并且对这条连接完全跳过「二进制是否存在」的检查。

三件事——第三件绊住的是绝大多数连自己局域网机器的人。

### 22 端口，或者你自己的 SSH 端口

<b>TCP.</b> 没有一次成功的 SSH 登录，mosh 根本引导不起来。不存在「只跑 mosh」这种模式。

### 60000–61000，双向

<b>UDP.</b> 服务端入站要开，手机出站也要开。`mosh-server` 每个会话在这个区间里绑一个端口。

### 本地网络权限

<b>iOS.</b> 只跟局域网主机有关。拒绝之后 iOS 会把发往 192.168.x.x 的 UDP 直接丢掉—— 表现就是 SSH 连得上、mosh 一片黑。公网主机不受影响。

## Moshpit 实际执行的那条命令

```sh
PATH="$PATH:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin:$HOME/.local/bin" \
  mosh-server new -s -c 256 -p 60000:61000 -l LANG=en_US.UTF-8
```

`new -s` 把 UDP socket 绑到这条 SSH 连接的服务端 IP 上， 这样数据报走的就是你刚刚 SSH 登录走过的同一条路。`-c 256` 是要 256 色。 服务端打印一行之后就 fork 到后台：

```
MOSH CONNECT 60001 <22 个 base64 字符 = 一把 16 字节 AES-128 密钥>
```

Moshpit 读到这行之后**就把 SSH 连接关掉**，之后画面全部走 UDP。 这也是为什么「连上之后再封 SSH」不会让 mosh 会话的画面停掉——而封 UDP 会。 有一处例外要说清楚：连接如果用了 tmux 或 herdr，Moshpit 会*另开*一条 SSH 连接跑控制面， 这条在整个会话期间都需要 TCP 通着。

## 端口区间填错时是降级，不是报错

- 起始端口不可用（非正数，或者大于 65535）→ 整个 `-p` 被丢掉， 由 mosh-server 用它自己的默认区间
- 结束端口不可用 → 起始端口变成单端口锁定，`-p 60000`
- 只有结束端口确实大于起始端口时，才会是 `-p start:end`

设置里那个编辑页在保存前会校验：非数字、超出 1–65535、From 大于 To，各有各的提示。

全局默认在 设置 → MOSH · ROAMING；按连接的那份在 Add/Edit Connection 表单的 ROAMING · MOSH 分组里。这两处一共有四行是只存不读的， 与其等你自己撞上，不如在这里点名。

设置 → Mosh · Roaming

## 四行，按你会用到的顺序排

这个分组自己的脚注就说得很直白：<i>「Mosh runs over UDP and survives IP changes. If your server is behind a strict firewall, open the port range above outbound from your iPhone.」</i>

- 关掉 **Mosh by default**，新主机就是纯 SSH；已有的连接保持它们保存时的设置
- Moshpit 真正会去执行的路径只有一个：连接自己那行 `mosh-server path`。 留空就跑裸的 `mosh-server`
- 连接表单里显示的端口区间是设置里的镜像，在那儿是只读的——它在你保存表单时被抄进这条连接， 所以改了全局区间，之前保存过的连接不会自动跟上，得重新打开保存一次

![Moshpit 设置页的 Mosh 与漫游分组：Mosh by default、Predictive Echo、Server binary、UDP port range](/13-mosh-settings.jpg)

| 设置项 | 位置 | 默认值 | 作用 |
| --- | --- | --- | --- |
| Mosh by default | 设置 | 开 | 新建的 SSH 主机连接时自动用 mosh-server 包一层 |
| Server binary | 设置 | `/opt/homebrew/bin/mosh-server` | 只存不读，连接时根本不看它——见下 |
| UDP port range | 设置 | 60000 – 61000 | From / To，保存连接表单时抄进那条连接，再作为 `-p` 传下去。1–65535，且 From ≤ To |
| Use Mosh | 连接表单 | 跟随默认 | 按主机选协议。也可以在终端顶栏的传输标签上实时切换 |
| `mosh-server path` | 连接表单 | 空 | Moshpit 真正会执行的那个路径。留空就用 PATH 上的 `mosh-server`——比写死绝对路径更通用 |
| Predictive Echo · Predict Mode | 两处都有 | Adaptive | 只存不用——见下 |
| Trail on predict | 设置 → Cursor | 开 | 只存不用——见下 |
| Roam on Cellular | 连接表单 | 开 | 只存不用——开不开都会漫游 |

## 设置里的 Server binary 没接上

这一行在、编辑页也能存，但连接流程从头到尾不看它。 启动 mosh 只读一个路径：这条连接自己的 `mosh-server path`，留空就是裸的 `mosh-server`。所以那个默认值 `/opt/homebrew/bin/mosh-server` 并不是你的 Linux 主机上正在跑的东西——它们跑的是上面那段扩展 PATH 里解析出来的 `mosh-server`，而这恰好就是你想要的行为。 <b>真要指定某个二进制，请填连接表单里的那个框，不是这一行。</b>

## 预测回显（predictive echo）没有实现

这个选择器是真的、选完也真的存下来了，但在当前版本里它什么都不改变。 传输层里根本没有本地预测引擎：Moshpit 只渲染服务端发来的字节。 协议里那个 echo-ack 字段被当成不认识的字段直接跳过，这个模式从来没被传给 UDP 传输层， 远端命令行上也完全没有它的踪影。**Trail on predict** 处境一样—— 它只驱动设置页里的一小块预览色卡，此外什么都不做。 漫游横幅上那个「Predict ON」小标签是写死的文字，不是状态读数。

本站别处的文案目前仍把预测回显当成已发布功能在写。它不是，那些文案是错的。 在高延迟链路上，你看到自己敲的字符的时机，就是服务端回显它们的时机，跟走 SSH 一样。

**Roam on Cellular** 同样是只存不读。漫游不是可选项： 它由系统告诉 Moshpit「出现了更优路径」来驱动，这个开关开着关着都会发生。 与其让你为此提一个 bug，不如我们在这里写清楚。

mosh 传的是渲染完的屏幕差分，不是一条裸字节管道，按行分帧的控制协议撑不过去。 实测验证过：`tmux -CC` 在服务端确实起来了，但一条 `%begin` 或 `%output` 都回不来。herdr 的帧协议同理。

## 走 SSH

原生渲染

- **tmux** —— 控制模式：Sessions、Windows、Pane 都是真列表
- **herdr** —— 帧协议：整屏宽的单个 pane，原生的 workspace 和 tab 列表
- 面包屑、滑动切换、所有 sheet 都来自同一个控制面

## 走 mosh

它们自己的全屏界面

- **tmux** —— mosh shell 里那个普通的 attach TUI，用 Ctrl-b 操作，跟着 mosh 一起漫游
- **herdr** —— herdr 自带的 TUI，Moshpit 只是渲染器
- **另开的一条轻量 SSH 连接**喂原生 sheet：渲染走 UDP，控制走 TCP

这件事 Moshpit 是在你连接之前告诉你的，不是连上才说。 连接表单的脚注最后一句就是它：<i>「With Mosh, herdr runs its own terminal UI; native rendering needs SSH.」</i>另有一点要从那个 TUI 里带走：herdr 的前缀键同样是 Ctrl-b， 但前缀之下的绑定跟 tmux 不是一套——**Ctrl-b q 在 tmux 里是列 pane，在 herdr 里是 detach**。 App 里的 sheet 会按各自的多路复用器打印真实按键；你的肌肉记忆没有这层保护。

## 双传输在实际使用中意味着什么

- <b>mosh + tmux 是同时两条连接。</b>如果 SSH 被封而 UDP 通， 控制面 sidecar 起不来——Moshpit 会退回到往 mosh shell 里直接敲 `tmux attach`。你拿到的是 tmux，但拿不到原生 sheet。
- <b>两个控制面都骑在 SSH 上，而 iOS 会在后台把它杀掉。</b>回到前台时 mosh 渲染通道自愈，`-CC` sidecar 或 herdr 轮询器则被重建。
- **tmux 窗口是通过 sidecar 钉到手机的字符网格上的**， 因为 mosh 渲染器自己没法给窗口设尺寸。没有这个钉住， 它那个 80×24 的 PTY 会把 TUI 卡在屏幕上半部分的 80×24 里。
- <b>滚动回看是这条规则的例外。</b>sidecar 的 copy-mode 不会去重绘另一个 mosh 客户端， 所以一次滑动只能从 mosh 这条通道自己走。窗格里如果是抢了鼠标的程序——Claude Code、vim、less ——收到的是转发过去的滚轮事件，由它自己滚；如果是普通 shell，才发 copy-mode 按键，粒度是整页。
- **mosh + tmux 的引导最长可能要 15 秒左右**（第二次 SSH 握手 + 轮询）， attach 循环最多等 6 秒，等控制面落到某个 session 上。
- <b>Moshpit 绝不替你建 tmux session。</b>如果 sidecar 活着但服务端一个 session 都没有， 它就让 mosh shell 保持原样，等你自己显式创建第一个。

socket 到了 ready，你敲的键也送到了服务端，但回程数据报一个都没到。 黑屏、光标还在闪、没有任何报错。这几乎总是你此刻所在网络上的网络隧道、中间节点或防火墙 ——放行了出站 UDP，丢掉了入站的。

## Moshpit 会主动发现它

连接进入 ready 并开始 flush 之后，会挂上一个**8 秒的看门狗**。 这个时长是故意给得宽的：第一个回包在第一次 flush 之后一个 RTT 就该到， 所以哪怕延迟好几秒的链路也能远远赶在它之前应答。只有*什么都不回*的链路才会把它耗完。

只要落地**任何一个**数据报，看门狗立刻解除—— 哪怕那个包解密失败或者解析失败也算——所以「能用但很慢」的链路永远不会误触发。 而且它只在第一个数据报之前武装：会话中途卡住是另一回事， 那种情况通常靠漫游能自己恢复，所以它不管。

一旦触发，终端顶部会出现一条琥珀色横幅：

```
⚠  Mosh isn't receiving data — your network may be
   blocking UDP (VPN, proxy, or firewall).

   Switch to SSH                                    ×
```

<b>这条横幅从不挡住终端。</b>击键照样送得到服务端，只是渲染被饿住了。 **Switch to SSH** 会翻转这条连接的协议、存下来、然后用 TCP 重连—— TCP 走的正是那条把你 UDP 吃掉的路径，而它能通。 点 × 只是关掉横幅，mosh 会话原样保留。

你也可以随时自己切：点顶栏的 **MOSH** 标签，确认 <i>「Switch to SSH?」</i>。重连会重跑能力探测，所以不会有什么状态一直卡在降级里。

## 怎么读那几个计数器

在 mosh 会话上长按传输标签，会弹出 **MOSH DIAGNOSTICS**—— 四个数字，是给你截图发出来用的，不是给你欣赏的：

**datagrams** 收到的 UDP 数据报总数，在解密和解析之前就计数。标签显示「已连接」而这个数很低， 说明问题在网络或 NAT，不在 App。

**applied** 实际应用到的最高显示状态号。*datagrams* 在涨而它一直是 0， 说明到目前为止每个 diff 都掉进了「空洞」或「解析失败」分支—— 服务端有内容排着队，但一条都没被接受。

**parse fails** 到了但解不开的 diff。数字高，说明这台主机的输出里有什么东西解不出来。

**gaps** 乱序到达的 diff。丢包链路上偶尔几个是正常的；但如果它一直涨而 *applied* 一直不动，说明客户端卡在反复重新求差分上了。

<b>「No datagrams yet.」</b> 这句话就是判据。一个包都没到过——那是回程路径死了，不是渲染 bug。 别再查 Moshpit 了，去查网络。

这个浮层之所以存在，是因为这类故障要真实的丢包、乱序， 或者某台远端特定的 shell 配置才复现得出来——这些环回测试全都造不出来。 对一个谁也没法从外部检查的会话来说，把这个浮层截图发出来是唯一实用的诊断手段。

## 提 issue 之前先按这个顺序过一遍

- <b>换蜂窝试试。</b>5G 上正常、Wi-Fi 上不正常，那答案是这个 Wi-Fi 网络，不是 App。
- **把手机上的网络隧道功能关掉**再连。分流模式的网络配置是最常见的元凶。
- <b>连的是局域网主机？</b>去手机的 设置 → Moshpit → 本地网络 看一眼。 被拒绝的话，iOS 会把发往 192.168.x.x 和 10.x.x.x 的 UDP 直接丢掉，而且谁都不告诉。
- **查云上的防火墙 / 安全组**——UDP 区间入站开了吗？ 开的是不是 Moshpit 里配置的*同一个*区间？
- <b>在同一个网络下用笔记本跑一次 `mosh`。</b>如果它也黑屏， 那就不是 Moshpit 的问题。

:::note
Moshpit 的保活是 12 秒一次的定时器，而且**只在前台跑**——iOS 在后台会挂起定时器。 离开超过 20 秒之后回来，Moshpit 会强制重连，而不是去信任那个在半开通道上会误报「还活着」的存活探测。 mosh 在这条回归路径上是例外，走它自己的 UDP resume； 但 tmux 或 herdr 的控制面骑在 SSH 上，所以会被重建。

实时的 agent 状态跟着 App 一起暂停：**完全被挂起的 App 不会去轮询**，灵动岛在那期间停止更新，等你回来才补上。但 agent 的提醒在挂起期间照样送达——那是主机自己端到端密封推送出来的（[细节](/zh/docs/agents)）。

进入后台时，Moshpit 还会丢掉所有缓存的已解密密钥， 逼下一次回前台重新走一遍 Face ID 门控的钥匙串读取； 同时把每个被钉住的 tmux 窗口尺寸交还给服务端。
:::

<details>
<summary>我一定要用 mosh 吗？</summary>

不用。不开它 Moshpit 就是一个正常的 SSH 客户端。在设置里关掉 **Mosh by default**，或者在某条连接上不勾 **Use Mosh**。 除了漫游，其他表现完全一样；而 tmux 和 herdr 的原生渲染只有走 SSH 才有—— 网络本来就稳的话，SSH 是更划算的那一边。

</details>

<details>
<summary>mosh 是用来替代 SSH 的吗？</summary>

不是。没有一次成功的 SSH 登录，mosh 引导不起来：是 SSH 去启动 `mosh-server` 并把会话密钥带回来的。之后 Moshpit 才关掉 SSH 连接、转到 UDP。 所以 TCP 22（或你自己的 SSH 端口）必须可达。

</details>

<details>
<summary>mosh + tmux 下我的终端为什么卡在 80×24？</summary>

那是 sidecar 没起来。mosh 渲染器自己改不了 tmux 窗口尺寸， 这个「钉住」动作来自走 SSH 的 `-CC` 控制连接。 SSH 被封的话，你拿到的就是 tmux 的默认 PTY 尺寸。这台主机改用 SSH， 或者把 SSH 和 UDP 一起放开。

</details>

<details>
<summary>这是真的 mosh，还是个仿的？</summary>

它跟真实的、未经修改的 `mosh-server` 说的是真协议—— 但客户端是 State Synchronization Protocol 的独立净室 Swift 实现， 按公开的协议描述写成，不是从 mosh 的 GPL-3.0 C++ 派生的。 加密部分用 RFC 7253 自带的测试向量验证过。

</details>

<details>
<summary>mosh 能和 herdr 一起用吗？</summary>

能，而且会漫游。但 herdr 跑的是它自己在 mosh shell 里的 TUI， 原生的 workspace 和 tab sheet 由另一条 SSH 连接轮询 `herdr api snapshot` 来喂——有变化时 2 秒一轮，树不动了之后退到 8 秒一轮。 herdr 的 CLI 没有开放事件订阅，所以这个轮询就是 sheet 和 agent 状态能有多新的天花板。 另外 herdr 0.7.3 的 pane 对象不带命令名，所以窗格面包屑显示的是 `pane 1` 这种兜底文案，0.8.0 才补上这个字段。

想要 herdr 被原生渲染，那就走 SSH。走之前先知道一件事：herdr 的直连附着是**按窗格独占**的， 第二个客户端连上同一个窗格会把第一个踢掉。两台 Moshpit 盯同一个窗格就会来回抢， 直到其中一边退让——Moshpit 的退让规则是 30 秒内被踢三次就先停一会儿， 并显示<i>「Another client is using this pane — retrying shortly.」</i> herdr 自己的 TUI 不占这个附着位，所以你笔记本上开着的普通 `herdr` 不算竞争者。

</details>

<details>
<summary>为什么我敲的东西过了几分钟才出现在一个旧会话里？</summary>

现在不该再有了。没被 ack 的击键 10 秒后就过期，不再永远重传， 为的正是不让一条死了很久又回来的链路，把一堆陈旧输入回放进你的 shell。 如果你还遇到，那是值得报上来的 bug。

</details>
