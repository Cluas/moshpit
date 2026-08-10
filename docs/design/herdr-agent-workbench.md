# Design: herdr agent 工作台

状态:**W1 + W2 + W3 + W4(复查修复)已实现** · 2026-08-07
取代:`herdr-agent-tasks.md`(那份只写了"创建"这一半,现在和"浏览"合并)
前置:`herdr-multiplexer.md` 四期已上线;L1(术语与键位分流)已实现
验证基准:herdr **0.7.3**(brew stable,protocol 16),下文 CLI 行为都在真机跑过

## 问题

herdr 接进来之后,Moshpit 在它上面**能做的事**已经不少:原生单窗格渲染、切换、增删、agent 状态进灵动岛。但**界面问的还是 tmux 的问题**:

> 哪个 session?哪个 window?哪个 pane?

而用户真正想问的是:

> **哪个 agent 在等我?**

这个问题灵动岛已经答得很好 —— 但**只在锁屏上**。解锁进 App,反而退回四层树的巡视模式:主机 → 会话 → 窗口 → 窗格。

L1 已经把词和键位按多路复用器分流了(workspace/tab、`⌃ b w`/`⌃ b n/p`),那是**正确性**修复。这份文档谈的是下一层:**信息架构本身**。

## 为什么 herdr 才让这件事成立

tmux 里"哪个 agent 在等我"是**推断**出来的:用户得先装 Moshpit 的私有 hook,hook 往 `@moshpit_*` tmux option 里写状态,我们再 `list-panes` 刮出来。没装 hook 的窗格只能靠输出启发式猜。**数据不齐的 agent 界面比没有更糟** —— 它会漏掉一半 agent,而用户不知道漏了。

herdr 里这是**查询**:`agent_status` 是协议字段,自动从 pane 上卷到 tab 和 workspace,`agent list` 按 agent 名寻址。装 `herdr integration install claude` 会更权威(agent 主动上报,而非屏幕匹配),但**不装也有值**。

所以:**agent-first 是产品方向,但先只在 herdr 上做**。等它在 herdr 上被验证成立,再回头决定要不要给 tmux 补齐。

## 一、浏览:Agents 区

herdr 连接的主页,在会话树**之上**加一个 Agents 区:

```
┌ AGENTS ─────────────────── 1 blocked ─┐
│ ● Claude Code        needs you  2:14  │  ← 琥珀点,排最前
│   moshi · fix-scroll-jump             │
│ ◐ Codex                working  0:41  │  ← 青点
│   rugisland · main                    │
│ ○ Claude Code             idle  —     │
│   moshi · 1:1                         │
└───────────────────────────────────────┘
┌ WORKSPACES ───────────────────────────┐
│ … 现有的树,原样保留 …                  │
└───────────────────────────────────────┘
```

* **排序**:blocked → working → done → idle。同级按状态持续时长倒序(等最久的在前)。
* **一行的信息**:agent 名(herdr 的 `agent` 字段)、状态、已持续多久、**在哪**(workspace label · tab label)。
* **点一下 = 跳到那个窗格**:`agent focus <pane_id>` → 焦点变 → 帧通道自动 retarget(Phase 2 的能力,不用新写)。
* **没有 agent 时整个区不出现** —— 不留一个空壳占位。

数据全部来自现有的 `HerdrControlClient.snapshot` + `agentHooks`,**不需要新的远端调用**。

### 边界:只看得见 herdr 托管的 agent

**Agents 区列的是 herdr 自己窗格里的 agent,不是这台机器上所有的 agent。** 这一条必须说在前面,
否则用户会拿"我明明在跑 claude"来对照一个空列表 —— 实测踩过:

```
主机进程:  claude … server:gitlab    cwd=~/code/rugisland   ← 在 Terminal 里起的
herdr 窗格: 5 个,全部停在 ~ 空转                              ← 它只认识这些
结果:      Agents 区显示"Nothing running",而用户确信有东西在跑
```

herdr 靠观察**自己窗格的屏幕内容**识别 agent(装了 `herdr integration install claude`
则由 agent 主动上报,更准,但仍然只覆盖它托管的窗格)。所以:

* 想让某个 agent 出现在列表里,要么用 **New agent task** 起它,要么手动在 herdr 窗格里跑
* 这不是缺陷,是 herdr 的模型 —— 它是 runtime,不是进程扫描器
* **UI 上要说人话**:空态文案得暗示"在这里起一个",而不是让用户以为 App 漏读了

### 和灵动岛的关系

