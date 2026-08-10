# 官网重构建议(给 site-next 的 agent)

2026-08-10,来自负责 App/发布侧的 agent。背景:TestFlight 外测链接即将生效
(Beta 审核 ~24h),正式上架无固定日期。**网站现在的第一任务:承接 beta 流量,
并为上架日准备好一键切换的 CTA。** 以下按"定案 → 红线 → 待办"组织;文案事实源
是 `docs/appstore/metadata.md`、`pricing.md`、`RELEASE-RUNBOOK.md`,数字别自造。

## 1. 信息层级(已定案,不必重开辩论)

- **Hero 只打一个场景:人不在电脑前,从锁屏指挥 agent。** 画面 = 灵动岛跳
  "needs you" → 锁屏按 Allow → agent 继续跑(dist 里已有 20/21-lockscreen 素材,
  方向对)。副标题级口号沿用 App Store 的 `Answer agents from lock screen`。
- 之下三支柱,顺序固定:
  1. **真终端**:SSH + Mosh(换网/断网会话不死)+ 原生 tmux 控制模式;
  2. **为 agent 而生**:herdr 零配置状态、tmux 一行 hook、从手机开 worktree
     起 agent、**端侧语音口述 prompt(新)**;
  3. **诚实**:一次性买断、无账号、无埋点、流量只去用户自己的服务器。
- **herdr 词策略**:App Store 名已定 `Moshpit: herdr & tmux Terminal`(吃搜索
  风口),但**站外受众不认识这个词**——页面职责是教育它(docs/herdr 页保留并强
  化),hero 里说"结果"不说名词。争取一个增长动作:herdr 官方 README/文档加一行
  "iOS client: Moshpit" 反链,价值大于任何站内 SEO。

## 2. 措辞红线(承诺型文案,与代码/法务锁死)

- **语音输入**:可以放心写 "transcribed entirely on-device — audio never
  leaves the device"。这不是营销话术,代码强制了端侧(服务器识别路径已物理
  移除),但**只许用于语音功能,不许泛化**到"整个 app 无网络"之类。
- **隐私**:口径 = "Data Not Collected"(与 PrivacyInfo.xcprivacy、ASC 隐私
  标签一致)、no accounts、no cloud relay、keys in Secure Enclave。
- **价格**:**$6.99 一次性买断**(2026-08-10 刚修订,旧稿里的 $4.99 全部作废);
  "one-time purchase, no subscription, no IAP" 是核心承诺,别加"限时/折扣"。
- **地区**:出口合规按"不在法国分发"申报——避免 "available worldwide" 这类
  绝对表述;上架初期也排除中国大陆(ICP)。
- **中文页面绝对禁词**:VPN / 代理 / proxy / 翻墙 / 科学上网 / 加速器。zh 文案
  未来会复用到中国区元数据,沾上 5.4(VPN 类)个人账号直接出局。
- 竞品对比沿用 compare 页的三分桶框架(订阅制终端 / 买断 tmux 客户端 /
  agent-first 云中转应用)。注意 **getmoshi.app 的 "Moshi" 是 agent-first 桶的
  直接竞品**——对比行要能区分我们(真终端+无云中转),但别指名攻击。

## 3. 不可破坏项(硬约束,重构最容易炸的地方)

- **`/privacy.html` 与 `/support.html` 已填进 App Store Connect**(TestFlight
  外测的 Privacy Policy URL 字段)。新站这两个**精确路径必须 200**。Astro 若走
  `/privacy/` 目录式路由,在 nginx.conf 里补 `.html → /` 的 301 或别名,部署后
  立刻 curl 验证这两个 URL。
- 老站 docs 深链(`docs-herdr.html`、`docs-intro.html` 等)如有外部引用,变
  URL 要 301;sitemap 重新提交。
- 中文镜像:老站是 `*.zh.html` 约定 → Astro i18n 后确保每页 hreflang 成对、
  zh 页不掉队(历史教训:zh 页容易漏更新)。

## 4. 新内容待办

- **Voice input 区块/页**(本周刚上的功能):要点 = mic 键在终端快捷条上、
  先预览后 Insert(不会误敲进 shell)、端侧转写、中文可用、语言可选。截图素材
  还没有——需要"终端 + 听写浮层(LISTENING + 转写文本)"一张、"Insert 落进
  prompt"一张;capture 体系在 `scripts/capture-flow-shots.sh` + 
  `-MOSHPIT_SEED_*` 参数,找 App 侧 agent(我)配合出图即可,别手截模拟器。
- **Beta 期 CTA**:hero 主按钮 = "Join the beta (TestFlight)",链接等 Beta 审核
  通过后由 Wenlong 提供;做成单点常量/组件,上架日换成 App Store badge
  (Apple ID 6799896801)+ "$6.99, one-time"。
- 对比表加一行 Voice input(on-device)。

## 5. 验收与部署(既定家规,不是建议)

- **每页真实浏览器截图核对过才算完成**——HTTP 状态码、grep、构建成功都不算
  证据。桌面 + 移动两档宽度,亮暗两个主题(站点若只有暗色则暗色即可)。
- 部署:k3s 直连 kubectl(不走 iai)。site-next 已是 Dockerfile + k8s.yaml 的
  镜像流,保持;老的 `scripts/deploy-site.sh` 是 ConfigMap 流,切换后要么适配
  要么明确废弃,别留两套都"半能用"。
- 域名沿用 moshpit.cluas.eu.org;OG/twitter 卡片图建议直接用锁屏场景截图。

## 6. 一句话总结

**Hero 卖"锁屏指挥 agent"这个瞬间;herdr 用来教育而不是假设人人认识;
承诺型文案(端侧语音/零收集/买断 $6.99)一字不能松;privacy.html 一秒不能 404。**
有拿不准的事实,以 docs/appstore/ 三个文件为准,或直接问发布侧。
