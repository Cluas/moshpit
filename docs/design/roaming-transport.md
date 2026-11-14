# Design: herdr 控制面推送(路线 C 的 Phase 0)

状态:**Phase 0 已上线**(协议层与接入均在 2026-08-17 落地)。路线 C 的其余部分——自建漫游桥
daemon(Phase 1)、sidecar 退役与事件驱动的灵动岛(Phase 2)、基于桥的预测回显(Phase 3)——
**没有做**,设计稿已从仓库删除(git 历史里有本文的 v2.1 原稿和 `bridge-daemon-spec.md`)。
本文只记录还在跑的部分:herdr 的两条协议面、Phase 0 的推送管道,以及接入时实测出来的协议事实。
调研基准:herdr 0.8.0 / protocol 19(master 当时已是 protocol 20,`src/protocol/wire.rs:16`)。

## herdr 的协议面(源码证据)

| 层 | herdr 已有 | 证据 |
|---|---|---|
| 控制面协议 | `herdr.sock`(**JSON API**):90 个方法、`events.subscribe`(pane/workspace/tab/agent/layout/worktree 全事件)、`session.snapshot` 全量——文档明说这是给「维护本地缓存的客户端,**重连后再拉一次 snapshot**」用的 | `herdr api schema --json`(protocol 19);herdr.dev/docs/socket-api |
| 终端帧 | `herdr terminal session observe/control <target>`:NDJSON 帧(`seq`/`full`/base64 ANSI)+ `terminal.input/resize/scroll/release`,控制端独占 + `--takeover`。**注意:这是二进制客户端协议(`herdr-client.sock`)上的 CLI 桥,不在 JSON API 里** | `src/client/mod.rs:949-1050`;schema 中无 terminal.* 方法 |
| 跨机传输 seam | `--remote`:本地瘦客户端 ↔ SSH stdio 字节泵 ↔ `herdr-client.sock`;远端半场是 41 行的 `remote-client-bridge`(`run_remote_client_bridge()`:connect + 双向 `copy_flush`)。**传输已抽象为「给我一条双向字节管道」** | `src/remote/attach.rs`、`src/remote/host_unix.rs`、`src/main.rs:575` |
| 断连语义 | snapshot 全量重建 + subscribe 重订 + attach 整屏 `full` 帧;server 侧 live handoff。**事件信封无 seq/id,断连后只能 re-snapshot+diff,不能事件重放**(Disc #2577 提过,未回应)——恰好与本设计「不做历史重放」一致 | docs;schema |

App **从不直接说 `herdr-client.sock` 的二进制协议**:控制面走 JSON(`herdr api` CLI 轮询 + 下面的
socket 推送),终端帧走主机上的 CLI 桥(`terminal session control`)。二进制协议要求客户端与服务端
**协议号完全一致**(`ensure_remote_server_running` 直接报错),herdr 每 2–6 周 bump 一次
(16→19→20),上架的 iOS app 跟不起;CLI 桥由 herdr 自己保证同机版本一致,天然免疫。

## Phase 0 — 推送升级(轮询之上的快路径)

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
  方案跑 socket 泵(python3,能力探测 + 缺失即降级回轮询)。

**接入已完成(2026-08-17,待真机验证):**
- `HerdrPushBoot.bootLine()`:PTY 泵启动行(`stty raw -echo; exec python3 -c 'exec("…")'`,
  单行、无单引号、python3 直连 herdr.sock)。**完整 boot line 经真实 ssh 对本机 herdr
  打 ping 已 e2e 验证**(pong 原样返回)。
- `HerdrPushDriver`:订阅全部 24 种全局事件(**刻意排除三种按 pane 订阅的**——裸订阅是
  invalid_request 且 server 随即断连),事件 → 200ms 尾去抖 → `quicken()`。
  轮询保留为安全网;push 只收窄延迟,不承担正确性。
- `HostCapabilities` 探 `python3`(`hasPython3`;探测前的 unknown 乐观为 true)。
- `SessionHub`:SSH+herdr 与 mosh+herdr 各接 `startHerdrPushUpgrade()`(专用 SSH 连接 + PTY 泵;
  失败静默留在轮询),`stop()` 全量 teardown。
- 单测 611 全过(HerdrSocketClient 9 条 + HerdrPushDriver 6 条,fixtures 为真实抓包)。

剩余:真机验证(事件推送延迟 <100ms、灵动岛状态翻转即时、后台恢复路径)。
