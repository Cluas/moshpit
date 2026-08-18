# Design: 预测回显(predictive local echo)

状态:**调研定稿,分期待批** · 2026-08-18
调研:双 agent 深读一手源码——VS Code typeahead(1575 行)+ sshx fork(1961 行)+
mosh 论文 §3.2 + tmux control.c + herdr wire.rs/render_stream.rs + 本仓库全部输入/回显路径。
判决所需的全部引用(file:line、issue 号)在调研原文,此处只留结论与决定。

## 0. 一句话判决

**做 overlay(mosh 式),不做字节改写(VS Code 式);用 VS Code 的输入分类器当
输入侧参考和影子模式骨架,输出侧全部换成「叠加层 vs 单元格」比对。**

## 1. 为什么不是字节改写

1. **只覆盖一种传输**。字节匹配要求服务端输出是「因果有序、最小、光标相对的回显」——
   只有 tmux -CC 的 `%output`(裸 pty 字节,tmux window.c:2422 实证)满足。
   herdr 帧(绝对定位单元格 diff,16ms 合帧,无模式序列)和 mosh(`Display::new_frame`
   屏幕 diff)结构性不满足。
2. **向 diff 协议驱动的缓冲区写预测 = 按构造复刻 363 白块 bug**:服务端按「它以为
   客户端已有的画面」算 diff,本地脏写的格子永远不会被重发。我们唯一的修复原语是
   resize 微扰——需要靠整屏重绘来擦一个错字的引擎不是引擎。
