# Spec: 漫游桥 daemon(工作名 roadie)—— 线级施工图

状态:**技术规格,待实现** · 2026-08-18
上游文档:`roaming-transport.md`(v2.1,架构决策:为什么是薄桥、为什么长在 herdr 上、三轨对比)。
本文是它的下一层:字节格式、状态机、实现与测试计划。实现前本文先行评审。

> 命名:**roadie**(巡演搬运工——给乐队搬设备的人,贴 Moshpit 的演出隐喻,
> 短、可 brew、不含 "Mosh" 商标)。备选:`stagehand`(舞台工作人员,略长)、
> `pitcrew`。最终定名待批准,本文以 roadie 行文;协议常量不含名字,改名零成本。

## 0. 范围与非目标

**做**:一条可漫游的 UDP(可退化 TCP)传输,承载 N 条可靠有序字节通道;
端点只有两类——herdr 的 JSON socket、herdr 的 terminal attach 流。
**不做**(v1):跨断连的历史重放(herdr 协议自带重连语义:re-snapshot + 整屏 `full` 帧);
通用 exec(密钥被窃 ≠ 得到 shell,端点白名单是纵深防御);文件传输(维持按需 SSH/SFTP);
多 daemon 互联;tmux 托管(后续可选)。

## 1. L0 引导

```
(App 经已认证的 SSH exec 执行)
$ roadie new [-p LOW[:HIGH]] [--tcp] [--idle-timeout SECS]
ROADIE CONNECT 1 60741 f4T7...22字节base64...Qk   ← 唯一一行 stdout,然后守护化
```

- 行格式:`ROADIE CONNECT <proto-version> <udp-port> <key-b64>`。key = 16 字节
  (AES-128-GCM),22 字符无填充 base64——与 `MOSH CONNECT` 同构,
  `MoshBootstrap.parse` 的全部防御平移(端口 `UInt16` 类型化、banner 交错容忍、输出净化)。
- 60 秒无首个已认证数据报 → 自杀(mosh-server 同款)。`--idle-timeout`(默认 7 天)
  无活动自杀,杜绝孤儿堆积——360/361 永生循环的教训写进默认值。
- 端口:默认在 61100–61999 内取(避开 mosh 的 60000–61000,两者可共存)。
- `--tcp`:同端口再听一个 TCP;信封原样跑在长度前缀的 TCP 流上(UDP 被墙的兜底,
  客户端先 UDP 3 秒无回落 TCP)。
- 每会话一个 daemon 进程(mosh-server 模型)。不做多会话复用——进程即隔离。

## 2. L1 信封(漫游层)

每个 UDP 数据报:

```
| ver(4bit) dir(1bit) rsv(3bit) | seq u48 BE | ciphertext... |
         1 字节                      6 字节
ciphertext = AES-128-GCM(key, nonce = dir||seq 扩展到 12B, aad = 首 7 字节, plaintext)
```

- `dir`:0 = client→daemon,1 = daemon→client。两方向独立 seq,单调,不回绕
  (u48 @ 1M pkt/s ≈ 8.9 年)。nonce 由 dir+seq 决定 → 永不重用。
- **重放窗**:每方向 1024 位滑动位图;窗前/已见 → 静默丢弃。
- **漫游(核心)**:daemon 记录「当前客户端地址」= 最近一个**通过 AEAD 认证**的
  数据报的源地址。新地址认证即原子替换(tsshd 的 newest-wins;mosh 同源)。
  回包只发当前地址。客户端换网后第一个包即完成 re-home——无握手、无 RTT 惩罚。
- **保活**:双向 3 秒空闲发 PING 帧(进通道层,type 见下)。客户端 9 秒未闻回包 →
  重建本地 socket 再发(`MoshTransport.rebuildFlow` 的僵尸流经验平移)。
- MTU 1350 固定;通道层保证单帧 ≤ 1350−7−16(GCM tag)= 1327 字节,信封不分片。
- **密钥生命周期**:单会话单钥;seq 逼近 u48 上限或 24h → daemon 发 REKEY_REQUIRED
  控制帧,客户端走完整重连(SSH 重引导)。v1 不做在线 rekey。

## 3. L2 通道(复用层)

信封明文 = 一个或多个**帧**(可打包到同一数据报凑 MTU):

```
| type u8 | chan u16 BE | 载荷... |
```