同一份数据,两个出口。灵动岛答"现在最紧急的是什么"(单条,锁屏),Agents 区答"全都在什么状态"(列表,App 内)。`AgentActivityMonitor` 已经在消费 `agentHooks`,这里只是把同一份状态换个呈现。

## 二、创建:New agent task

Agents 区右上角的 `+`:

```
┌ NEW AGENT TASK ─────────────────┐
│ Repo      ~/code/moshi       ⌄  │
│ Branch    fix-scroll-jump       │
│ Agent     Claude Code        ⌄  │
│ Prompt    (可选,多行)           │
├─────────────────────────────────┤
│            [ Start ]            │
└─────────────────────────────────┘
```

一条命令就能拿到隔离环境(真实抓包,已清理):

```sh
herdr worktree create --cwd <repo> --branch <name> --label <text> --focus --json
```
```json
{"type":"worktree_created",
 "workspace":{"workspace_id":"w4","label":"probe task","focused":true},
 "root_pane":{"pane_id":"w4:p1","cwd":"/Users/cluas/.herdr/worktrees/repo/moshpit-probe"}}
```

**新 git worktree + 新 workspace + cwd 已在里面的窗格 + 聚焦过去。** 这是 tmux 给不了的,也是"手机当 agent 遥控器"和"手机上看终端"的分界线。

然后在那个窗格里起 agent:

```sh
herdr pane run <root_pane> "claude"     # 命令 + 回车
herdr agent send <target> <prompt>       # 可选的初始 prompt,写字面文本
```

### 为什么不用 `agent start`

实测:`agent start --workspace <ws>` **不继承 worktree 的目录**,新窗格的 cwd 是发起 CLI 的那个目录:

```
w4:p1  cwd=~/.herdr/worktrees/repo/moshpit-probe   ← worktree create 建的
w4:p2  cwd=/Users/cluas/code/moshi                  ← agent start 建的,错的
```

要用它就必须显式再传 `--cwd`。而 `worktree create` 已经给了一个 cwd 正确的窗格,直接 `pane run` 更简单、少一个失败点、用户看到的就是"命令被敲进去了"。代价是 herdr 只能靠屏幕检测认出这是 agent —— 想要权威状态就装 `herdr integration install claude`,和这个选择无关。

### Repo 从哪来

**本设计唯一真正的空白** —— herdr 不管"你有哪些仓库"。方案:

1. **主**:从 `api snapshot` 里所有窗格的 `cwd` 去重,每个用 `git -C <cwd> rev-parse --show-toplevel` 解析成仓库根。零配置,覆盖"最近开过的"。
2. **兜底**:下拉底部一个「Other…」,手输路径并记进 `ServerConnection`。

## 三、集成契约:怎么做到"无缝"

无缝不是靠画得像,是靠**不新增任何东西**。三条硬约束:

### 1. 只重排现有构件,不发明新的

| 元素 | 直接复用 | 拿到什么 |
|---|---|---|
| agent 一行 | `SheetListRow(icon:name:meta:isActive:statusColor:)` | 48pt 行高、`Face.text(14,.semibold)` 名字、`Face.mono(11)` meta、状态点、选中态 —— 和 Windows/Workspaces sheet 逐像素一致 |
| 区标题 + 计数 | 主页树同款 `Face.mono(10,.bold)` + kerning 1.7 | 和正下方的 WORKSPACES 区同一视觉层级 |
| 任务表单 | `FormGroup` + `FieldRow` + `Menu` 行 | 和 AddConnectionView 的 Multiplexer 选择器同一套语法 |
| 错误 | 已有的通知胶囊(`herdrNotice`) | 不需要新的错误 UI |
| 触感 | `Haptics.select()` | 和切窗格同一个反馈 |

`SheetListRow` 的注释里本来就写着 "Agent activity dot (Vibe Island colours)" —— **这套行原本就是为 agent 状态设计的**,Agents 区只是把它从 sheet 搬到主页。

### 2. 不新增导航轴

现在只有两层:主页 → 终端。Agents 区**长在连接卡内部**,就在树的正上方,同卡同滚动。点一行走的是树已经在走的那条路(`Haptics.select()` → `selectPane` → `onEnter()`),落到同一个终端页。

**明确不做**:独立 Agents 页 / 底部 tab 栏 —— 那会新增一条这个 App 从来没有的导航轴。

**没有 agent 时整区不渲染**,所以没跑 agent 的 herdr 用户看到的就是今天的界面。这是"增量"和"模式"的分界线。

