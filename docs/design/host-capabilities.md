# Design: 目标机器缺 tmux / mosh-server 的处理

状态:设计稿(待用户确认后实现) · 2026-06-13

## 问题

Beacon 假设目标机已装 tmux(会话持久化/原生导航)和 mosh-server(漫游)。
现实中大量主机两者皆无。当前的失败形态:

| 场景 | 现状 | 问题 |
|---|---|---|
| SSH+tmux,无 tmux | `tmux -CC attach` 命令找不到 → 永远停在 "Attaching tmux…" → 空状态误报 "No tmux sessions" | 误诊;Create Session 也会失败 |
| mosh,无 mosh-server | bootstrap exec 报错 → 弹原始错误串(`command not found`) | 用户不知道怎么办 |
| mosh,无 tmux | mosh 连上但 `tmux attach` 失败,留在裸 shell;sidecar 每 2s 重试制造服务端噪音 | 半残体验 |

## 原则

1. **永远能连上**:依赖缺失绝不阻断连接——降级,不失败。SSH 是保底传输,裸 shell 是保底形态。
2. **说人话**:明确告知"缺什么、影响什么、怎么装",不甩 stderr。
3. **不偷装**:任何安装动作必须用户显式触发,且**在终端里可见地执行**(sudo 密码、输出全程可见),Beacon 只代填命令。

## 方案

### 1. 能力探测(HostCapabilities)

首条 SSH 通道建立后(bootstrap exec 或 sidecar),一次性探测:

```
command -v tmux mosh-server; echo "::$(uname -s)::$(command -v apt-get dnf yum pacman apk brew 2>/dev/null | head -1)"
```

产出 `HostCapabilities { hasTmux, hasMoshServer, os, packageManager }`,
按 connection.id 缓存于内存 + UserDefaults(下次连接先用缓存、后台刷新)。
连接表单的 Custom tmux Path / Server binary 覆盖优先于探测。

### 2. 降级矩阵

| 用户配置 | 主机实况 | 行为 |
|---|---|---|
| SSH+tmux | 无 tmux | **自动降级纯 SSH 单 pane**(本次会话内禁用 -CC 路径),顶部一条可关闭 banner:"tmux not found on this host — plain SSH session. [Install tmux]" |
| mosh | 无 mosh-server | **自动降级 SSH**(复用既有 SSH 路径),banner:"mosh-server not found — connected over SSH instead. [Install mosh]";该连接标记 `moshUnavailable`,首页卡片 MOSH pill 显示降级态(灰 pill + ⚠) |
| mosh+tmux | 有 mosh 无 tmux | mosh 裸 shell(不发 attach 行,sidecar 不启动重试),banner 提示装 tmux |
| 任意 | 全有 | 现行为 |

降级只影响当次会话;能力缓存刷新后(比如用户装好了)下次连接自动回满血。

### 3. 安装助手(Install Assist sheet)

点 banner 的 [Install …] 弹 sheet:

- 按探测到的包管理器生成命令:
  - `apt-get` → `sudo apt-get install -y tmux mosh`
  - `dnf`/`yum` → `sudo dnf install -y tmux mosh`
  - `pacman` → `sudo pacman -S --noconfirm tmux mosh`
  - `apk` → `sudo apk add tmux mosh`
  - `brew`(macOS)→ `brew install tmux mosh`
  - 未识别 → 显示通用指引 + 文档链接
- 两个动作:**Run in terminal**(把命令粘进当前 shell 并回车——sudo 交互、输出全部可见)/ **Copy command**
- 装完点 **Re-check**:重跑探测,成功则提示"Reconnect to enable …"(一键重连)

无 root 的进阶路线(v2,不在本期):static binary 安装到 `~/.local/bin`
(mosh-server static、tmux appimage),配合表单里的自定义路径。

### 4. 空状态修正

`TmuxEmptyStateView` 增加分支:`!capabilities.hasTmux` 时标题改为
"tmux not installed on this host",CTA 从 Create Session 换成 Install tmux
(同一 sheet)。消除现在的误诊。

### 5. 实现落点

- 新:`Beacon/Services/HostCapabilities.swift`(探测 + 缓存,~80 行)
- 改:`SessionHub.start/startMosh`(探测注入 + 降级分支,~40 行)
- 新:`Beacon/UI/Terminal/HostBannerView.swift` + `InstallAssistSheet.swift`(~150 行)
- 改:`TmuxEmptyStateView` 分支、`ConnectionCard` 降级 pill
- 测试:HostCapabilities 解析单测(mock exec 输出);UI 测试用 `-MOSAIC_SEED_TMUX_BIN /bin/false` 模拟缺 tmux 走降级路径
- 实测:`tmux -L` 探测干扰不了(探测的是 PATH 中的 tmux);用一台干净 Docker/无 tmux 用户验证

## 不做的

- 静默自动安装(违反原则 3)
- 给 mosh 缺失弹模态阻断(降级即可,banner 足够)
- 在 Beacon 里内置二进制分发(签名/架构矩阵成本,v2 再议 static-binary 助手)
