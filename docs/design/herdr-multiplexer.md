# Design: herdr 作为 tmux 之外的第二种多路复用器

状态:**全部四期已实现并实机验证** · 2026-08-06
调研基准:herdr 源码 **main / v0.8.0**,`PROTOCOL_VERSION = 19`,<https://github.com/herdrdev/herdr>
(下文引用的 `src/...` 行号均指该版本)

> 🐛 **版本漂移已经咬过一次(2026-08-07 修复)。** 用户实测:主机装了 herdr 但**服务器没在跑**时,
> App 一片黑屏 —— 没有空状态、没有报错、Windows sheet 里点「+」也毫无反应。
> 根因:我按 0.8.0 源码写的检测匹配的是结构化的 `{"error":{"code":"server_not_running"}}`,
> 而 **0.7.3 吐的是裸 Rust 错误** `Error: Os { code: 2, kind: NotFound, … }`。文本对不上 →
> `serverNotRunning` 一直是 false → 空状态永远不显示。而且 `executeCommand` 会把 stderr 合进
> stdout **并丢掉退出码**,所以连"命令失败了"都看不出来。
> 修法:不再匹配任何错误文案 —— 每条命令后面跟一个 `; echo "__moshpit_rc=$?"`,
> 用**退出码**判定(两个版本都是 1)。教训:跨版本判定要挑最稳定的信号,错误字符串是最不稳定的那个。
>
> ⚠️ **版本漂移是真的,不是理论风险。** 实机验证时 `brew install herdr` 装到的稳定版是
> **0.7.3,协议 16**;而上面读的 main 是 0.8.0 / 协议 19 —— 相差 3 个协议版本。Phase 0
> 只依赖 `herdr` 这一个命令,不受影响;但 Phase 1/2 依赖的 `api snapshot` 与
> `terminal session control` 必须按"字段缺失即降级"写,并在连上后核对 snapshot 里的
> `protocol` 字段。0.7.3 上 `herdr api snapshot` 已存在且返回同样的信封结构
> (`{"id":…,"result":{"snapshot":{…}}}`) —— 注意**是信封,不是裸 snapshot**。

## 为什么值得做

herdr 是专门给 CLI coding agent 写的多路复用器 —— 和 Moshpit 的用户是同一批人。
它不是"tmux 的另一种皮肤",而是把 Moshpit 现在自己扛的三件苦活做进了服务端:

| Moshpit 现状 | herdr 提供的 |
|---|---|
| Agent 状态要用户在主机上装 hooks,往 tmux option 写 `@moshpit_*`(`IslandHooksInstallView`) | `agent_status` 是协议一等字段(`idle/working/blocked/done/unknown`),pane→tab→workspace 自动上卷,**零安装** |
| 滚动要靠 `#{mouse_any_flag}` 自己判"这个 pane 是鼠标应用还是 shell",分别发滚轮序列或 copy-mode(`project_scroll_architecture`) | `terminal.scroll` 是协议动作,服务端 `runtime.wheel_routing()` 自己决定转成 mouse report 还是走宿主 scrollback(`src/server/headless.rs:340`) |
| 手机小屏会把 tmux window 拉小,所以要 pin 窗口尺寸、和"最小客户端"规则搏斗(`syncMoshWindow` / `repinActiveWindow`) | 直连客户端的 resize 只作用于**它自己那个 pane 的 PTY**(`src/server/headless.rs:2990` → `runtime.resize`),不影响别的客户端 |

另外:单个 Rust 二进制,`brew install herdr` 或 `curl -fsSL https://herdr.dev/install.sh | sh`,比"tmux + 一堆 hooks 脚本"好装。

代价是 herdr 很新、协议还在动(protocol 19 已经迭代了很多轮),而且**不在任何 Linux 发行版仓库里**(`packaging/` 只有 windows)。所以定位是 **tmux 的可选替代,不是替换**:每个连接自己选。

---

## 1. 概念映射

herdr 有两层"会话":

