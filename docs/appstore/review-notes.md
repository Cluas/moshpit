# App Review 备注（提交时贴进 App Review Information → Notes）

状态：模板 · 提交前填 ⟨⟩ 占位符。审核员没有你的服务器 — **必须**给一台演示主机，
否则第一轮就是 "we were unable to evaluate"。

## Demo host（提交前 30 分钟准备）

一台一次性 VPS（或临时开的容器），开密码登录的受限账户：

- Host: `⟨demo.example.com⟩`  Port: `22`
- Username: `⟨review⟩`  Password: `⟨一次性密码⟩`
- 主机上预装：tmux + herdr（`brew install herdr` / install.sh），并预先跑一个
  `herdr server` + 一个 workspace，里面放一个假 agent 状态（`herdr pane
  report-agent <pane> --agent "Claude Code" --state blocked`），这样审核员能
  看到 Agents 区和锁屏通知的实际样子。
- 审核结束后销毁主机。

## Notes 正文（英文，直接贴）

```
Moshpit is an SSH/Mosh terminal client. It requires a remote server, so we
have prepared a demo host for your review:

  Host: ⟨demo.example.com⟩   Port: 22
  Username: ⟨review⟩         Password: ⟨password⟩

Steps to see the main features:
1. Add Connection → enter the host/username/password above → Save → tap the
   card to connect. On first connection you will be asked to trust the host
   key — tap Trust.
2. The home card shows an AGENTS section and a WORKSPACES tree: this server
   runs "herdr", a terminal workspace manager for AI coding agents. One agent
   is intentionally left in the "needs you" state so the amber status dot,
   the lock-screen notification (Allow / Deny / Reply actions) and the
   Dynamic Island Live Activity can be observed.
3. Tap any row to open the terminal. Typing goes to the remote shell.

Notes:
- The app collects no data and has no accounts (privacy label: Data Not
  Collected). All traffic goes only to servers the user configures.
- The Local Network permission is requested only for connecting to hosts on
  the user's own LAN; the demo host above is on the public internet, so you
  will not see that prompt.
- Mosh is a UDP-based remote-terminal protocol (https://mosh.org); the demo
  host also accepts it if you toggle "Use Mosh" in the connection form.
- One-time paid app: no IAP, no subscriptions, nothing to unlock.
```

## 常见 rejection 预案

| 风险 | 预案 |
|---|---|
| 2.1 "无法评估"（连不上 demo 机） | 提交前用手机蜂窝网自测 demo 凭据；Notes 里写清端口 22 未改 |
| 5.1.1 权限文案 | 本地网络/FaceID 文案已在 Info.plist，理由具体 — 一般不中 |
| 出口合规追问 | 答"standard encryption algorithms (SSH, AES-OCB3), no proprietary crypto"；ITSAppUsesNonExemptEncryption 已在二进制里 |
| 4.2 最小功能质疑 | 不太可能（完整终端）；如中，回信列 mosh/tmux/herdr/Live Activity 差异化 |
| Guideline 2.3.10 提及第三方平台 | 描述里提 Claude Code/Codex 是功能事实（检测其状态），非蹭名 — 如被挑战，改为 "AI coding agents" 泛称重提 |