3. **原作者的判决**:微软在 #115949 自己写明 mosh 的 overlay 是正确架构
   (「powershell 每键重写整行的场景 overlay 免费支持」),然后在 2025-02 把自家实现
   默认关闭(缓冲区损坏可致误执行命令 + 密码回显 #130821 + 无失败熔断 #119103)。
   已知失败模式九类,逐条带「本可防住它的护栏」,全部进本设计的护栏清单。
4. Overlay 删掉的东西 = 失败模式的来源:SGR 影子跟踪、七个 rollback、`CSI X` 无回流
   擦除、DSR 反射过滤。「撤销预测」变成「从字典删一项」,不可能有损。

## 2. 三个关键资产(调研的意外收获)

| 资产 | 位置 | 含义 |
|---|---|---|
| **mosh EchoAck 已在线上** | `MoshWire.swift:317`(解析后丢弃) | mosh 论文的核心机制——服务端 50ms 后 ack「按键 N 的效果已在当前画面」,客户端零超时。捡起来 ≈ 一天工作量,mosh 传输的 overlay 由此完整 |
| **herdr 模式状态存在但未暴露** | herdr `pane/terminal.rs:116` `InputState{alternate_screen, application_cursor, bracketed_paste,…}`,已 Serialize | DECCKM 类问题的正解不是猜,是暴露现成结构。herdr 不收 PR ⇒ **落在 roadie daemon**(它本来就坐在 herdr 进程边界上):一次改动同时解锁 ack 和模式门控 |
| **fork 的 IME overlay 先例** | `iOSTextInput.swift:289` + `updateCursorPosition` 的让位守卫(patch 11) | 「浮层在格坐标定位 + 在远端重绘下存活」的完整先例;预测叠加层照抄机制 |

## 3. 架构(定稿)

- **输入侧**:移植 VS Code 的 `_onUserData` 分类器 + `_lastRow`/`CharPredictState` +
  代际(epoch)记账 + `PredictionStats`(MIT,久经测试,本就是 mosh epoch 模型的复刻)。
  影子模式是它的原生开关(`_showPredictions=false` 时全套统计照跑、零渲染)。
  单一输入咽喉 = `Coordinator.send`(真按键)/`sendInput`(非键盘输入一律 flushAll)。
- **验证侧**:不重写 PTY 字节。每帧落地后按 (row,col) 比对叠加层与 SwiftTerm 缓冲:
  格子被触及且同 glyph → confirm(该 epoch 转正显示);不同 → 整 epoch 刻杀;
  未触及 → 悬置(mosh 教训:不是证据)。ack 可用时(mosh EchoAck / roadie)以
  `pred.input_seq <= echo_ack` 为悬置边界,客户端零超时。
- **渲染侧(fork)**:走 `buildAttributedString` 的**单元格属性注入**路线
  (selection 注入的同款先例),不走浮动 UILabel——两个 agent 分歧处的裁决:
  我们的用户 CJK 密度高 + 363 的宽字符前科,格网保真(宽字符/换行正确)值得多付的
  实现成本;且该函数是 CoreText 与 Metal 双渲染器的唯一共用 seam,一处改两处生效。
  样式:淡色 tint,**不用下划线**(与 fork 的超链接下划线补丁冲突)。IME 组合永远
  优先于预测叠加层。

## 4. 护栏(每条对应一个已发生过的事故)

- **密码**:行为式护栏为主——零确认的 epoch 永不显示(无回显提示符下天然无渲染,
  语言无关;「密码:」regex 只当附加抑制器,我们的 CJK 用户靠 regex 必漏)。
  epoch 作用域 = (行, 命令代际);控制字符/Enter/命令变化/alt 切换/250ms 静默均重置为试探。
- **程序门控**:allowlist(bash|zsh|fish|sh|dash|ksh),不是 blocklist;信号用
  `#{pane_current_command}`(tmux)/`pane.process_info`(herdr,含整个前台进程组 argv,
  比 tmux 更强——脚本里的 `sudo -v` 可见);**永不用终端标题 regex**(#110109)。
- **宽字符**:v1 只预测单码点、wcwidth==1、ASCII 0x20–0x7E(mosh 论文测得覆盖 ~2/3
  按键;CJK 用户不劣化);预测必须按 wcwidth 占格、刻杀时全清;**363 的回放 rig 作为
  预测引擎的门禁测试**。
- **熔断**(#119103 的补课):单 pane 连续 3 次刻杀 → 关 10s,需一整行全确认才复活;
  启用精度地板 0.90(不是 VS Code 的 0.30)。
- **硬重置**:resize、herdr `full:true`、tmux capture-pane 重绘对(两 feed 之间禁新预测)、
  `%pause`、pane 切换、重连/漫游/协议切换、粘贴(且 pending 上限 64,mosh #482 的 5GB 教训)。
- **阈值**(起点值,影子模式实测后定):启用 = 精度≥0.90/64 环 + 中位 RTT≥60ms +
  样本≥20;关断滞回 <40ms;标记 = SRTT≥150ms 或悬龄≥250ms;过期 = min(1500, max(300, 2×SRTT))。

## 5. 分期(带 kill criteria)

| 阶段 | 内容 | 产出/验收 |
|---|---|---|
| S0 | 影子模式,仅 tmux -CC:输入分类器 + epoch + stats,零渲染;`pollAgentHooks` 的 list-panes 顺带捎上 `#{alternate_on} #{mouse_any_flag}`(零额外往返);诊断浮层加 PREDICTION 段 | **App 史上第一次实测 SSH+tmux send-keys 往返延迟**(每字节一个 -CC 往返,全 app 最痛路径)+ 各 pane 精度/刻杀直方图 |
| S1 | 影子全传输:验证器换单元格比对 | mosh/herdr 的可行性变成经验数据;herdr 无 ack 降级规则的 go/no-go |
| S2 | 渲染,tmux shell pane,opt-in:fork 的格属性注入(新补丁 13/14 + PATCHES.md + pin bump);`trailOnPredict`→`Ink.predictUnderline` 首次接线;Predict 徽标搬出 RoamBanner 变实况 | 精度≥0.90 门禁下的真实体验 |
| S3 | mosh:接 EchoAck(input_seq/ack 对),`PredictMode` 四件套 UI 全部通电 | 高延迟会话(mosh 的本职场景)吃到最大收益 |
| S4 | herdr:依赖 roadie daemon 的 input_seq/echo_ack + InputState 暴露(已回写进 bridge-daemon-spec) | 无 ack 则以 S1 数据裁决降级规则 |

**Kill criteria**:任一传输的 S1 影子精度在 shell pane、RTT≥60ms 下过不了 0.90,
该传输不上预测,UI 明说。

## 6. 死 UI 的处置(全清单在调研原文 §5)

- 保留:`trailOnPredict` + `Ink.predictUnderline`(改 tint 语义)、per-connection
  `predictModeRaw`、两处 picker 的 UI 形状(UI 测试钉着字面量)。
- 修改:全部文案去 mosh 化(预测是传输无关的,SSH+tmux 收益反而最大),picker 搬出
  MOSH 组;`Text("Predict ON")` 硬编码徽标(TerminalScreen:2397)改实况并搬出 RoamBanner。
- **待批**:`PredictMode.experimental` 删除(mosh 语义我们不实现,留着是空承诺;
  一行 enum 改动,无测试破坏)——建议删。

## 7. 移植时必须绕开的上游 bug(调研在源码里抓到的)

- VS Code/sshx 共有的 `eatStr` 切片错误(`slice(index, substr.length)` 把长度当终点,
  仅 index==0 时正确)——光标移动快路径从未在 chunk 中段命中过。不移植 matches 侧
  即天然规避,但输入侧移植时警惕同类。
- `CursorMovePrediction.rollback` 只还原 x 不还原 (y, baseY)(#131961 的三因之一)。
- `flushOutput` 在两个代码库里都是 no-op——物理光标可能建立在未消化自家写入的缓冲上;
  overlay 路线无此问题(不写入)。