* **命名 session** —— 独立的服务器进程 + 独立 socket(`~/.config/herdr/sessions/<name>/herdr.sock`,默认 `~/.config/herdr/herdr.sock`),靠 `--session <name>` / `HERDR_SESSION` 选择。更像"另开一个 tmux server",不是 tmux session。
* **workspace / tab / pane** —— 服务器内部的三层树。

对到 Moshpit 现有模型:

| Moshpit(`TmuxTypes.swift`) | tmux | herdr | 说明 |
|---|---|---|---|
| `SessionInfo` | session `$0` | **workspace** `w1` | 带 `label` / `agent_status` / `pane_count` / `tab_count` / `worktree` |
| `WindowInfo` | window `@0` | **tab** `t1` | 带 `label` / `number` / `agent_status` / `pane_count`;没有 tmux 那种 layout 字符串,布局单独走 `layout.export` |
| `PaneInfo` | pane `%0` | **pane** `w1:p1` | 带 `terminal_id` / `agent` / `agent_status` / `cwd` / `title` / `revision` |
| — | — | 命名 session | **第一版只用默认 session**,不进 UI |

三层能对上,所以 `TmuxSnapshot` 这个值类型和现有的 Windows/Sessions/Panes sheet 可以原样复用,只需要换个协议名 + 一层翻译。

> 注意 herdr 没有 tmux 的 layout 串。`WindowInfo.layout` 在 herdr 模式下留空,`TmuxLayoutParser` 不参与;分屏比例要拿 `pane.layout` / `layout.export`(结构化 `LayoutNode`),Phase 2 再说。

---

## 2. 协议面:两条流

herdr 的 socket 是 **newline-delimited JSON**(Unix domain socket)。iOS 侧不可能直连远端 unix socket,所以两条流都通过在远端跑 `herdr` CLI 转成 stdio 来接。

### 2.1 控制面 —— `herdr api snapshot`

一次调用吐一整份状态(`session.snapshot`):

```
workspaces[]  WorkspaceInfo   workspace_id, label, number, active_tab_id, focused,
                              agent_status, tab_count, pane_count, worktree, tokens
tabs[]        TabInfo         tab_id, workspace_id, label, number, focused,
                              agent_status, pane_count
panes[]       PaneInfo        pane_id, tab_id, workspace_id, terminal_id, label, title,
                              terminal_title, agent, agent_status, cwd, focused, revision
agents[]      AgentInfo       上面那些 + interactive_ready / launch_pending / state_change_seq
layouts[]     LayoutDescription
focused_workspace_id / focused_tab_id / focused_pane_id
protocol / version
```

一发 JSON 就够画整棵树 + 每个节点的 agent 状态,比 tmux 那边 `list-sessions` / `list-windows -a` / `list-panes` 三条命令再拼要干净。

**没有推送**:socket 有 `events.subscribe`(26 种事件,含 `pane_agent_status_changed` / `pane_output_changed` / `tab_created` …),但 CLI **没有暴露** subscribe 子命令,只有 `api snapshot` / `api schema`。所以第一版控制面是**轮询**(1–2s)+ 每次写操作后立即刷新。等 herdr 开出 `herdr events subscribe`,或者我们愿意走 `ssh -W`/`socat` 直连 unix socket,再换推流。

写操作全部是独立 CLI 调用(每次一个 exec):

| Moshpit 动作 | herdr 命令 |
|---|---|
| 新建 session | `herdr workspace create [--label X] [--cwd P]` |
| 重命名 / 关闭 session | `herdr workspace rename <id> <name>` / `workspace close <id>` |
| 新建 window | `herdr tab create [--label X] [--workspace <id>]` |
| 重命名 / 关闭 window | `herdr tab rename` / `tab close` |
| 切换 | `herdr workspace focus` / `tab focus` / `pane focus` |
| 新建 pane | `herdr pane split <target> --direction right\|down` |
| 关闭 pane | `herdr pane close <id>` |
| 缩放 | `herdr pane zoom <id>` |