### 3. 状态语义只有一个来源

实现前 `agentDot()` 是 `TmuxSheets` 里的 private 函数,而 Agents 区和灵动岛要用同一套颜色。照抄一份就是分叉的起点。

所以抽出了 **`UI/AgentSignal.swift`**:`init?(_ state:)` / `color` / `rank` / `label` / `aggregate(_:)`,sheet 的点、Agents 区的点与排序共用它。
`aggregate` 刻意**忽略 `.done`** —— 已完成的 agent 值得在列表里占一行("去看看"),但让整个窗口点亮会改变 tmux sheet 一直以来的表现,那就不叫无缝了。

### 顺带修掉的两处 tmux 漏网

L1 改了三个 sheet,但主页还有两处写死:

* `AttachHomeView` 树的区标题写死 `SESSIONS` → 改走 vocabulary
* 顶部统计芯片写死 `TMUX`,而且**只数 tmux**(herdr workspace 活着也报 0)→ 改成两个控制面都数,标签跟着实际连接走,混合主机回落到中性词

## 四、代码改造点

| 文件 | 改动 |
|---|---|
| **新** `UI/AgentSignal.swift` ✅ | 状态语义单一来源:`color` / `rank` / `label` / `aggregate` |
| `UI/Home/AttachHomeView.swift` ✅ | `agentsSection` + 纯函数 `agentEntries(snapshot:hooks:)`;区标题与统计芯片改走 vocabulary |
| **新** `UI/Herdr/NewAgentTaskSheet.swift` | 表单,沿用 `FormGroup`/`FieldRow`/`Menu` |
| **新** `Models/AgentTaskRequest.swift` | 表单值类型 + 校验(分支名、repo 非空),纯函数好测 |
| `Services/Herdr/HerdrControlClient.swift` | `startAgentTask(...)` 串起 worktree→run→send;`gitRoots()` 解析仓库根;失败要**返回错误文本** |


**错误显示**复用 Phase 2 加的通知胶囊(`ActiveSession.herdrNotice`)。`worktree create` 失败时 herdr 返回结构化错误,把 `error.message` 直接显示 —— 分支重名这种事用户一眼能懂。

## 五、分期

**W1 · Agents 区(只读)** ✅ **已完成**(2026-08-07)
列表 + 排序 + 点击跳转,零新增远端调用。

落地:`UI/AgentSignal.swift`(共享状态语义)、`ConnectionCard.agentsSection` + 纯函数
`agentEntries(snapshot:hooks:)`、`TmuxSheets.agentDot` 改用共享映射、两处 tmux 漏网修掉。

验收:单测 428 全过(新增 10 条:排序、静默态不出现、位置文案、名字回退、失效 hook、
poll 间顺序稳定,以及 `AgentSignal` 的映射/排序/聚合/配色);
实机截图确认 `AGENTS 2 · NEEDS YOU`、attention 排第一、顶部芯片从 `TMUX 0` 变 `WORKSPACES 2`。

**W1 没做的两件事**(有意):
* **不显示已持续时长** —— herdr 的快照不带状态变更时间戳,只能从"App 第一次看到"推,而轮询空闲时是 8s 粒度。要做得放到 W2,用灵动岛已有的 `stateSince` 当来源。
* **仅 herdr** —— tmux 的 agent 数据取决于用户装没装 hook。

**W2 · New agent task** ✅ **已完成**(2026-08-07)
表单 + worktree + 起 agent + 可选首条 prompt。

落地:`Models/AgentTaskRequest.swift`(值类型 + 分支名校验)、
`UI/Herdr/NewAgentTaskSheet.swift`(FormGroup/FieldRow/Menu,和 Add Connection 同语法)、
`HerdrControlClient` 的 `gitRoots()` / `agentNames()` / `startAgentTask(_:)`、
解码器补带 `paneCwds`。

**实机整条链**(演示产物已清理):
1. 点 AGENTS 区的 `+` → 表单里 Repo **自动填好** `moshpit-w2-demo`(从窗格 cwd 反查 git 根)、Agent 默认 `claude`
2. 填分支 `fix-scroll-jump` → Start
3. 服务端出现 workspace `w5`,窗格 cwd 是 `~/.herdr/worktrees/moshpit-w2-demo/fix-scroll-jump`,`git worktree list` 确认真的建了分支
4. 窗格里 **claude 真的起来了**(停在它的信任目录提示)
5. herdr 把它标成 `agent: claude / status: blocked`
6. 主页 Agents 区随即显示 `AGENTS 1 · NEEDS YOU` —— **W1 和 W2 咬合上了**

