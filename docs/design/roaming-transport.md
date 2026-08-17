# Design: 可漫游传输(路线 C)—— 长在 herdr 上

状态:**提案 v2.1** · 2026-08-17
调研基准:herdr 源码 **master / 51b7064e**(本地 clone:`~/code/herdr`),herdr 0.8.0 / protocol 19
(master 已是 **protocol 20**,`src/protocol/wire.rs:16`);
传输层调研报告 + herdr 社区调研(claude.ai/code/artifact/a1e3b399-00b1-4e4b-8aa0-7893a8386a08);
路线 B 机制对照:`scripts/spike-mosh-per-window.sh`(实机 PASS)。

> 📝 **v2 修正了 v1 的核心决策;v2.1 并入社区调研后再修两处技术假设。**
> v1 主张自建全功能 daemon(自扛 -CC 托管 + 自设计应用协议),依据「herdr 外部不可控」。
> 源码核查证明 herdr 已建好几乎全部楼层,要自建的只剩一条**薄桥**。
> v2.1 的新事实:①herdr 有**两个 socket**,终端帧的 `terminal session control` 是**二进制
> 客户端协议上的 CLI 桥**,不是 JSON API 方法——桥的通道端点因此改为「spawn CLI」而非
> 「连 client socket」;②上游从「时间线不可控」硬化为「实质关闭」(见下,证据充分);
> ③**竞争警报:herdr 官方文档点名推荐的 iPhone 客户端是竞品 moshi(getmoshi.app)**
> ——herdr 生态的移动端推荐位已被占,见「竞争态势」一节。

## herdr 已经解决了什么(源码证据)