服务器没起来时,CLI 返回结构化错误 `{"code":"server_not_running","message":"no herdr server is running at …"}`(`src/cli/server_not_running.rs:29`)。这正好对应主页那张"没有会话"的空状态卡 —— 不要当连接失败处理。

### 2.2 帧面 —— `herdr terminal session control`

这是最关键的发现:herdr 有一条**机器可读的单窗格附着流**,相当于 tmux `-CC` 的对位物,而且更好用。

```
herdr terminal session control <target> --cols 52 --rows 30 [--takeover]
```

* `<target>` 传 snapshot 里的 `terminal_id` 最稳(pane id / agent 名也能解析,`src/server/headless.rs:1666`)。
* **stdout**:每帧一行 JSON

  ```json
  {"type":"terminal.frame","seq":42,"encoding":"ansi","width":52,"height":30,
   "full":false,"bytes":"<base64 ANSI>"}
  ```

  `bytes` 解 base64 后是**可以直接写进终端的 ANSI 字节**(`src/client/mod.rs:952`),即 SwiftTerm `feed(data:)` 的入参。`full=true` 表示整屏重绘而非增量差分 —— 收到时先 reset 再喂。结束时发 `{"type":"terminal.closed","reason":…}`。
* **stdin**:每行一条 JSON 指令(`src/client/mod.rs:989`),一共四种

  ```json
  {"type":"terminal.input","text":"ls\n"}                    // 或 "bytes": base64
  {"type":"terminal.resize","cols":52,"rows":30}
  {"type":"terminal.scroll","direction":"up","lines":3,"source":"wheel"}   // source: wheel | page_key
  {"type":"terminal.release"}                                 // detach
  ```

三条必须知道的语义:

1. **resize 只改这一个 pane**(`src/server/headless.rs:2972-2993`)。TerminalAttach 模式的客户端 resize 直接打到 `runtime.resize(rows, cols, …)`,不走 `resize_shared_runtime_to_effective_size()`。手机的 52×30 不会把用户笔记本上的布局压扁 —— tmux 那套 window pin 的活儿全部消失。
2. **附着是独占的**。`terminal_attach_owners` 里已有 owner 且不是自己时,不带 `--takeover` 会被直接拒(`src/server/headless.rs:2616`)。Moshpit 断线重连必然撞上自己的旧连接,所以**默认带 `--takeover`**;但这会不会踢掉用户笔记本上那条直连,要实机验证(普通 TUI 客户端不是 TerminalAttach 模式,理论上不冲突)。
3. `terminal session observe` 是只读版:同样吐帧,但**不 resize**、不接受输入。可以拿来做"窗格预览"(sheet 里的缩略),不抢 owner。

### 2.3 和 tmux `-CC` 的对照

| | tmux `-CC` | herdr |
|---|---|---|
| 通道数 | 1 条,全部 session 的 pane 都复用 `%output` | **每个 pane 一条** exec 通道 |
| 状态推送 | `%window-add` / `%layout-change` / `%session-changed` 实时 | 无 → 轮询 `api snapshot` |
| 输出格式 | 八进制转义的原始字节 | base64 ANSI,带 `seq` / `full` |
| 尺寸 | 客户端 attach 会拉动 window 尺寸,要 pin | 只影响自己的 pane |
| 滚动 | 客户端自己判 wheel vs copy-mode | 服务端决定 |
| Agent 状态 | 需要 hooks | 内建 |

结论:**herdr 的多窗格同屏比 tmux 弱(每 pane 一条 SSH channel),单窗格体验比 tmux 强**。手机上本来就是一次看一个 pane —— 正好。

---

## 3. 传输:怎么接进 Moshpit

### SSH(完整能力)

两条流各占一个 SSH channel:

* 帧面:`herdr terminal session control <terminal_id> --cols C --rows R --takeover`
* 控制面:周期性 `herdr api snapshot`(可复用现有的 `SSHSession.executeCommand` 一次性 exec)