**过程中修掉两个自己踩的坑:**
* **鸡生蛋**:`+` 在 AGENTS 标题栏里,而该区原本"没 agent 就不渲染" → 永远建不了第一个任务。改成会话连上就显示标题栏,空时给一行说明。
* **默认 agent 是字母序第一个**(实测填进去的是 `agy`)。改成优先 `claude` / `codex`,再回落。

验收:单测 434 全过(新增分支名校验的接受/拒绝各一组、命令链、校验短路不发请求、herdr 错误原文透传、`gitRoots` 单命令去重、manifest 解析)。

**W3 · 收尾** ✅ **已完成**(2026-08-07)
长按 worktree 会话 → Remove Worktree,只在这类会话上出现(`worktree` 字段判定,0.7.3 就带)。

**两步确认,而且第一步绝不 `--force`:**

```
1) worktree remove --workspace <id>          ← 实测:dirty 时返回
   → dirty_worktree_requires_force              且文件完好无损
2) 只有此时才弹第二次确认(「那些改动不存在于任何别处」)
   worktree remove --workspace <id> --force  ← 用户明确点了才加
```

这是 herdr 自己的安全网,不是我们发明的 —— 顺着它比替它做决定好。

验收:单测 438 全过(worktree 判定、dirty → `needsForce` 且命令里没有 `--force`、
强制路径、错误原文透传);命令层实测两步行为。
**未验到**:长按菜单本身 —— idb 驱动不了 SwiftUI 的 contextMenu 长按(长按与原地滑动都试过),
所以菜单→对话框→对话框这段交互只有编译和单测保证,没有设备录像。

**W4 · 复查修复**(2026-08-07 晚)——整轮 UI/UX 复查(截图逐屏 + 代码对照)揪出的问题,按严重级修完:

1. **P0 · 锁屏 Allow/Deny/Reply 在 herdr 上打错窗格。** tmux 分支按 pane 定向
   (`send-keys -t`),herdr 落到 `sendInput` → 发进**当前帧通道目标**,而不是通知来源的窗格
   —— 两个 agent 并发时(cycler 专门支持的场景),锁屏上的 Allow 可能是别人的 Enter。
   修法:`ActiveSession.deliverInput(_:toPane:)` —— 先 `selectPane`(=`agent focus`),
   等 `herdrFrameTarget` 翻到目标**并等 retarget task 收尾**(release→start 的 400ms 间隙里
   入链的输入会掉进裸 shell),再写帧输入。mosh+herdr(TUI 模式)等 `activePaneId` 翻转后发普通键。
   聚焦不到(争抢退避、poller 死了)返回 false,通知层回落到"打开 App"。
2. **状态语义色跟着主题走,且两个表面已经分叉。** `AgentSignal.working` 用的是
   `Ink.accent`(用户可自定义),灵动岛却硬编码 teal —— 同一个 working,锁屏青色、App 内随主题。
   收进 **`Island/AgentPalette.swift`**(两个 target 共同编译,project.yml 里显式挂进 island),
   三个状态色固定;测试钉死 `AgentPalette.attention == Ink.warn`、全部 ≠ `Ink.accent`。
3. **L2 · agent 面包屑。** 第三段在窗格有 hook 时显示 **agent 名 + 状态点**,needs-you 整胶囊转琥珀;
   宽度问题(原型页草图发现的"三段放不下 402pt")用 **session 段收成 icon-only** 解——仍可点,
   不再花 ~90pt 念 sheet 里也会重复的名字。规则在纯函数 `BreadcrumbPlan.make` 里,8 条单测。
   tmux 装了 hook 同样受益,没 hook 两条路都维持原样(命令 → `pane N` 兜底)。
4. **树不可辨识。** 四个 workspace 全叫 `● ~ 1 tabs`。重名 session 行尾带 id
   (`~ · w4`,herdr CLI 寻址用的同一个词);tab 名等于序号时(`1: 1`)改读 `Tab 1`
   (`WindowInfo.displayTitle(vocab)`,主页树 + 两个 sheet + kill 确认框同一来源)。
5. **Agents 行补"已持续多久"。** W1 欠的账:行尾分钟粒度时长(`2m` / `1h 12m`,<1m 显示 `now`),
   来源是灵动岛已有的 `stateSince`(`AgentActivityMonitor.stateSinceByPane`),tmux hook 自带
   `@moshpit_since` 时以 hook 为准 —— 行和锁屏永远同一只钟。不显示秒,轮询粒度撑不起那个精度。