| type | 名 | 载荷 | 语义 |
|---|---|---|---|
| 0 | PING | ts u32 | 保活 + RTT 采样(PONG 回 ts) |
| 1 | PONG | ts u32 | |
| 2 | OPEN | kind u8, params(见 L3) | 客户端开通道;chan 为客户端选定(奇数),daemon 侧被动 |
| 3 | OPEN_OK | credit u32 | 初始接收信用(字节) |
| 4 | OPEN_ERR | code u8, msg utf8 | |
| 5 | DATA | seq u32, bytes | 可靠有序字节 |
| 6 | ACK | cumAck u32, credit u32 | 累计确认 + 增量信用(credit-based 流控;ET #631 的教训:大流量通道不得饿死键击——每通道独立信用,信封层不设全局队列上限) |
| 7 | FIN | seq u32 | 半关;两侧 FIN+ACK 后通道号可复用(+2 递增,不立即复用) |
| 8 | RESET | code u8 | 立即废弃 |
| 9 | REKEY_REQUIRED | — | 见 L1 |

- **可靠性**:每通道独立 go-back-N:发送侧滑窗(初始 32 KiB,按信用钳制),
  RTO = SRTT + 4·RTTVAR(RFC 6298 简化,下限 200ms 上限 3s),超时重传窗内首帧;
  收到 3 次重复 cumAck → 立即重传(fast retransmit)。v1 无 SACK——通道流量特征
  (JSON 行 + 终端帧)不需要。
- **断连语义(设计立场)**:信封 30 秒无法送达(重传耗尽)→ 所有通道 RESET,
  连接进入 dead;**不做重放环**。app 层重开连接 + 重订阅 + 重 attach——herdr 的
  幂等帧和 snapshot 全量使这条路免费(v2.1 的核心简化,坚持住)。
- 单通道内帧序 = seq 序;跨通道无序保证(这正是要的:api 慢查询不阻塞终端帧)。

## 4. L3 端点(daemon 侧,白名单)

| kind | params | daemon 行为 |
|---|---|---|
| 0 `null` | — | 回声(握手自检 & 测试) |
| 1 `herdr-api` | session utf8(空=默认) | connect `${HERDR_SOCKET}`(路径解析同 herdr CLI:env → 默认);字节双向直通 |
| 2 `herdr-attach` | target utf8, cols u16, rows u16, takeover u8 | spawn `herdr terminal session control <target> --cols C --rows R [--takeover]`,stdio 直通;进程退出 → FIN(exit code 进 FIN 前的最后一个 DATA?否——FIN 载荷已有 seq;退出码走 OPEN_ERR 语义不适用,**决定:进程退出即 FIN,退出码不传**,app 靠 herdr 自己的 `terminal.closed` JSON 已含 reason) |
| 3 `herdr-observe` | 同 2 减 takeover | spawn observe(sheet 缩略图预览,只读不抢) |

- 无通用 exec。herdr 二进制解析:`$PATH` + Homebrew 补全目录(HostCapabilities 同款列表,
  daemon 侧硬编码同一组)。
- daemon 无 herdr 时 OPEN_ERR(code=herdr_missing)→ app 走既有 Install Assist。

## 5. 客户端(Swift)改造映射

| 新/改 | 内容 | 蓝本 |
|---|---|---|
| 新 `Services/Roadie/RoadieTransport.swift` | actor:信封收发/重放窗/漫游 rebuild/PING;`DatagramChannel` seam 原样复用(测试注入假信道) | `MoshTransport`(收发骨架、rebuildFlow、liveness、诊断计数器全部平移) |
| 新 `Services/Roadie/RoadieChannel.swift` | 每通道:滑窗+RTO 状态机;对外 `dataStream: AsyncStream<Data>` + `write()` | 正好是 `TmuxTransport` 协议形状——`HerdrSocketClient`/`HerdrFrameChannel` 零改动接上 |
| 新 `RoadieBootstrap.swift` | CONNECT 行解析 | `MoshBootstrap` |
| 改 `SessionHub` | 探测 roadie(bootstrap SSH 上 `roadie --version`)→ herdr 连接走桥:api 通道替代 python 泵与 sidecar;attach 通道替代 mosh raw-attach 循环与 SSH PTY 帧通道 | 降级矩阵:无 roadie → 今天的形态原样(永不静默换协议) |
| 改 方向键 | `pane.send_keys` 走 api 通道(实测 2ms) | 367 的 CLI 路径退为 fallback |
| 分发 | app 资源内嵌 4 平台静态二进制(linux x86_64/aarch64 musl、macOS arm64/x86_64,目标 <3MB/个 zstd);首次启用一次显式确认 → SFTP/base64 推到 `~/.moshpit/bin/roadie-<ver>`;版本不匹配即重推(薄,所以便宜) | 上传机制现成(`ExecUploadCommands`) |

## 6. daemon 实现计划(Rust)

```
roadie/
  src/envelope.rs   # AEAD 封解、重放窗、地址表、socket 泵    (~300 行)
  src/channel.rs    # 滑窗/RTO/信用 状态机(纯逻辑,无 IO)     (~400 行)
  src/endpoint.rs   # null / unix-socket / child-process 泵    (~250 行)
  src/main.rs       # clap、CONNECT 行、tokio 装配、TCP 兜底   (~200 行)
```

- 依赖:tokio(net/process/time)、aes-gcm、clap、rand。无 TLS、无 quinn——
  这就是它薄的原因。
- 仓库位置:**moshpit 仓库子目录 `roadie/`**(推荐:协议两端同 PR 演进、CI 一起跑、
  版本天然锁步;独立仓库等 brew tap 需要时再拆)。待批准。
- 构建:`cross` 出 musl 静态 linux 二进制;macOS 本机出双架构;CI 产物直接进 app 资源。

## 7. 测试计划(与实现同 PR,不后补)

- **channel.rs 纯逻辑**:property 测试(随机丢/重/乱序注入,断言字节流完整有序)、
  RTO/快重传/信用饿死用表驱动用例。**这层是协议的心脏,测试先行。**
- **信封**:重放窗边界、nonce 唯一性、错钥静默丢弃、漫游 newest-wins(双假地址交替)。
- **互操作**:Swift 侧 CLI(spike 模式:`RoadieTransport` + shims 编译成 mac 可执行)
  ↔ 真 daemon,中间夹一个**丢包/乱序/复制 UDP mangler**(Python,rig 经验复用);
  场景:10% 丢包下 attach 通道打字回显、api 通道 snapshot、mangler 换端口模拟换网。
- **App 生命周期**:接到任务 #14 的基建上——roadie 连接的 keepalive/重建/降级
  用假 `DatagramChannel` 脚本化(mosh 状态机测试同款手法)。

## 8. 里程碑与验收

| M | 内容 | 验收 |
|---|---|---|
| M0 | envelope+channel+null 端点,loopback | mangler 10% 丢包下 1MB 回声零差错;channel property 测试绿 |
| M1 | herdr-api 端点 + Swift 客户端最小闭环 | Moshpit(模拟器)经 roadie 拿 snapshot + 订阅推送;方向键走 send_keys <10ms |
| M2 | 漫游 + 保活 + TCP 兜底 | mangler 中途换源端口/地址,通道不断;UDP 全丢 3s 内落 TCP |
| M3 | herdr-attach/observe 端点,SessionHub 切换 | mosh+herdr 与 SSH+herdr 统一走桥;sidecar、python 泵、raw-attach 循环全部退役;真机 WiFi↔蜂窝打字不断 |
| M4 | 自动分发 + Install Assist + 降级矩阵 | 干净主机一次确认即用;拔掉 roadie 二进制,连接自动回落今天的形态且有横幅 |

## 9. 待批准的决定

1. 命名 `roadie`(§0 备选)。
2. 仓库:moshpit 子目录 vs 独立仓(§6,推荐前者)。
3. 内嵌 4 平台二进制的包体代价(~12MB 压缩前;可裁 linux-only 先行)。
4. 端口段 61100–61999 与防火墙文案(mosh 文档的同款章节要写)。

## 10. 已知风险

- herdr 漂移三案在册(退出码、agent focus、socket 不回话)——凡依赖 herdr 行为处
  必须带实测注释与版本标注;`herdr-attach` 走 CLI 而非二进制 socket 正是为此。
- herdr 上游 server-owned protocol 重写落地时,本 daemon 的退出路径 = 端点层换成
  一方协议,信封与通道层原样保留(它们不知道 herdr 的存在)。
- iOS 后台:信封无连接,恢复 = 换 socket 重发(mosh 经验);Pause/Resume 信用置零
  即天然停流,恢复即续——不需要额外协议。
