# App Store 元数据（提交时直接复制）

状态：2026-08-08。占位符用 `⟨⟩` 标出。首发区域：**全球 − 中国大陆**（ICP 备案，
见 RELEASE-RUNBOOK）。**欧盟开放**，前提是先完成 DSA trader 认证。

## 名字（≤30 字符）

品牌定名 **Moshpit**（2026-08-09 由 Offhook 改名。*Mosh* 正是 app 说的协议，
而 mosh pit 就是一屏 agent 同时开工的样子——密、吵、全都在动；icon 换成 crowd
surf，一个人被许多只手托起。中文不另起名字）。tagline 同步换成 **"Your agents
can reach you."**。

> ⚠️ **提交前必办**：App Store 同名查证还没对 "Moshpit" 做过。之前"无同名"的
> 结论是针对旧名 "Off The Hook" 的（只有海鲜餐厅和小游戏），改名后不能顺延。

**名字讲故事，副标题扛搜索**：

1. `Moshpit: SSH · Mosh · Agents` （28 ✓ 首选——机制词全放冒号后做 SEO）
2. `Moshpit` （裸名，若 ASC 占得到可作正名 + 副标题扛全部机制词）

## 副标题（≤30 字符）

- `Answer agents from lock screen`（30 ✓ 首选——独有卖点）
- 备选：`Mosh · tmux · agent workbench`（29）

## 关键词（≤100 字符，逗号分隔，不要空格）

```
ssh,mosh,tmux,herdr,terminal,claude,codex,agent,ai,worktree,shell,cli,devops,server,console
```
（95 字符。"Moshpit"、副标题里出现过的词不用重复占位。）

## 类目

- Primary: **Developer Tools**
- Secondary: Utilities

## 描述（EN，≤4000）

```
Your coding agents don't stop when you stand up. Moshpit is the terminal
that lets you answer them from anywhere — approve a permission prompt from
the lock screen, check what Claude Code or Codex is doing from the Dynamic
Island, and start a brand-new isolated task from your phone.

A REAL TERMINAL FIRST
• SSH and Mosh — sessions that survive Wi-Fi/5G handoffs, sleep, and bad
  hotel networks, with predictive echo for high-latency links
• tmux control mode: native rendering, swipe between windows and panes,
  create/rename/kill — no tiny fake desktop squeezed onto a phone
• herdr support: the multiplexer built for coding agents, rendered one
  full-width pane at a time
• Hardware-keyboard friendly, custom themes, seven terminal fonts

BUILT FOR AGENT WORK
• See every agent at a glance: who needs you, who's working, for how long
• Needs-you pings on the lock screen — Allow, Deny, or type a reply without
  unlocking into the app
• Start an agent on a fresh git worktree from your phone: pick a repo, name
  a branch, optionally hand it the first prompt
• Agent status needs ZERO host-side setup on herdr; a one-line hook install
  on tmux

HONEST BY DESIGN
• One-time purchase. No subscription, no accounts, no feature tiers.
• Nothing is collected. No analytics, no tracking — your keys stay in the
  Secure Enclave/Keychain and your traffic goes only to your own servers.
• Moshpit never installs anything on your host silently, never creates
  sessions behind your back, and tells you when it degrades instead of
  quietly switching transports.

Works with any SSH server. Mosh, tmux and herdr are optional — install
guides are built in.
```

## 描述（zh-Hant 草稿，提交 zh-Hant/ja 本地化时再润）

```
你的 coding agent 不会因為你離開座位而停下。Moshpit 讓你在任何地方回覆它們：
鎖定畫面直接批准權限請求、靈動島查看 Claude Code / Codex 狀態、手機上直接開一個
隔離的新任務（自動建 git worktree）。

• SSH 與 Mosh：跨 Wi-Fi/5G 漫遊不斷線，高延遲鏈路預測回顯
• tmux 控制模式原生渲染；herdr 單窗格滿寬渲染，為 coding agent 而生
• Agent 狀態零安裝（herdr）；鎖屏 Allow/Deny/回覆
• 一次性買斷，無訂閱、無帳號、零資料收集
```

## 其他字段

| 字段 | 值 |
|---|---|
| Promotional Text（可随时改，≤170） | `Launch price. Answer your agents from the lock screen — one-time purchase, no subscription, nothing collected.` |
| Support URL | `https://moshpit.cluas.eu.org/support.html` ✅ 已上线 |
| Marketing URL | `https://moshpit.cluas.eu.org` ✅ 已上线 |
| Privacy Policy URL | `https://moshpit.cluas.eu.org/privacy.html` ✅ 已上线 |
| Copyright | `© 2026 ⟨你的名字⟩` |
| 年龄分级 | 问卷全部"无" → 4+（无浏览器、无 UGC 分发、终端内容自备） |
| 价格 | 首发 **$6.99**（launch price），下一档 $9.99 标准价——时间线与扳机见 pricing.md |
| App 隐私标签 | **Data Not Collected**（与 PrivacyInfo.xcprivacy 一致：零收集零跟踪） |

## 截图（提交最低集）

- iPhone 6.9"（1320×2868）**必须**：`./scripts/capture-marketing-shots.sh`（默认就是 17 Pro Max，隔离环境+真实内容）。
  ⚠️ 别用 `capture-flow-shots.sh`——它是 6.3" 文档用图，**该尺寸不能上传**
- iPad 13"（2064×2752）**必交** —— iPad 支持已确定保留（2026-08-08）。
  用 `MOSHPIT_SIM="iPad Pro 13-inch (M4)" ./scripts/capture-marketing-shots.sh` 取素材
- 建议顺序：① Agents 区(有 blocked) ② 锁屏 Allow/Deny 通知 ③ 终端+agent 面包屑 ④ New Agent Task 表单 ⑤ 灵动岛 ⑥ 连接表单/主题