**已知障碍**:帧面需要一条**能写 stdin 的长连 exec 通道**。`SSHService` 现在只有两种形态 —— `requestPTY` + 流式 stdin(交互 shell),和 `executeCommand`(一次性、缓冲到结束)。两条路:

* **方案 A(推荐)**:给 `SSHService` 加流式 exec(无 PTY,stdout 走 `AsyncStream<Data>`,stdin 可写)。要确认 Citadel 的 exec channel 支持写 stdin —— **这是本设计最大的未验证点**,先写个 20 行 spike 验掉。
* **方案 B(兜底)**:仍用 PTY,但启动行写 `stty raw -echo; exec herdr terminal session control …`。不这么做的话,行规程会回显我们发出去的 JSON,而且 canonical 模式 4096 字节上限会截断长 base64 输入行。能用,但脏。

### mosh(降级为 TUI)

mosh 传的是渲染完的屏幕差分,承载不了行分帧的控制协议 —— 这一点 tmux `-CC` 上已经踩过并写进了 `TmuxTransport.swift` 的注释,herdr 同理。所以 mosh 模式下:

* 终端里跑 herdr 自己的 TUI(`herdr`),Moshpit 只当渲染器,和现在 mosh+tmux 完全同构;
* 控制面另开一条轻量 SSH 连接跑 `api snapshot` 轮询,喂原生 sheet(和现在的 `moshControl` sidecar 同构)。

前缀键 herdr 默认也是 `ctrl+b`,现有的 prefix 处理和快捷键面板基本能复用。

### 降级矩阵(在 `SessionHub.Missing` 上扩)

| 用户选择 | 主机情况 | 结果 |
|---|---|---|
| herdr + SSH | 有 herdr | 原生控制面 + 帧面 |
| herdr + mosh | 有 herdr | mosh 跑 TUI + SSH 控制面 |
| herdr | 无 herdr,有 tmux | 横幅提示,**不自动改用 tmux**(会话内容完全不同,静默切换是骗人),给 Install Assist |
| herdr | 都没有 | 裸 shell + 横幅 |

---

## 4. 代码改造点

| 文件 | 改动 |
|---|---|
| `Models/ServerConnection.swift` | `useTmux: Bool` → `multiplexer: Multiplexer {none, tmux, herdr}`;`Codable` 迁移:老数据 `useTmux == true` → `.tmux`,`false` → `.none`。加 `herdrPath: String?`(对位现有 `tmuxPath`) |
| `Services/HostCapabilities.swift` | `probeCommand` 的 `command -v` 加 `herdr`;加 `hasHerdr`;`static let unknown` 保持乐观默认 |
| `UI/Terminal/InstallAssistView.swift` | 加 herdr 分支。**注意没有发行版包**:brew → `brew install herdr`,其余一律 `curl -fsSL https://herdr.dev/install.sh \| sh`,不要套 `PackageManager.installCommand` |
| `Services/Tmux/TmuxControlling.swift` | 改名 `MultiplexerControlling` 并挪到 `Services/`。方法名保留 App 术语(session/window/pane),herdr 侧翻译 |
| **新** `Services/Herdr/HerdrSnapshot.swift` | `session.snapshot` JSON → `TmuxSnapshot` + `[String: AgentHook]` 的纯函数解码器(**无 IO,单测直接喂 fixture**) |
| **新** `Services/Herdr/HerdrControlClient.swift` | 控制面:轮询 + 写操作命令构造 + shell 转义,conform `MultiplexerControlling` |
| **新** `Services/Herdr/HerdrFrameChannel.swift` | 帧面:NDJSON 分帧 → base64 解码 → `feed`;输入/resize/scroll 编码。同样把**分帧+编解码做成纯函数**给单测 |
| `Services/SessionHub.swift` | 连接分支按 `multiplexer` 走;herdr 分支不需要 window pin / copy-mode 那套 |
| `Views/Terminal/TerminalScrollGesture.swift` | herdr 模式下手势直接翻译成 `terminal.scroll`,跳过 `mouse_any_flag` 启发式 |
| `Island/AgentActivityMonitor.swift` | herdr 模式从 `agent_status` 直接构 `AgentHook`(`working/blocked/done` → 现有三态),`state_change_seq` 当变更序号 |
| `UI/Island/IslandHooksInstallView.swift` | herdr 连接下整页隐藏 —— 不需要装任何东西 |
| `UI/Tmux/TmuxSheets.swift` / `AttachHomeView.swift` | 泛型已经是 `<C: TmuxControlling>`,换协议名即可;文案里的 "tmux" 改成多路复用器中性词 |