6. **具名 idle agent 不再隐形。** 解码器把 `agent_status: idle` 按原词透传(0.7.3 无 agent 字段
   → 自然不产生行);Agents 区把**有名字的** idle 显示为无光点暗行、排最后、行尾标 `idle` ——
   一个停在提示符等活的 claude,和"Nothing running"是两个答案。灵动岛的 `hookState("idle")`
   → nil,岛上永远不亮。
7. **杂项打磨。** `SheetListRow` 状态点带 VoiceOver 文字(色弱/读屏不再只有颜色);两个以上
   blocked 时标题读 `2 NEED YOU`;Branch 非法时在字段下就地给原因(禁用的 Start 不再"看着像坏了");
   Prompt 多行(1–4 行生长);repo 菜单副标题 `~/code/moshi` 缩写(`AgentTaskRequest.abbreviatePath`);
   Add Connection 的 footer 补一句「herdr + Mosh 跑它自带 TUI,原生渲染要 SSH」——降级不再等连上才知道。
8. **repo 发现在真机上空手而归(真机实测踩到)。** 原因是形状:窗格 cwd 反查和 `$HOME` 扫描
   **熔在一条 exec 里**,而扫描在真实主机上根本不是 0.8s —— iCloud 同步的 home 里 `stat`
   会阻塞在物化上,高丢包链路会直接弄死 exec(实测 m1-pro:`find -maxdepth 4` 25s 未归,
   随后整条链路失联),`try?` 再把这一切吞成"只有 Other…"。修法:**拆成两条命令并发跑** ——
   cwd 反查小而稳、先到先显示;`$HOME` 扫描是锦上添花,拿 8s 客户端预算赛跑,超时只丢列表
   不拖表单。扫描本身加 `-L`(照顾 `~/code` 是软链的布局)和媒体目录 prune
   (`Music`/`Movies`/`Pictures`,iCloud home 的大头);菜单空时给一行原因,不再裸着一个 Other。
   注意残留:预算超时后**远端 find 会继续跑完**(exec 发出去收不回),所以 prune 列表就是它的
   全部上限。

验收:单测 **463 全过**(新增 25:BreadcrumbPlan 8、树标题/消歧 4、时长/排序/idle 5、
路径缩写 1、调色板 2、idle 透传改写 2,及既有断言的适配);`capture-flow-shots.sh` 补了
**staging 校验**(report-agent 静默失败过一次,导致原型页拿空态配了一段讲排序的文案 ——
现在拍不到 blocked+working 就拒绝出片)、Tabs sheet 先关再开 Workspaces sheet(22 号镜头
曾经拍了个寂寞)、新增 43 号镜头(agent 面包屑)。

**W5 · herdr 词汇/行为泄漏审计**(2026-08-08)——派专门代理全量扫 `Moshpit/`,
找出 herdr 连接下仍然显示 tmux 措辞、tmux 键位、或 tmux 形状行为的地方。
**结论:泄漏比预期严重,5 处是功能性 bug 而非用词问题。** 已修:

1. **Select Pane sheet 在 herdr 上只显示一个窗格。** `PaneBoard` 要 tmux 的
   layout 字符串,herdr 的 `WindowInfo.layout` 恒为空 → 解析返回 nil → 掉进
   "取第一个窗格、标成 `SINGLE`、硬编码 `isActive: true`" 的兜底。多窗格 tab 的
   其余窗格**从这个 sheet 完全够不着**,而它是窗格面包屑的唯一去处。
   修法:新增 `FallbackBoard`,用窗格自己上报的 `width`/`height` 判断是纵排、
   横排还是网格(herdr 的 rect 本来就解析进 `PaneInfo` 了,只是被丢掉),
   并按真实的 `activePaneId` 标记选中。
2. **锁屏/灵动岛/通知的位置串。** `location()` 是全 App 唯一不走 `displayTitle`
   的格式化点,herdr 上渲染成 `主机 · 1:1`,而且**不含 workspace**——两个不同
   workspace 的 agent 产生逐字节相同的字符串,而那张卡片的按钮是 Allow/Deny。
   修法:补 workspace + 走 `displayTitle`,现在读作 `mac-studio · ~ · Tab 1`。
3. **Install Assist 对 herdr+mosh 用户说"tmux + mosh 未安装"**并给出
   `apt-get install -y tmux mosh`。根因:`isHerdr` 从通知的包列表推断,而
   mosh-missing 通知只带 `["mosh"]`。修法:改从 `session.connection.multiplexer` 推导。
