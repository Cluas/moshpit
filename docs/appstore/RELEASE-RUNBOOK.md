# 上架执行手册（账号批下来照着跑）

状态：2026-08-08。前提：个人开发者账号（已决定）、付费买断（首发 $4.99 →
$6.99 → 标准价 $9.99，见 pricing.md）、首发排除中国大陆（已决定）、iPad 保留、欧盟开放。

## ✅ 已拍板（2026-08-08）

1. **iPad 保留**（`TARGETED_DEVICE_FAMILY: "1,2"`，project.yml 里已注明是决定而非默认）。
   随之而来的两件必办事项，都不能省：**每次发版都要交 13" iPad 截图**
   （2064×2752），以及**发版前真机/模拟器过一遍 iPad 布局**——审核员会在 13"
   屏上打开它。这是单向门：卖出去过 iPad 版就不能再撤。
   **2026-08-08 实测**（iPad Pro 13" M5 模拟器）：能装能跑、布局不破——但它就是
   一套居中的 iPhone 布局，13" 上左右大片留白。审核不会因此拒（4.2 讲的是功能
   不是布局），但用 iPad 的人会看出来。要不要投入做真正的 iPad 布局
   （侧栏连接列表 + 右侧终端的分栏）是一个独立的产品决定，**不阻塞首发**。
2. **欧盟区开放。** 因此必须在 App Store Connect 完成 **DSA trader 认证**：
   公示姓名/地址/邮箱/电话，并勾选 "I am a trader"。**未完成认证的 App 会被
   欧盟区下架**，所以这一步排在提审之前，不能拖到上线后。地址会对欧盟用户
   公开可见——如果不想用住址，先准备一个可收信的地址。
3. **站点已上线** —— <https://moshpit.cluas.eu.org>（`/privacy.html`、
   `/support.html` 同域）。三个 URL 见 metadata.md，可直接填。

## 账号批准当天

- [ ] App Store Connect → Agreements, Tax, and Banking：
      签 **Paid Applications** 协议 → 填银行账户 → 税表（个人选 W-8BEN）。
      不齐这个，付费 App 审过了也上不了架。
- [ ] 申请 **Small Business Program**（15% 抽成）：developer.apple.com →
      App Store Small Business Program → enroll。批准是前瞻性的，尽早交。
- [ ] Xcode ▸ Settings ▸ Accounts 登录该 Apple ID → `./scripts/team-id.sh`
      → 把 10 位 Team ID 填进 `Signing.xcconfig` →
      `git update-index --skip-worktree Signing.xcconfig`
- [ ] certificates 页面确认自动签名能建 **Apple Distribution**（首次归档时
      Xcode 会代办；公司电脑要注意钥匙串权限）

## 建 App（占名越早越好）

- [ ] ASC → My Apps → ＋ New App：
      Bundle ID `com.cluas.moshpit`（已在 project.yml），
      名字先试裸名 `Moshpit`，占不到用候选 1 `Moshpit: SSH · Mosh · Agents`，
      SKU 随意（`moshpit-ios`），语言 English (U.S.)。
- [ ] Pricing：首发 $4.99；Availability：**全球 − 中国大陆**（ICP，见下）。
      欧盟**开放**，但先完成 DSA trader 认证。
- [ ] **DSA trader 认证**（欧盟区必需）：ASC → Business → Trader Status，
      填姓名/地址/邮箱/电话并提交验证。没做完就发布 = 欧盟区被摘。

## 出包与内测

- [ ] `./scripts/release-archive.sh` → `build/AppStore/Moshpit.ipa`
      （构建号=commit count，appex 自动同步 — 脚本都处理了）
- [ ] 传上去：Xcode Organizer 或 Transporter.app 拖 ipa
- [ ] TestFlight：内部测试组拉自己+同事（**用 TestFlight 替代 7 天续签的
      sideload**；促销码留给正式版）
- [ ] 真机过一遍：Live Activity / 锁屏 Allow / herdr 三件套 / 付费下载路径无碍

## 填元数据（全部备好，见同目录）

- [ ] 描述/副标题/关键词 ← `metadata.md`
- [ ] 隐私标签：**Data Not Collected**（与 PrivacyInfo.xcprivacy 一致）
- [ ] 隐私政策 URL ← `privacy-policy.md` 放上站点后的地址
- [ ] 截图：`MOSAIC_SIM="iPhone 17 Pro Max" ./scripts/capture-flow-shots.sh`
      取 6.9"（1320×2868）原始素材（默认的 17 Pro 是 6.3"，**不能上传**）；
      **iPad 13"（2064×2752）必交** —— iPad 已确定保留
- [ ] 年龄分级问卷（全"无" → 4+）
- [ ] App Review 备注 + demo 主机 ← `review-notes.md`（提交前 30 分钟起机）

## 提交与放行

- [ ] 出口合规追问答"standard algorithms"（键已在二进制）
- [ ] 版本发布方式选 **Manually release**（审过后自己挑时间上线，配合发帖）
- [ ] 审核通过后：Features → Promo Codes 生成（每版本 100 个、28 天有效）发员工
- [ ] 上线当天：Show HN / X 发 30 秒视频（锁屏 Allow 一个 Claude Code 权限请求）

## 中国区（首发不做，2026-08-08 调研结论）

卡点是 **MIIT 的 APP备案**，Apple 在 ASC 硬校验（app 名与主体名都要与备案记录
逐字一致）。个人可以备案、成本很低（约 ¥100–200/年、1–3 周），**但必须通过
大陆云厂商提交，需要中国身份证人脸核验 + 一台大陆云主机 + 实名域名**。
Apple 的备案豁免只覆盖"不联网或只连 Apple 服务器"的 App —— 我们连用户自己的
SSH 主机，不符合。

其余三条都不是障碍：个人开发者账号可上中国区；加密不需要任何许可（密码法
第 28 条，大众消费类产品不实行进出口管制）；SSH 类目在中国区健康（Prompt 2、
Echo、Conduit 都在架，含个人开发者与 mosh）。

⚠️ 唯一的雷是**文案**：中国区元数据里绝不能出现 VPN / 代理 / proxy / 翻墙 /
科学上门 / 加速器 —— 一旦被归入指南 5.4（VPN），个人账号根本没有资格。

两个未坐实、真要做之前必须先问管局或代办的点：**个人备案能否用于付费 App**
（经营性认定有分歧）；**个体工商户能否作为备案主体**。

## 回本仪表

目标：**~24 份 × $4.99**（Small Business 15% 抽成，净 ~$4.24/份）≈ $99 年费。
回本后按 pricing.md 的扳机涨到 $6.99——**回本本身就是第一个扳机**。
两个月 < 5 份 → 是曝光问题，加帖不降价。