### 4.1 用户在哪里选

**注意现在没有开关**:`AddConnectionView.save()` 直接写死 `connection.useTmux = true`(`AddConnectionView.swift:234`),ADVANCED 组里只有一个 "Custom tmux Path" 输入框。多路复用器选择器是全新 UI。

落点:Add / Edit Connection 表单,用现有 `Menu` + `MiniChevron` 行的样式(和 "Predict Mode" 同款,`AddConnectionView.swift:160`),放在 ADVANCED 组顶部:

```
ADVANCED
  Multiplexer            tmux  ⌄     ← Menu: None / tmux / herdr
  Custom tmux path       [____]      ← 跟着选择切换 placeholder 与绑定
  Compress Output        [ o]
```

规则:

* **每连接一份**,不做全局默认 —— 同一个人的不同主机装的东西不一样,全局默认只会制造"为什么这台连不上"。
* 路径框跟着选择走:`.tmux` 显示 "Custom tmux path"(绑 `tmuxPath`),`.herdr` 显示 "Custom herdr path"(绑 `herdrPath`),`.none` 隐藏。
* 选项行带副标题说清差别,别让用户猜:tmux = "成熟、到处都有";herdr = "为 coding agent 设计,agent 状态免装 hooks";None = "单窗格裸 shell"。
* 选了主机上没有的那个,**不静默改用另一个** —— 两边的会话内容毫不相干,静默切换等于骗人。走横幅 + Install Assist(见降级矩阵)。

**迁移**:`ServerConnection` 的 `Codable` 要能读老数据。老库里 `useTmux` 恒为 `true`,所以解码时缺 `multiplexer` 字段一律落到 `.tmux`,存量连接行为零变化。

---

## 5. 分期

**Phase 0 — 打通 TUI 模式** ✅ **已完成**(2026-08-06)
探测 + 连接设置里的多路复用器三选一 + SSH/mosh 都能启动 `herdr` TUI + Install Assist。

落地的东西:

| 文件 | 内容 |
|---|---|
| `Models/ServerConnection.swift` | `Multiplexer` 枚举 + `multiplexer` 计算属性(带 `useTmux` 迁移)+ `herdrPath` + `multiplexerPath` |
| `Services/HostCapabilities.swift` | 探 `herdr`、`hasHerdr`、`has(_:)`、手写 `init(from:)` 兼容旧缓存、`Multiplexer.installCommand(using:)` |
| `Services/Herdr/HerdrLaunch.swift` | 启动命令(PATH 前缀 / 自定义路径) |
| `Services/SessionHub.swift` | 三条路径(SSH / mosh / mosh→SSH 降级)按 multiplexer 分支;`DegradeNotice.herdr` |
| `UI/Home/AddConnectionView.swift` | ADVANCED 组里的选择器 + 跟随切换的路径框 |
| `UI/Terminal/InstallAssistView.swift` | herdr 独立安装流(brew / install.sh)+ `~/.local/bin` 说明 |
| `scripts/verify-herdr-launch.sh` | 实机端到端验证(SSH,`MOSAIC_MOSH=1` 走 mosh) |

验收结果:

* 单测 364 全过,新增 17 条 —— 老格式 JSON 迁移、herdr 探测、安装命令合成、启动命令。
* `scripts/verify-herdr-launch.sh` 干净重装后跑通:SSH 与 mosh 两条传输都起了 herdr TUI,
  `herdr api snapshot` 确认服务端 1 workspace / 1 tab / 1 pane。