| 层 | herdr 已有 | 证据 |
|---|---|---|
| 控制面协议 | `herdr.sock`(**JSON API**):90 个方法、`events.subscribe`(pane/workspace/tab/agent/layout/worktree 全事件)、`session.snapshot` 全量——文档明说这是给「维护本地缓存的客户端,**重连后再拉一次 snapshot**」用的 | `herdr api schema --json`(protocol 19);herdr.dev/docs/socket-api |
| 终端帧 | `herdr terminal session observe/control <target>`:NDJSON 帧(`seq`/`full`/base64 ANSI)+ `terminal.input/resize/scroll/release`,控制端独占 + `--takeover`。**注意:这是二进制客户端协议(`herdr-client.sock`)上的 CLI 桥,不在 JSON API 里** | `src/client/mod.rs:949-1050`;schema 中无 terminal.* 方法 |
| 跨机传输 seam | `--remote`:本地瘦客户端 ↔ SSH stdio 字节泵 ↔ `herdr-client.sock`;远端半场是 41 行的 `remote-client-bridge`(`run_remote_client_bridge()`:connect + 双向 `copy_flush`)。**传输已抽象为「给我一条双向字节管道」** | `src/remote/attach.rs`、`src/remote/host_unix.rs`、`src/main.rs:575` |
| 断连语义 | snapshot 全量重建 + subscribe 重订 + attach 整屏 `full` 帧;server 侧 live handoff。**事件信封无 seq/id,断连后只能 re-snapshot+diff,不能事件重放**(Disc #2577 提过,未回应)——恰好与本设计「不做历史重放」一致 | docs;schema |
| 许可 | **v0.8.0 起 Apache-2.0**(#1340 逐人授权迁移;**≤0.7.5 仍是 AGPL,fork 必须从 ≥0.8.0 拉**);无 CLA | `LICENSE`;CHANGELOG |
| 生态位 | 29.9k★/4.7 个月,homebrew-core 月装 21.7k;第三方客户端一大排(collie ★430、herdr-remote ★258、ghosthub、whip、herdr-mobile-relay……)**全部以外部进程形态存在,没有一个解决了原生漫游**;`Mic92/herdr-eternal`(QUIC+WS 外置代理)是与本设计同形态的先行者(Disc #1923,无维护者回应) | GitHub;awesome-herdr |

**唯一缺的:字节管道只有 SSH(TCP)一种形态**——换网即断、iOS 后台即死。这正是 Moshpit 的地盘。

## 上游现实:实质关闭(证据链)

- `CONTRIBUTING.md`:**不接受未经批准的实现 PR**,bot 自动关闭;`APPROVED_CONTRIBUTORS` 68 人,
  「名单不是申请制」。史上最大外部合并 ~1.5k 行(还是 DHH 的)。
- **同款功能已经被人做完并被拒过**:PR #1641(SSH 引导的可续传 QUIC `--remote`,
  +6999/−331、31 文件、附设计文档、先在 Disc #1640 对齐过)——**开出 10 秒被 bot 关闭**,
  Discussion 四周无维护者回应。mosh 传输提案 #1779 开出 **7 秒**被关(模板违规);
  WebSocket #188、Eternal Terminal #2635 同命运。
- 维护者(solo founder,Ogulcan Celik,**YC F26,Herdr Inc.**)已把这层保留给自己:
  「backend server-client architecture 是当前最高优先级,我想自己做」(Disc #515,置顶请求 ↑72);
  `AGENTS.md:65`:「Herdr 正迁移到 server-owned runtime protocol,TUI 只是其中一个客户端」。
- **战略含义**:那次重写很可能产出网络友好的一方协议,甚至成为 herdr 的商业形态
  (YC 文:「multiple clients, a laptop, a VPS」)。对我们既是机会(原生协议可换掉薄桥)
  也是竞争风险(一方移动客户端)。**桥必须做薄,随时可弃。**

## 竞争态势 + 关系轨(单独一轨,低成本维护)

**herdr 官方文档《Work from your phone》点名:"On iPhone, apps like moshi (getmoshi.app) work well."
——全文档唯一被点名的 iOS app,而 moshi 是竞品**(herdr.dev/docs/how-to-work,
仓库内 `docs/next/.../how-to-work.mdx:58`)。竞争读法:

- herdr 对手机场景的官方叙事是「装任意 SSH 客户端连上去跑 herdr TUI」——moshi 占的是
  **通用 SSH 客户端**推荐位。Moshpit 的原生 herdr 集成(帧通道原生渲染、agent 状态直入
  灵动岛、零 hook 安装)与本设计的漫游传输,都是 moshi 叙事覆盖不到的层——**差异化在
  「原生 + 漫游」,不在「也能连上」**。
- 该推荐位不是排他的("apps like"),靠产品事实可以进入/替换;渠道是 Show and tell
  与文档 PR 之外的 sanctioned 路径(下)。

关系轨动作(全部走 sanctioned 渠道,不碰 PR 红线):

1. 高质量可复现 bug 报告(这是进 APPROVED_CONTRIBUTORS 的唯一文档化路径)。现成的移动端相关
   issue 可参与:#2815(iOS 上 SSH 触摸滚动)、#2404/#2405(一台手机 attach 把桌面端布局压窄
   ——正是 herdr-multiplexer.md 里我们关心的 per-client sizing)、#1104(kitty 图形 APC 在 iOS 漏成明文)。
2. 唯一值得提的微型补丁:`Command::new("ssh")` 硬编码(`src/remote/attach.rs:458`)加程序覆盖
   ——一行、非架构、解锁整个外置传输生态(Mic92 也在等)。注意它按 PATH 解析,
   **今天就可以用名为 `ssh` 的 shim 二进制绕过,无需等上游**。
3. Show and tell 发 Moshpit 集成(collie/whip/herd-eternal 都在那)。
4. **盯住 backend 重写**:server-owned protocol 落地时重估本设计。

## 三轨对比(v2.1 定稿)

| | ① 薄桥 daemon(主轨) | ② 上游(关系轨,非交付轨) | ③ fork/vendor(兜底) |
|---|---|---|---|
| 内容 | 漫游 UDP 信封 + 通道 ↔ herdr 公开 seam | bug 报告 + 微补丁 + show&tell | Apache-2.0(≥0.8.0!)fork |
| 依赖 herdr 内部 | **零**(JSON socket + CLI 桥,全是版本化/生产化的公开面) | — | 全部,且 headless.rs 是 440KB 单文件、周更、正被重写 |
| 先例 | herd-eternal 同形态在跑 | 无人成功交付过传输 | 无 |
| 风险 | JSON API 漂移(有 schema 版本化 + 字段缺失即降级纪律) | 时间线为零预期 | 跟版成本 + 「逼用户装奇怪 herdr」 |

## 薄桥设计(v2.1)

```
Moshpit (iOS)                            远端主机
┌──────────────────────┐                ┌────────────────────────────────────┐
│ HerdrControlClient   │                │ bridge daemon(新,Rust,~1-2k 行)  │
│  → JSON 直连+订阅    │ ←UDP 信封+通道→ │  通道类型A ↔ connect herdr.sock     │
│ HerdrFrameChannel    │    (漫游)      │  通道类型B ↔ spawn `herdr terminal  │
│  (NDJSON 分帧原样)   │                │     session control <pane>` (stdio) │
└──────────────────────┘                │ herdr server(不改)                 │
                                        └────────────────────────────────────┘
```

- **L0 引导**:SSH exec 起 daemon → `<NAME> CONNECT <port> <key>` → 关 SSH。
  `MoshBootstrap` 同构,解析防御复用。**SSH 仍是唯一信任根**(PR #1641 的表述值得照抄:
  "no new auth surface")。
- **L1 信封(漫游)**:AES-GCM + 单调序号 + clientID,newest-authenticated-wins(tsshd 形);
  客户端 = `MoshTransport` 的 rebuildFlow/resume/liveness 平移。UDP 被墙走同信封 TCP 兜底。
  **必须自带鉴权:herdr socket 零鉴权(无 peer-cred、无 token,仅 0600 文件权限),
  裸暴露 = 任意命令执行**(`pane.send_input`/`agent.start`)。
- **L2 通道**:活着时每通道可靠有序(ARQ 滑窗);**不做历史重放**——通道死了重开 +
  re-snapshot + 重 attach(herdr 的事件无 seq,重放本来就不可能;`full` 帧幂等)。
  键击可选 datagram 双发(tezzer 技巧)。
- **L3**:通道类型 A = `herdr.sock` 的 JSON 原字节(控制面);通道类型 B = daemon spawn
  `herdr terminal session control/observe` 的 stdio(帧面,NDJSON)。
  **刻意不碰 `herdr-client.sock` 的二进制协议**:它要求客户端与服务端**协议号完全一致**
  (`ensure_remote_server_running` 直接报错),herdr 每 2–6 周 bump 一次(16→19→20),
  上架的 iOS app 跟不起;CLI 桥由 herdr 自己保证同机版本一致,天然免疫。
- **帧语义已知边界**:帧是合帧后的屏幕状态,快速滚动**有损**(实测 200 行进 23 行 pane
  只出 5 帧)——scrollback 继续走 `terminal.scroll`/`pane.read --source recent`,
  与现状一致,不指望帧重放重建历史。

## 客户端改造(对照现状全是升级)

| 现状(CLI-over-SSH) | 桥之后 |
|---|---|
| `herdr api snapshot` 2–8s 轮询 exec | `events.subscribe` 推送;snapshot 仅在连接/重连时拉 |
| 帧通道跑在 PTY 登录 shell,`stty raw -echo`,4096 字节行上限 | daemon 直接 spawn CLI,无 shell、无行规程 |
| 每写操作一个 exec + `__moshpit_rc=$?` | JSON 请求/响应,结构化 `error_response` |
| CLI 没有 `pane.focus`(只能 `agent focus` 吞错) | socket 全量 90 方法 |
| mosh 模式额外一条 sidecar SSH | 一条 UDP 全包,sidecar 退役 |
| `HerdrFrameChannel` / `HerdrSnapshot` 解码器 | **原样复用** |
| tmux 路径 | 不变(后续可选:桥加通用 exec 通道托管 -CC) |

## 分期

**Phase 0 — socket 直连验证(dev-only 管道,不写 daemon)** 🚧 **协议层已完成**(2026-08-17)

已落地:
- **wire 协议全部实锤**(herdr 0.8.0 / protocol 19,真实抓包进 fixtures):请求 `{"id","method","params"}`
  ——**id 必须是字符串**(整数 id 被静默忽略,连错误都没有)、**params 必填**(缺了同样静默);
  响应 `{"id","result"|"error"}`;解析失败回 `"id":""` 且 **server 随即断连**;
  `pane.agent_status_changed`/`pane.scroll_changed`/`pane.output_matched` 是**按 pane 订阅**
  (必须带 `pane_id`),其余全局。
- **意外之喜:subscribe 自带全量引导**——订阅成功后 server 先把现存 workspace/tab/pane
  以合成 created/focused 事件回放一遍,再进入实时流。重连语义因此比设计假设更省
  (甚至不必 snapshot+diff,重订阅即重引导;snapshot 仍保留为校验手段)。
- `terminal session control/observe` 经 `ssh -T`(exec + 管道 stdin)**全通**:resize 被消费、
  stdin EOF 干净 `terminal.closed`,不需要 PTY/stty。
- **`HerdrSocketClient` 落地**(`Services/Herdr/HerdrSocketClient.swift`):传输无关
  (feed + write 闭包,`TmuxControlClient.feed` 同款模式),请求关联、超时、空 id 全灭、
  EOF 全灭、订阅编码;9 条单测吃真实抓包(`MoshpitTests/Fixtures/herdr-socket-*`),
  全套 605 条单测通过。响应行原样交给 `HerdrSnapshot.decode`(socket 信封与 CLI 一致,
  `result.snapshot` 嵌套)。
- **Citadel 0.12.1 复验:exec 通道仍无可写 stdin**(`ExecCommandStream` 只有 stdout/stderr;
  `withTTY` 标注 macOS 15)。⇒ App 内的过渡管道沿用帧通道已验证的 PTY + `stty raw -echo`
  方案跑 socket 泵(socat/python3,能力探测 + 缺失即降级回轮询);Phase 1 的桥彻底替掉它。

**接入已完成(2026-08-17,待真机验证):**
- `HerdrPushBoot.bootLine()`:PTY 泵启动行(`stty raw -echo; exec python3 -c 'exec("…")'`,
  单行、无单引号、python3 直连 herdr.sock)。**完整 boot line 经真实 ssh 对本机 herdr
  打 ping 已 e2e 验证**(pong 原样返回)。
- `HerdrPushDriver`:订阅全部 24 种全局事件(**刻意排除三种按 pane 订阅的**——裸订阅是
  invalid_request 且 server 随即断连),事件 → 200ms 尾去抖 → `quicken()`。
  轮询保留为安全网;push 只收窄延迟,不承担正确性。
- `HostCapabilities` 探 `python3`(`hasPython3`,旧缓存缺 key 默认 false,unknown 乐观 true)。
- `SessionHub`:SSH+herdr 与 mosh+herdr 各接 `startHerdrPushUpgrade()`(专用 SSH 连接 + PTY 泵;
  失败静默留在轮询),`stop()` 全量 teardown。
- 单测 611 全过(HerdrSocketClient 9 条 + HerdrPushDriver 6 条,fixtures 为真实抓包)。

剩余:真机验证(事件推送延迟 <100ms、灵动岛状态翻转即时、后台恢复路径)。

**Phase 1 — 薄桥 daemon(漫游)**
L0/L1/L2 落地;Moshpit 把 Phase 0 管道换成 UDP 通道。
验收:WiFi↔蜂窝切换打字不断;后台 5 分钟回前台 ≤1s;断网 30s 通道重开 + 重 attach 自动完成。

**Phase 2 — sidecar 退役 + 灵动岛事件驱动**
SSH+herdr 与 mosh+herdr 统一走桥;`AgentActivityMonitor` 对 herdr 从 2s sweep 改为
`pane.agent_status_changed` 事件驱动(`trackWhenReady` 已就位)。

**Phase 3 — 预测回显 + 关系轨收获**
typeahead(MIT、纯客户端)先行,`#{pane_current_command}`/herdr `agent` 字段做逐 pane 门控;
mosh 级 echo-ack 依赖服务端 50ms 语义——herdr 不收 PR 的现实下,作为 Discussion 提案挂着,
同时评估 daemon 侧能否用 `pane.wait_for_output` 近似。backend 重写落地时重估整个设计。

## 未决 / 需要验证

1. ~~**Citadel exec 写 stdin**~~——已复验(0.12.1):**仍然不行**,公开 API 的 exec 只有
   stdout/stderr。走 PTY + stty raw 过渡,Phase 1 桥替掉。
2. **`terminal session control` 的输入延迟**在 spawn-per-pane 形态下是否达标(vs 二进制协议);
   observe 多路并发(sheet 缩略图)资源占用。
3. **多客户端语义**:手机 + 桌面同时在线时 takeover 行为(#2404/#2405 的 per-client sizing
   问题在 herdr 侧仍未解,我们的 resize 只作用于自己 attach 的 pane,需实测确认)。
4. **daemon 命名与分发**:不含 "mosh"(商标);brew tap + install.sh;Install Assist 接入。
5. **上传通道**:维持按需 SSH/SFTP,不进桥 v1。
6. **herdr 商业化路线的竞态**:server-owned protocol / 一方移动端若出现,薄桥的退出路径
   (设计上已保证:零 herdr 内部依赖,随时可换成一方协议)。

## 证据链

- herdr 源码(`~/code/herdr`,master/51b7064e):`src/remote/attach.rs`、`src/remote/host_unix.rs`
  (41 行桥)、`src/main.rs:575`、`src/client/mod.rs:949-1050`、`src/server/socket_paths.rs`(0600)、
  `src/protocol/wire.rs:16`(protocol 20)、`CONTRIBUTING.md`、`AGENTS.md:65`。
- 社区证据:PR #1641(10 秒被关的 7k 行 QUIC 实现)、Disc #515/#1640/#1780/#1923/#2635/#2577、
  Issue #1779/#188/#1340(relicense)/#2815/#2404/#2405/#1104;herdr.dev YC 博文;
  **herdr.dev/docs/how-to-work 对竞品 moshi 的官方推荐**。
- 传输层调研报告 + herdr 社区调研全文(文头 artifact 链接)。
- 路线 B spike(`scripts/spike-mosh-per-window.sh`,实机 PASS):对照组保留。
