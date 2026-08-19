# tmux -CC lab — SSH+tmux 渲染的测试框架

SSH+tmux 模式下 App 是一个 **tmux 控制客户端（`-CC`）**：`TmuxSessionController`
把 `%output` 字节喂进每个 pane 自己的 SwiftTerm，用 `capture-pane` 做权威重绘
（resync），用 `resize-window`/`refresh-client -C` 把 window 钉在手机网格上。
对端（tmux、Claude Code、mosh）全是 **diff 渲染器**：它们认为没变的格子永远
不会重发，所以本地网格一旦和对端模型分歧，分歧就是永久的，且随滚动扩散
（见 memory `diff-peer-contract`）。这套框架的目标就一句话：**任何对抗性
字节流打完之后，本地网格必须收敛到 tmux 的屏幕。**

## 三层测试

### 1. 单元回放（CI 常驻）— `MoshpitTests/Services/TmuxSessionControllerTests.swift`

`MockTmuxTransport` 把 -CC 协议行直接推进控制器，逐格断言 SwiftTerm 网格。
关键场景（2026-08-19 新增）：

- `gateHoldsUntilRepairFrame` — 尺寸战争回到本机宽度后，%output 必须继续被
  门控**直到修复帧真正落地**：横跳期间 %output 事件会在转义序列中间截断，
  开门早了会把 `5;210m…` 这种序列尾巴当字面量画上屏（真机录像实锤）。
- `rejectedFrameIsRetried` — capture 帧因外来尺寸被拒后必须重试到成功；
  只 re-pin 不重抓会在横跳风暴里饿死修复（滚动时格子永久滞留的直接原因）。
- `foreignWidthLinesRejected` — 行数合格但**行宽**超网格的帧同样要拒
  （桌面帧底部空行被 trim 后行数会"合格"）。
- `scrollWhileStreamingParksLocally` — -CC 渲染路径永不驱动 copy-mode。

### 2. tmux 实验室（协议实证 + 取证）— `lab.py`

私有 socket（`-L moshpit-lab-<pid>`，`-f` 干净配置）上跑真 tmux 3.6a，
pty 驱动真实 `-CC` 客户端 + 一个可变尺寸的「桌面敌人」客户端 + `fake_tui.py`
（模拟 Claude Code：alt-screen + SGR mouse + 逐行 diff 重绘 + 静态边框列，
边框列就是"永不重发的格子"探针）。**绝不接触用户默认 tmux server。**

```bash
python3 scripts/tmux-cc-lab/lab.py copymode      # copy-mode 对 -CC 可见性
python3 scripts/tmux-cc-lab/lab.py wheel         # 滚轮 diff 重绘基线
python3 scripts/tmux-cc-lab/lab.py sizewar       # 尺寸战争全录（release→敌人赢→repin）
python3 scripts/tmux-cc-lab/lab.py capturerace   # 回宽瞬间 capture 的陈旧度
python3 scripts/tmux-cc-lab/lab.py analyze <bin> # 分析任意 -CC 字节流录像
```

产出 `stream.bin/jsonl`（-CC 客户端逐字节+时间戳）、`notes.jsonl`（步骤对齐）、
`captures/*.txt`（ground truth）、`report.txt`（layout 时间线、外来坐标违例、
序列截断点）。`analyze` 同样适用于 `~/.moshpit/cctap/` 的真机录像。

**已实证的协议事实（tmux 3.6a，2026-08-19）：**

1. **copy-mode 对 -CC 客户端完全不可见**：进入 copy-mode、滚 10 行、退出，
   -CC 流里 `%output` 为零（只有 `%pane-mode-changed`）。tmux 只为普通 tty
   客户端绘制 copy-mode → -CC 渲染器驱动 copy-mode = 滚一个看不见的屏幕，
   还会把共享 window 的桌面客户端劫持进 copy-mode。真机录像里 mode 退出时
   的 `?25l [H … ?25h` 帧是 tmux 恢复 alt-screen 应用画面，不是滚动内容。
2. **copy-mode 滚动期间 `capture-pane` 抓的是 live 底部**，不是滚动视图
   （scroll_position=10 时 before/during 两帧逐字节相同）。
3. **`%output` 事件边界会落在转义序列中间**（macOS pty ~1024B 分块），任何
   按事件粒度的丢弃门控都必须考虑半截序列（关门半截→SwiftTerm 挂起参数态，
   下一帧 ESC 自愈；开门半截→字面量乱码，必须由修复帧兜底）。
4. **战争机制**：`resize-window -x -y` 会把 window 锁成 manual 尺寸，
   `window-size latest` 失效——桌面敲键抢不回去。生产里的横跳必要条件是
   App 后台化时 `set -u -w window-size` 交还 pin。LAN 时序下
   `%layout-change` 与对应宽度的 %output 排序是干净的；破坏来自修复路径
   （拒收不重试、开门早于修复帧），不是通知乱序。
5. **回宽后立即 capture 拿到的是裁剪版旧画面**（capturerace 场景：immediate
   帧顶行 `L0024`、右边框被截；+300ms 才是真 70 宽重绘 `L0000`）。alt-screen
   在 resize 时 tmux 只裁剪不重排，而这种帧行数、行宽都"合格"，任何尺寸校验
   都拦不住——这就是 settling 双抓帧 + 拒收重试必须存在的原因；修复帧最长有
   ~700ms 的陈旧窗口，由第二遍 settle 收尾。
6. **lab 自身的坑**：pane 在 copy-mode 时外部客户端 `send-keys -l` 可能挂死
   等回包（`ext()` 已加 5s 超时兜底）；真机录制用 `~/.local/bin/moshpit-cc-tap`
   （script(1) 插 pty；管道会让 -CC 拒载）。

### 3. 全链路 e2e（真 tmux ↔ 真控制器）— `run-e2e.sh`

`lab.py serve` 起真 tmux + 桌面敌人，开两个 TCP 口：数据口桥接
`tmux -CC attach` 的 pty，控制口收 JSON 指令（敌人敲键/抓 ground truth）。
模拟器里的测试 `TmuxLabE2ETests` 用 `LabSocketTransport` 接上生产
`TmuxSessionController` + SwiftTerm，打三轮真实尺寸战争（release → 敌人赢
latest → repin → 滚动），最后**逐行比对本地网格 vs `capture-pane`**。

```bash
./scripts/tmux-cc-lab/run-e2e.sh [sim-udid]   # 无 lab 时测试自动 skip
```

## 真机取证链

1. 服务端：`~/.local/bin/moshpit-cc-tap`（把 App 的 tmux 路径指过去），
   录制落 `~/.moshpit/cctap/out-*.bin` → `lab.py analyze` 直接吃。
2. App 端：`-MOSHPIT_CC_TAP <dir>` 启动参数 → `cc-raw.bin` + `cc-pairing.log`
   （SEND/POP/FRAME/FRAME-REJECT 全配对）。
3. 症状→指纹速查：满屏碎片折行 = 外来宽度字节直画（查 layout 横跳）；
   小段 `5;210m` 字面量 = 门控切了序列（查开门时序）；滚动时部分格子不动 =
   修复帧没落地的 diff 分歧（查 FRAME-REJECT 后有没有重试成功）。