* 降级链路实测(`brew unlink herdr` 制造缺失):横幅"herdr not found on this host —
  plain shell session." → Install Assist 显示 `brew install herdr` → `brew link` 后
  Re-check 翻成"Installed — reconnect to enable it."。**没有**静默改用 tmux。

实机观察:herdr 的侧边栏在手机上吃掉约 1/3 屏宽,可用但局促 —— 这正是 Phase 2 单窗格
帧通道的价值所在,不是"锦上添花"。

**Phase 1 — 控制面** ✅ **已完成**(2026-08-06)
`api snapshot` → `TmuxSnapshot`,现有三个 sheet + 主页会话树 + 面包屑在 herdr 下可用。

落地的东西:

| 文件 | 内容 |
|---|---|
| `Services/MultiplexerControlling.swift` | `TmuxControlling` 改名 + 移出 `Tmux/`,术语保持 session/window/pane |
| `Services/Herdr/HerdrSnapshot.swift` | 纯解码器:herdr JSON → `TmuxSnapshot` + `agentHooks`,全字段可选 |
| `Services/Herdr/HerdrControlClient.swift` | 2s 轮询 + 命令下发,`HerdrCommandRunner` 抽象让单测无需网络 |
| `Services/SessionHub.swift` | SSH 复用主连接、mosh 起轻量 sidecar(无 PTY)、resume 重建、stop 清理 |
| `UI/…` | 三个 sheet + 主页树按具体类型分支(保住 Observation 追踪) |

**0.7.3 实测挖出来的三件事**(设计稿基于 0.8.0 源码,这些是差异):

1. **`herdr pane focus` 只接受方向,不接受 pane id** —— 0.7.3 和 0.8.0 的 CLI 都是。socket API 里有 `pane.focus`,但 CLI 从没暴露。唯一能按 id 聚焦的是 `herdr agent focus <pane_id>`,它对普通 shell 窗格会报 `agent_not_found` **但焦点确实会移动**:服务端 `focus_agent_target` 先聚焦、再构造 agent 响应体才失败(`src/app/agents.rs`,两个版本同序)。所以错误是关于**返回体**而非动作,我们显式吞掉它。这是本期最脏的一处,注释写清楚了。
2. **0.7.3 的 pane 对象没有 `title`/`agent`/`label` 字段**(0.8.0 才有),所以 herdr 窗格的"命令"是空的。面包屑因此加了兜底:命令为空时显示 `pane N` —— 否则窗格面包屑整个消失,而它是进 Select Pane sheet 的唯一入口。
3. **窗格尺寸和 zoom 不在 pane 对象上**,在 `layouts[]` 里(每个 tab 一条,带每个窗格的 rect)。

验收结果:

* 单测 389 全过,新增 25 条 —— 解码器吃**真实 0.7.3 抓包**(`MoshpitTests/Fixtures/herdr-snapshot.json`)、版本容差、shell 注入引号、轮询失败保留旧树。
* 实机双向验证:App 里点 Sessions sheet 的 workspace 行 → 服务端 `focused_workspace_id` 从 w1 变 w2;反过来在终端 `herdr workspace rename` → App 面包屑在一个轮询周期内跟上。
* `scripts/verify-herdr-launch.sh` 现在同时断言"服务端活着"和"App 读到了树"。

遗留:herdr 的**空状态**还没做。服务端没起时轮询会拿到 `server_not_running`,客户端已经把它单独记为 `serverNotRunning` 并清空树,但主页还没有对应的"herdr 没有会话,点这里新建"卡片 —— 现在会走 `isLiveWithoutTree` 显示"打开终端"。Phase 2 顺手补。

**Phase 2 — 帧面原生渲染** ✅ **已完成**(2026-08-06,仅 SSH)
单窗格 `terminal session control` 接 SwiftTerm,输入/resize/scroll 全走协议。