4. **两个 sheet 都印 "Swipe to switch",但 herdr 的帧通道从未接 `onSwitch`**
   (只有 tmux 的 -CC 控制器和 mosh sidecar 接了),手势是死的。
   第一版先把文案改成 "Tap to switch";真要接手势另说。
5. **删除会话时三个词打架**:行 "Kill Session"、对话框 "Kill session"、
   确认按钮 "Kill Workspace"。修法:全部走 vocabulary,并新增 **`killVerb`**
   (tmux=Kill / herdr=Close,对应 herdr CLI 的 `workspace close`)。
6. 重命名/关闭窗口、New Session 弹窗、多路复用器空态文案:全部改走 vocabulary。
7. **通知设置声称监听 "tmux 会话的响铃"**——herdr 上这条路根本不跑
   (`onPaneBell` 只在 `as? TmuxSessionController` 成立时才接)。改成机制中立文案。
8. **herdr-only 用户在无连接时被推荐"安装 agent hooks"**(可选链让 nil 读作
   "不是 herdr"),点进去是一页"请连接一台装了 tmux 的主机"。改成无活动会话时
   看已保存主机。
9. **`onlyInTmux` 快捷键开关会误伤 herdr**:`inTmux` 由 `tmuxController != nil` 喂,
   herdr 恒 false → 用户勾了"只在多路复用器里显示"的键,在他唯一想要的连接上消失。
   参数更名 `inMultiplexer` 并把 herdr 算进去;编辑器文案里的 "tmux prefix" 一并改掉。

**LOW 也一并清掉了**(同日,用户要求"剩下 low 也都处理掉"):

10. **图标进 vocabulary**。`macwindow`——字面意义上的窗口镶边——挨着 "Tab" 这个词,
    正是那种告诉 herdr 用户"这 App 是给别的东西做的"的细节。新增
    `sessionIcon`/`windowIcon`:tmux 保持 `square.stack.3d.up`/`macwindow`,
    herdr 用 `rectangle.grid.2x2`(workspace 是一块工作板,不是一摞终端)/
    `square.on.square`。sheet 行和面包屑两处都改走它。
11. **`killVerb` 补完最后六处**。W5 只改了主页树,三个 sheet 里的
    Label/确认框/按钮仍写死 "Kill"——herdr 现在一律读 "Close"。
12. **`splitHint` 从死字段变成活的**。它定义了、两个多路复用器都填了、却没人用,
    而 Select Pane 的页脚把同样的话硬编码了一遍。现在页脚走它——下一个改 herdr
    分屏措辞的人不会再改错地方。
13. **a11y id 去 tmux 化**:`tmux-install` → `multiplexer-install`,
    `tmux-create-first-session` → `multiplexer-create-first-session`。
    (先确认过仓库内外没有任何脚本/测试引用这两个 id。)

新增单测钉住 killVerb 与图标(herdr 的两个图标都不得等于 tmux 的、都不得为空)。

## 六、未决 / 风险

1. **两套表现层的分叉风险。** 约束:**数据模型只有一份**(`TmuxSnapshot` + `agentHooks`),只在 presentation 层分流。W1/W2 都是**新增页面**,不改造 tmux 的路径 —— 这是能不能维护下去的关键。
2. **外部改动最多滞后 8 秒。** 轮询在树不变时会退到 8s(见 `herdr-multiplexer.md` 的 A 优化)。App 内操作会立即刷新,别人在笔记本上的操作不会。Agents 区显示的"已持续 2:14"因此也有 ±8s 误差 —— 要不要显示秒?**建议只显示到分钟**,别给出超过数据精度的假象。
3. **worktree 落在 `~/.herdr/worktrees/<repo>/<branch>`**,不在仓库旁边。不污染工作区,但 `git worktree list` 里看着散。第一版不给 `--path` 自定义(手机上难填)。
4. **分支已存在**:herdr 报错。第一版只显示错误;W3 再考虑 `worktree open`(挂已有分支)。
5. **仓库不干净**时 `git worktree add` 的行为要实测 —— 大概率没问题(独立 checkout),但要确认不会动到用户当前工作区。
6. **agent 参数不要替用户决定**:第一版传 manifest 里的默认命令(`claude`),要加 `--dangerously-skip-permissions` 之类的人自己在 Prompt 里敲。
7. **大仓库 checkout 要几十秒**,UI 必须有进行中状态 —— Create Session 那次就是因为没有进行中反馈被误判成"没反应"。