**Citadel spike 的结论:方案 A 不可行,走方案 B。** Citadel 0.7 公开的 API 里没有"exec 指定命令 + 可写 stdin":`executeCommand`/`executeCommandStream` 不给 stdin,`withPTY`/`withTTY` 给 `TTYStdinWriter` 但只能跑默认 shell(内部的 `.command(String)` 模式是 internal)。所以帧通道跑在 PTY 登录 shell 里,靠 `stty raw -echo` 压掉行规程 —— 本机 PTY 实测:开了之后我们发的 JSON **零回显**,resize 生效,seq 单调。

| 文件 | 内容 |
|---|---|
| `Services/Herdr/HerdrFrameChannel.swift` | 增量 NDJSON 分帧器 + 四条指令编码(input/resize/scroll/release)+ 启动行 |
| `Services/SessionHub.swift` | SSH+herdr 改为跑帧通道而非 TUI;retarget、写序列化、teardown 归还 attach |
| `UI/Terminal/TerminalScreen.swift` | 空状态泛化成 `MultiplexerEmptyStateView`,herdr 的 `server_not_running` 用同一套 |

**实测挖出来的四件事:**

1. **帧是逐格绝对定位的重绘**(`ESC[3;6H` 一段一段),不是追加式字节流。喂给 SwiftTerm 完全正确,但**本地攒不出有意义的 scrollback** —— 历史只能走 `terminal.scroll` 由服务端翻页。印证了未决项 6。
2. **retarget 不用重建 SSH 通道**:同一个 shell 里 release → 等 ~400ms → 再起一条即可。中间 shell 会回显命令、macOS 还会插一句 "The default interactive shell is now zsh." —— 分帧器全部跳过,而且不需要 reset(截断行解析失败被丢弃,新 target 又是整屏重绘)。
3. **`terminal.closed` 会因为我们自己的 release 而触发**,且在新 target 设好之后才到。第一版无条件把 `herdrFrameTarget` 置 nil,结果 `sendInput` 掉回裸字节分支 —— **画面照常刷新、打字全部丢失**。这种"看起来一切正常"的 bug 单测抓不到,是实机切窗格时发现的;现在用 `herdrExpectedCloses` 计数区分,验证脚本也加了专门的 retarget 回归检查。
4. **冷主机的引导变了**:Phase 0 跑裸 `herdr` 会顺带把服务器拉起来,帧通道没有这个副作用。改成 `nohup herdr server &`(实测 ~500ms 起来,**自带一个 workspace/tab/pane**),且**只在用户显式点"创建"时执行** —— 反而更贴合"绝不替用户建会话"的原则。

**mosh 仍跑 herdr 自己的 TUI**,理由和 tmux `-CC` 一样:mosh 传渲染后的屏幕差分,承载不了行分帧。

验收结果:
* 单测 404 全过,新增 15 条 —— 半行/粘包/截断恢复/shell 噪音/缓冲上限/四条指令编码。
* `scripts/verify-herdr-launch.sh` 现在断言四件事:服务端活着、App 读到树、**窗格是整屏宽(没有侧边栏)**、**打字到达窗格**、**切窗格后打字仍然到达**。

**Phase 3 — 灵动岛直连** ✅ **已完成**(2026-08-06)
`agent_status` 喂 Live Activity,herdr 连接不再显示 hooks 安装页。

| 文件 | 内容 |
|---|---|
| `Services/MultiplexerControlling.swift` | 协议加 `onAgentHooksUpdated` + `pollAgentHooks()` |
| `Island/AgentActivityMonitor.swift` | `ConnRef.controller` 改存协议存在类型;`track` 泛型化 |
| `UI/Settings/SettingsScreen.swift` | herdr 连接下隐藏「Install agent hooks」入口 |

**设计决定:tmux 的输出/响铃启发式不给 herdr 用。** 那套东西存在的意义是"没装 hook 时猜 agent 在干嘛";herdr 自己报 `agent_status`,再叠一个猜测只会变成和真相竞争的次等意见。所以 `track` 只在 `as? TmuxSessionController` 成立时才接 `onPaneActivity`/`onPaneBell`。

实机验证(`herdr pane report-agent`,即 herdr 自家集成用的同一个接口):

* `--state blocked` → 灵动岛琥珀色圆点 + 感叹号(needs attention)
* `--state working` → 青色圆点 + 实时计时器
* 面包屑窗格段直接显示 `Claude Code`

全程**主机上零安装**。对照 tmux:同样的效果要用户先跑 hooks 安装页、往 `~/.claude/settings.json` 塞 hook、再让它写 `@moshpit_*` tmux option。

**已知边界**:App 进后台后 iOS 会挂起它的 SSH 轮询,所以后台期间状态不会再更新(实测:后台时把 blocked 改成 working,岛上不动;回前台一轮轮询后才翻)。这不是 herdr 的问题,是"不用远程推送"这个产品决定的必然结果,tmux 路径同理。

---

## 6. 未决 / 需实机验证

1. ~~**Citadel 能不能在 exec channel 上写 stdin**~~ —— 已验:**不能**,公开 API 里没有。走方案 B(PTY + `stty raw -echo`),见 Phase 2。
2. ~~**`--takeover` 会不会踢掉别的客户端**~~ —— **会,而且咬了一次(2026-08-07 修复)**。两个 Moshpit 连同一台主机时,后连的那个带 `--takeover` 把先连的帧通道**直接踢掉**,被踢的一方 `terminal.closed` → 目标清空 → 而 retarget 只在"焦点变化"时触发,焦点根本没变,于是**永远黑屏不恢复**。
   修法:控制面每次轮询都重报当前焦点窗格(不再只在变化时报),retarget 里的相等判断让重复调用零成本,被踢后 ≤2s 自动重连。实测:小偷抢走 → 1 秒后 App 抢回来 → 打字正常。
   代价是两个直连客户端会**互相抢**(各自 2s 重连一次),这是 herdr 独占式直连的固有行为,不是能在客户端消掉的。TUI 客户端不占 `terminal_attach_owners`,所以"笔记本开 herdr TUI + 手机连同一台"仍然安全 —— 这一条还是没验到。
3. **带宽/帧率**:增量 ANSI 帧走 SSH,`claude --dangerously-skip-permissions` 那种狂刷输出的场景,和 tmux `-CC` 比是多是少,**还没量**。逐格定位的重绘可能比 tmux 的 `%output` 更费字节。
4. ~~**`herdr` 会自动建 workspace**~~ —— 更正:那是**从 `~/.config/herdr/session.json` 恢复**,不是新建。真正干净的 `herdr server` 起来是**零 workspace**。所以冷主机引导必须"起服务器 + 建 workspace"两步(`bootstrapServer`),验证脚本的前置也踩过同一个坑。原始记录如下:Moshpit 现在的原则是"绝不替用户创建会话"(见 `attachMoshTmux` 注释)。Phase 0 接受了这个差异(终端里就是 herdr 自己的 UI,用户看到的和自己敲 `herdr` 一模一样,`HerdrLaunch` 的注释记了这一点);Phase 1 有了控制面之后,应改成先 `api snapshot` 探测、没有就显示空状态让用户显式点"新建",与 tmux 对齐。
5. **协议漂移** —— 已证实(见开头的版本警告)。解码器按"字段缺失即降级"写,并在 `protocol` 超出已知范围时挂横幅,而不是崩。另外别忘了响应是**信封**:`result.snapshot`,不是裸对象。
6. **没有全量 scrollback API**:`pane.read` 的 source 只有 `visible / recent / recent_unwrapped / detection`,没有"把整个回滚缓冲导出来"。所以 herdr 模式下的历史浏览必须走 `terminal.scroll` 的服务端翻页,现有 `TerminalScrollback` 那条本地路径不适用。
7. **命名 session** 暂不进 UI。如果用户在主机上用 `--session` 分了多个服务器,Moshpit 第一版只看得见默认那个 —— 要在设置里写清楚。
